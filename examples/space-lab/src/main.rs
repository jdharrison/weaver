//! Space Lab — high-accuracy solar system simulation.
//!
//! Replicates our solar system using real orbital elements, distances, and
//! periods. Each body follows a Keplerian ellipse solved from its
//! J2000-equivalent elements and is recorded into Worldline's deterministic
//! trajectory history. Moons orbit their parent planets. Reference-frame
//! relativity is exposed by allowing any body to be selected as the camera
//! origin.

use chrono::{DateTime, TimeZone, Utc};
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use weaver_app::debug::{MenuAction, MenuItem};
use weaver_app::{
    Application, ApplicationConfig, HeadlessApp, Renderable, WeaverWorld, WorldConfig,
};
use weaver_core::EntityId;
use weaver_render::{MeshHandle, MeshInstance, RenderTransform};
use weaver_render_wgpu::Vertex;
use weaver_worldline::{FrameId, SimulationClockConfig};

/// World units per astronomical unit.
const AU_SCALE: f64 = 8.0;

/// World units per kilometre for lunar/planetary satellite distances.
const KM_SCALE: f64 = 5e-6;

/// Seconds in one Earth year.
const SECONDS_PER_YEAR: f64 = 365.25 * 24.0 * 60.0 * 60.0;

/// Seconds in one Earth day.
const SECONDS_PER_DAY: f64 = 24.0 * 60.0 * 60.0;

/// Simulation speed: 1 simulation second per real second.
const TIME_MULTIPLIER: f64 = 1.0;

/// A celestial body with Keplerian orbital elements.
#[derive(Clone, Copy, Debug)]
struct Body {
    name: &'static str,
    /// Parent entity; `None` for the Sun.
    parent: Option<EntityId>,
    /// Semi-major axis. The unit depends on the body type (AU for planets, km
    /// for moons), and is interpreted with `distance_scale`.
    semi_major_axis: f64,
    eccentricity: f64,
    /// Inclination to the parent's orbital plane in degrees.
    inclination: f64,
    /// Longitude of the ascending node in degrees.
    longitude_ascending_node: f64,
    /// Argument of periapsis in degrees.
    argument_of_periapsis: f64,
    /// Mean anomaly at epoch in degrees.
    mean_anomaly_at_epoch: f64,
    /// Orbital period in seconds. Negative for retrograde motion.
    period_seconds: f64,
    /// Scale from `semi_major_axis` units to world units.
    distance_scale: f64,
    /// Visual radius in world units (exaggerated for visibility).
    visual_radius: f32,
    /// Approximate surface color.
    color: [f32; 4],
}

impl Body {
    /// Compute the position relative to the parent body's center.
    #[must_use]
    #[allow(clippy::many_single_char_names)]
    fn relative_position(&self, time: f64) -> Vec3 {
        if self.semi_major_axis <= 0.0 || self.period_seconds == 0.0 {
            return Vec3::ZERO;
        }
        let a = self.semi_major_axis;
        let e = self.eccentricity;
        let mean_motion = 2.0 * std::f64::consts::PI / self.period_seconds;
        let mean_anomaly = self.mean_anomaly_at_epoch.to_radians() + mean_motion * time;
        let eccentric_anomaly = solve_kepler(mean_anomaly, e);
        let true_anomaly = compute_true_anomaly(eccentric_anomaly, e);
        let r = a * self.distance_scale * (1.0 - e * eccentric_anomaly.cos());

        // Position in the orbital plane, x toward periapsis.
        let x = r * true_anomaly.cos();
        let y = r * true_anomaly.sin();

        rotate_to_ecliptic(
            x,
            y,
            self.argument_of_periapsis,
            self.inclination,
            self.longitude_ascending_node,
        )
    }
}

fn solve_kepler(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let mut eccentric_anomaly = mean_anomaly;
    for _ in 0..50 {
        let f = eccentric_anomaly - eccentricity * eccentric_anomaly.sin() - mean_anomaly;
        let fp = 1.0 - eccentricity * eccentric_anomaly.cos();
        let delta = f / fp;
        eccentric_anomaly -= delta;
        if delta.abs() < 1e-12 {
            break;
        }
    }
    eccentric_anomaly
}

fn compute_true_anomaly(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
    let half_e = eccentric_anomaly / 2.0;
    2.0 * ((1.0 + eccentricity).sqrt() * half_e.sin())
        .atan2((1.0 - eccentricity).sqrt() * half_e.cos())
}

fn rotate_to_ecliptic(
    x: f64,
    y: f64,
    argument_of_periapsis: f64,
    inclination: f64,
    longitude_ascending_node: f64,
) -> Vec3 {
    let arg = argument_of_periapsis.to_radians();
    let inc = inclination.to_radians();
    let lan = longitude_ascending_node.to_radians();

    let (cos_a, sin_a) = (arg.cos(), arg.sin());
    let (cos_i, sin_i) = (inc.cos(), inc.sin());
    let (cos_l, sin_l) = (lan.cos(), lan.sin());

    // R = R3(lan) * R1(inc) * R3(arg)
    let x1 = cos_a * x - sin_a * y;
    let y1 = sin_a * x + cos_a * y;

    let x2 = x1;
    let y2 = cos_i * y1;
    let z2 = sin_i * y1;

    let x3 = cos_l * x2 - sin_l * y2;
    let y3 = sin_l * x2 + cos_l * y2;

    Vec3::new(x3 as f32, y3 as f32, z2 as f32)
}

/// Mutable simulation state shared between setup, update, and tooltip callbacks.
struct SimulationState {
    /// Map from entity id to body definition.
    bodies: HashMap<EntityId, Body>,
    /// Insertion order; parents are always inserted before their moons.
    order: Vec<EntityId>,
    /// Seconds from the J2000 epoch to the application's start time. The
    /// simulation time is interpreted as seconds since J2000, so adding this
    /// offset makes the solar system orientation match the current date/time.
    epoch_offset: f64,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    if std::env::var("WEAVER_HEADLESS").is_ok() {
        let mut app = HeadlessApp::new(WorldConfig {
            background_color: [0.0, 0.0, 0.0, 1.0],
            starfield_count: 0,
            ..WorldConfig::default()
        })?
        .with_max_steps(60);
        app.run()?;
        return Ok(());
    }

    let j2000 = Utc
        .with_ymd_and_hms(2000, 1, 1, 12, 0, 0)
        .single()
        .expect("J2000 epoch");
    let epoch_offset = Utc::now().signed_duration_since(j2000).num_seconds() as f64;

    let state = Arc::new(Mutex::new(SimulationState {
        bodies: HashMap::new(),
        order: Vec::new(),
        epoch_offset,
    }));
    let state_setup = Arc::clone(&state);
    let state_update = Arc::clone(&state);
    let state_tooltip = Arc::clone(&state);
    let state_side_menu = Arc::clone(&state);

    let config = ApplicationConfig {
        title: "WVR | Space Lab".to_string(),
        width: 1280,
        height: 720,
        world: WorldConfig {
            background_color: [0.0, 0.0, 0.0, 1.0],
            starfield_count: 0,
            clock: SimulationClockConfig {
                steps_per_second: 60,
                time_multiplier: TIME_MULTIPLIER,
                ..SimulationClockConfig::default()
            },
            ..WorldConfig::default()
        },
        setup: Some(Box::new(move |world, renderer| {
            populate_solar_system(world, renderer, &state_setup);
        })),
        update: Some(Box::new(move |world, time| {
            animate_bodies(world, time, &state_update);
        })),
        tooltip: Some(Box::new(move |world, id, time| {
            format_body_tooltip(world, id, time, &state_tooltip)
        })),
        format_time: Some(Box::new(move |time| {
            format_simulation_time(time + epoch_offset)
        })),
        title_status: None,
        side_menu: Some(Box::new(move |world| {
            build_side_menu(world, &state_side_menu)
        })),
        present_mode: wgpu::PresentMode::AutoNoVsync,
    };

    let app = Application::new(config);
    app.run()?;
    Ok(())
}

fn spawn_body(
    world: &mut WeaverWorld,
    _renderer: &mut weaver_render_wgpu::WgpuRenderer,
    mesh: MeshHandle,
    body: Body,
    state: &mut SimulationState,
    emissive: f32,
) -> EntityId {
    let frame = match body.parent {
        Some(parent_id) => {
            let parent_frame = world.get(parent_id).map_or(FrameId::ROOT, |r| r.frame);
            world
                .frames_mut()
                .create(parent_frame)
                .unwrap_or(FrameId::ROOT)
        }
        None => FrameId::ROOT,
    };
    let id = world.spawn(Renderable {
        mesh: Some(MeshInstance {
            mesh,
            transform: RenderTransform {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: body.visual_radius,
            },
            color: body.color,
            emissive,
        }),
        sprite: None,
        emitter: None,
        label: Some(body.name.to_string()),
        frame,
    });
    state.order.push(id);
    state.bodies.insert(id, body);
    id
}

#[allow(clippy::unreadable_literal)]
#[allow(clippy::type_complexity)]
fn populate_solar_system(
    world: &mut WeaverWorld,
    renderer: &mut weaver_render_wgpu::WgpuRenderer,
    state: &Mutex<SimulationState>,
) {
    world.camera_mut().eye = Vec3::new(0.0, 25.0, 45.0);
    world.camera_mut().target = Vec3::ZERO;

    let sphere_mesh = MeshHandle::new();
    let (vertices, indices) = build_sphere(1.0, 24, 24);
    renderer.upload_mesh(sphere_mesh, &vertices, &indices).ok();

    let mut state = state.lock().unwrap();

    let sun = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Sun",
            parent: None,
            semi_major_axis: 0.0,
            eccentricity: 0.0,
            inclination: 0.0,
            longitude_ascending_node: 0.0,
            argument_of_periapsis: 0.0,
            mean_anomaly_at_epoch: 0.0,
            period_seconds: 0.0,
            distance_scale: AU_SCALE,
            visual_radius: 1.2,
            color: [1.0, 0.9, 0.3, 1.0],
        },
        &mut state,
        1.0,
    );

    let _mercury = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Mercury",
            parent: Some(sun),
            semi_major_axis: 0.387098,
            eccentricity: 0.205630,
            inclination: 7.005,
            longitude_ascending_node: 48.331,
            argument_of_periapsis: 29.124,
            mean_anomaly_at_epoch: 174.796,
            period_seconds: 0.2408467 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.12,
            color: [0.7, 0.7, 0.7, 1.0],
        },
        &mut state,
        0.0,
    );

    let _venus = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Venus",
            parent: Some(sun),
            semi_major_axis: 0.723332,
            eccentricity: 0.006773,
            inclination: 3.39458,
            longitude_ascending_node: 76.680,
            argument_of_periapsis: 54.884,
            mean_anomaly_at_epoch: 50.115,
            period_seconds: 0.615198 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.18,
            color: [0.9, 0.8, 0.5, 1.0],
        },
        &mut state,
        0.0,
    );

    let earth = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Earth",
            parent: Some(sun),
            semi_major_axis: 1.000001018,
            eccentricity: 0.0167086,
            inclination: 0.00005,
            longitude_ascending_node: -11.26064,
            argument_of_periapsis: 114.20783,
            mean_anomaly_at_epoch: 358.617,
            period_seconds: 1.0000174 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.19,
            color: [0.2, 0.5, 0.9, 1.0],
        },
        &mut state,
        0.0,
    );

    spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Moon",
            parent: Some(earth),
            semi_major_axis: 384_400.0,
            eccentricity: 0.0549,
            inclination: 5.14,
            longitude_ascending_node: 125.08,
            argument_of_periapsis: 318.15,
            mean_anomaly_at_epoch: 134.9,
            period_seconds: 27.321661 * SECONDS_PER_DAY,
            distance_scale: KM_SCALE,
            visual_radius: 0.04,
            color: [0.75, 0.75, 0.7, 1.0],
        },
        &mut state,
        0.0,
    );

    let mars = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Mars",
            parent: Some(sun),
            semi_major_axis: 1.523679,
            eccentricity: 0.0934,
            inclination: 1.850,
            longitude_ascending_node: 49.558,
            argument_of_periapsis: 286.502,
            mean_anomaly_at_epoch: 19.373,
            period_seconds: 1.8808 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.02,
            color: [0.9, 0.4, 0.2, 1.0],
        },
        &mut state,
        0.0,
    );

    spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Phobos",
            parent: Some(mars),
            semi_major_axis: 9_376.0,
            eccentricity: 0.0151,
            inclination: 1.093,
            longitude_ascending_node: 164.58,
            argument_of_periapsis: 150.06,
            mean_anomaly_at_epoch: 112.0,
            period_seconds: 0.31891023 * SECONDS_PER_DAY,
            distance_scale: KM_SCALE,
            visual_radius: 0.02,
            color: [0.6, 0.55, 0.5, 1.0],
        },
        &mut state,
        0.0,
    );

    spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Deimos",
            parent: Some(mars),
            semi_major_axis: 23_463.0,
            eccentricity: 0.0005,
            inclination: 1.793,
            longitude_ascending_node: 17.7,
            argument_of_periapsis: 260.1,
            mean_anomaly_at_epoch: 285.0,
            period_seconds: 1.2624409 * SECONDS_PER_DAY,
            distance_scale: KM_SCALE,
            visual_radius: 0.02,
            color: [0.6, 0.55, 0.5, 1.0],
        },
        &mut state,
        0.0,
    );

    let jupiter = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Jupiter",
            parent: Some(sun),
            semi_major_axis: 5.2044,
            eccentricity: 0.0489,
            inclination: 1.303,
            longitude_ascending_node: 100.464,
            argument_of_periapsis: 273.867,
            mean_anomaly_at_epoch: 20.020,
            period_seconds: 11.8626 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.8,
            color: [0.8, 0.7, 0.5, 1.0],
        },
        &mut state,
        0.0,
    );

    let jovian_moons: &[(&str, f64, f64, f64, f64, f64, f64, f64, [f32; 4])] = &[
        (
            "Io",
            421_700.0,
            0.0041,
            0.05,
            43.0,
            127.0,
            171.0,
            1.7691378 * SECONDS_PER_DAY,
            [0.95, 0.85, 0.4, 1.0],
        ),
        (
            "Europa",
            671_034.0,
            0.0094,
            0.47,
            119.0,
            287.0,
            352.0,
            3.551181 * SECONDS_PER_DAY,
            [0.75, 0.8, 0.9, 1.0],
        ),
        (
            "Ganymede",
            1_070_412.0,
            0.0013,
            0.20,
            63.0,
            192.0,
            202.0,
            7.15455296 * SECONDS_PER_DAY,
            [0.65, 0.65, 0.6, 1.0],
        ),
        (
            "Callisto",
            1_882_709.0,
            0.0074,
            0.20,
            298.0,
            124.0,
            327.0,
            16.6890184 * SECONDS_PER_DAY,
            [0.5, 0.45, 0.4, 1.0],
        ),
    ];
    for (name, a, e, i, lan, arg, ma, period, color) in jovian_moons {
        spawn_body(
            world,
            renderer,
            sphere_mesh,
            Body {
                name,
                parent: Some(jupiter),
                semi_major_axis: *a,
                eccentricity: *e,
                inclination: *i,
                longitude_ascending_node: *lan,
                argument_of_periapsis: *arg,
                mean_anomaly_at_epoch: *ma,
                period_seconds: *period,
                distance_scale: KM_SCALE,
                visual_radius: 0.03,
                color: *color,
            },
            &mut state,
            0.0,
        );
    }

    let saturn = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Saturn",
            parent: Some(sun),
            semi_major_axis: 9.5826,
            eccentricity: 0.0565,
            inclination: 2.485,
            longitude_ascending_node: 113.665,
            argument_of_periapsis: 339.392,
            mean_anomaly_at_epoch: 317.020,
            period_seconds: 29.4571 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.7,
            color: [0.9, 0.85, 0.6, 1.0],
        },
        &mut state,
        0.0,
    );

    let saturnian_moons: &[(&str, f64, f64, f64, f64, f64, f64, f64, [f32; 4])] = &[
        (
            "Mimas",
            185_520.0,
            0.0196,
            1.53,
            112.0,
            76.0,
            65.0,
            0.9424218 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Enceladus",
            238_020.0,
            0.0047,
            0.00,
            112.0,
            55.0,
            15.0,
            1.370218 * SECONDS_PER_DAY,
            [0.9, 0.9, 0.95, 1.0],
        ),
        (
            "Tethys",
            294_660.0,
            0.0001,
            1.09,
            112.0,
            33.0,
            250.0,
            1.887802 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Dione",
            377_400.0,
            0.0022,
            0.03,
            112.0,
            174.0,
            95.0,
            2.736915 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Rhea",
            527_040.0,
            0.0013,
            0.33,
            112.0,
            203.0,
            316.0,
            4.517500 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Titan",
            1_221_830.0,
            0.0288,
            0.33,
            112.0,
            180.0,
            170.0,
            15.945421 * SECONDS_PER_DAY,
            [0.85, 0.7, 0.4, 1.0],
        ),
        (
            "Iapetus",
            3_560_820.0,
            0.0283,
            14.72,
            112.0,
            276.0,
            340.0,
            79.3215 * SECONDS_PER_DAY,
            [0.6, 0.55, 0.5, 1.0],
        ),
    ];
    for (name, a, e, i, lan, arg, ma, period, color) in saturnian_moons {
        spawn_body(
            world,
            renderer,
            sphere_mesh,
            Body {
                name,
                parent: Some(saturn),
                semi_major_axis: *a,
                eccentricity: *e,
                inclination: *i,
                longitude_ascending_node: *lan,
                argument_of_periapsis: *arg,
                mean_anomaly_at_epoch: *ma,
                period_seconds: *period,
                distance_scale: KM_SCALE,
                visual_radius: 0.03,
                color: *color,
            },
            &mut state,
            0.0,
        );
    }

    let uranus = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Uranus",
            parent: Some(sun),
            semi_major_axis: 19.2184,
            eccentricity: 0.046381,
            inclination: 0.773,
            longitude_ascending_node: 74.006,
            argument_of_periapsis: 96.998857,
            mean_anomaly_at_epoch: 141.050,
            period_seconds: 84.0205 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.45,
            color: [0.4, 0.8, 0.9, 1.0],
        },
        &mut state,
        0.0,
    );

    let uranian_moons: &[(&str, f64, f64, f64, f64, f64, f64, f64, [f32; 4])] = &[
        (
            "Miranda",
            129_390.0,
            0.0013,
            4.22,
            311.0,
            68.0,
            30.0,
            1.413479 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Ariel",
            190_900.0,
            0.0012,
            0.26,
            311.0,
            115.0,
            40.0,
            2.520379 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Umbriel",
            266_000.0,
            0.0039,
            0.13,
            311.0,
            79.0,
            10.0,
            4.144177 * SECONDS_PER_DAY,
            [0.65, 0.65, 0.6, 1.0],
        ),
        (
            "Titania",
            435_910.0,
            0.0011,
            0.08,
            311.0,
            284.0,
            215.0,
            8.705872 * SECONDS_PER_DAY,
            [0.75, 0.75, 0.7, 1.0],
        ),
        (
            "Oberon",
            583_520.0,
            0.0014,
            0.10,
            311.0,
            104.0,
            315.0,
            13.463239 * SECONDS_PER_DAY,
            [0.65, 0.65, 0.6, 1.0],
        ),
    ];
    for (name, a, e, i, lan, arg, ma, period, color) in uranian_moons {
        spawn_body(
            world,
            renderer,
            sphere_mesh,
            Body {
                name,
                parent: Some(uranus),
                semi_major_axis: *a,
                eccentricity: *e,
                inclination: *i,
                longitude_ascending_node: *lan,
                argument_of_periapsis: *arg,
                mean_anomaly_at_epoch: *ma,
                period_seconds: *period,
                distance_scale: KM_SCALE,
                visual_radius: 0.03,
                color: *color,
            },
            &mut state,
            0.0,
        );
    }

    let neptune = spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Neptune",
            parent: Some(sun),
            semi_major_axis: 30.11,
            eccentricity: 0.009456,
            inclination: 1.77,
            longitude_ascending_node: 131.784,
            argument_of_periapsis: 272.846,
            mean_anomaly_at_epoch: 256.225,
            period_seconds: 164.8 * SECONDS_PER_YEAR,
            distance_scale: AU_SCALE,
            visual_radius: 0.43,
            color: [0.2, 0.4, 0.95, 1.0],
        },
        &mut state,
        0.0,
    );

    spawn_body(
        world,
        renderer,
        sphere_mesh,
        Body {
            name: "Triton",
            parent: Some(neptune),
            semi_major_axis: 354_759.0,
            eccentricity: 0.0000,
            inclination: 156.865,
            longitude_ascending_node: 177.0,
            argument_of_periapsis: 237.0,
            mean_anomaly_at_epoch: 200.0,
            // Negative period for retrograde motion.
            period_seconds: -5.876854 * SECONDS_PER_DAY,
            distance_scale: KM_SCALE,
            visual_radius: 0.04,
            color: [0.75, 0.8, 0.85, 1.0],
        },
        &mut state,
        0.0,
    );
}

fn animate_bodies(world: &mut WeaverWorld, time: f64, state: &Mutex<SimulationState>) {
    let state = state.lock().unwrap();
    let effective_time = time + state.epoch_offset;

    // First pass: compute each body's position relative to its parent.
    let mut relative: HashMap<EntityId, Vec3> = HashMap::new();
    for id in &state.order {
        let body = state.bodies.get(id).copied().unwrap();
        relative.insert(*id, body.relative_position(effective_time));
    }

    // Second pass: resolve absolute positions. Parents are guaranteed to come
    // before children in `state.order`.
    for id in &state.order {
        let body = state.bodies.get(id).copied().unwrap();
        let abs = if let Some(parent_id) = body.parent {
            let parent_abs = world
                .get(parent_id)
                .and_then(|r| r.mesh.as_ref().map(|m| m.transform.translation))
                .unwrap_or_default();
            parent_abs + relative[id]
        } else {
            relative[id]
        };
        if let Some(renderable) = world.get_mut(*id) {
            if let Some(mesh) = renderable.mesh.as_mut() {
                mesh.transform.translation = abs;
                mesh.transform.rotation = Quat::IDENTITY;
            }
        }
    }
}

fn format_body_tooltip(
    world: &WeaverWorld,
    id: EntityId,
    time: f64,
    state: &Mutex<SimulationState>,
) -> String {
    let state = state.lock().unwrap();
    let effective_time = time + state.epoch_offset;
    let Some(body) = state.bodies.get(&id).copied() else {
        return String::new();
    };
    let Some(renderable) = world.get(id) else {
        return String::new();
    };
    let transform = renderable
        .mesh
        .as_ref()
        .map(|m| m.transform)
        .unwrap_or_default();
    let p = transform.translation;
    let r = f64::from(p.length()) / body.distance_scale;

    let mut lines = vec![body.name.to_string()];
    if let Some(parent_id) = body.parent {
        if let Some(parent) = state.bodies.get(&parent_id) {
            lines.push(format!("parent: {}", parent.name));
        }
    }
    if body.parent.is_none() {
        lines.push(format!("semi-major axis = {} AU", body.semi_major_axis));
    } else {
        lines.push(format!("semi-major axis = {} km", body.semi_major_axis));
    }
    lines.push(format!("eccentricity = {}", body.eccentricity));
    lines.push(format!("inclination = {}°", body.inclination));
    if body.period_seconds.abs() >= SECONDS_PER_YEAR {
        lines.push(format!(
            "orbital period = {} yr",
            body.period_seconds / SECONDS_PER_YEAR
        ));
    } else {
        lines.push(format!(
            "orbital period = {} d",
            body.period_seconds / SECONDS_PER_DAY
        ));
    }
    if body.parent.is_none() {
        lines.push(format!("heliocentric distance = {r:.3} AU"));
    } else {
        lines.push(format!("distance from parent = {r:.3} km"));
    }
    lines.push(format_simulation_time(effective_time));
    lines.join("\n")
}

fn build_side_menu(_world: &WeaverWorld, state: &Mutex<SimulationState>) -> Vec<MenuItem> {
    let state = state.lock().unwrap();
    let sun_id = state
        .bodies
        .iter()
        .find(|(_, body)| body.name == "Sun")
        .map(|(id, _)| *id);
    let mut entries: Vec<(EntityId, &Body)> = state
        .bodies
        .iter()
        .map(|(id, body)| (*id, body))
        .filter(|(_, body)| body.parent.is_none() || body.parent == sun_id)
        .collect();
    // Sun first, then planets ordered by distance.
    entries.sort_by(|a, b| {
        a.1.semi_major_axis
            .partial_cmp(&b.1.semi_major_axis)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
        .into_iter()
        .map(|(id, body)| MenuItem {
            label: body.name.to_string(),
            action: MenuAction::FocusEntity(id),
        })
        .collect()
}

fn format_simulation_time(effective_time: f64) -> String {
    let j2000 = Utc
        .with_ymd_and_hms(2000, 1, 1, 12, 0, 0)
        .single()
        .expect("J2000 epoch");
    let datetime: DateTime<Utc> = j2000 + chrono::Duration::seconds(effective_time as i64);
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn build_sphere(radius: f32, stacks: u32, sectors: u32) -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for stack in 0..=stacks {
        let phi = std::f32::consts::PI * stack as f32 / stacks as f32;
        let y = phi.cos();
        let r = phi.sin();
        for sector in 0..=sectors {
            let theta = 2.0 * std::f32::consts::PI * sector as f32 / sectors as f32;
            let x = r * theta.cos();
            let z = r * theta.sin();
            let normal = Vec3::new(x, y, z).normalize();
            vertices.push(Vertex {
                position: [radius * x, radius * y, radius * z],
                normal: [normal.x, normal.y, normal.z],
                color: [1.0, 1.0, 1.0, 1.0],
                uv: [sector as f32 / sectors as f32, stack as f32 / stacks as f32],
            });
        }
    }

    for stack in 0..stacks {
        for sector in 0..sectors {
            let a = stack * (sectors + 1) + sector;
            let b = a + sectors + 1;
            indices.extend_from_slice(&[a as u16, b as u16, (a + 1) as u16]);
            indices.extend_from_slice(&[b as u16, (b + 1) as u16, (a + 1) as u16]);
        }
    }

    (vertices, indices)
}
