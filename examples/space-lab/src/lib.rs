//! Platform-neutral Space Lab solar-system simulation.

use chrono::{DateTime, TimeZone, Utc};
use glam::{Quat, Vec3};
use weaver_app_core::{
    AppAction, AppAssets, FrameContext, InputFrame, MeshAsset, OrbitController, PointerMode,
    WeaverApp,
};
use weaver_render::{Camera, MeshHandle, MeshInstance, RenderTransform, SceneSnapshot};
use weaver_render_wgpu::Vertex;

/// World units per astronomical unit.
const AU_SCALE: f64 = 8.0;

/// World units per kilometre for lunar and planetary satellite distances.
const KM_SCALE: f64 = 5e-6;

/// Seconds in one Earth year.
const SECONDS_PER_YEAR: f64 = 365.25 * 24.0 * 60.0 * 60.0;

/// Seconds in one Earth day.
const SECONDS_PER_DAY: f64 = 24.0 * 60.0 * 60.0;

/// Initial simulation speed in simulation seconds per wall-clock second.
const TIME_MULTIPLIER: f64 = 1.0;

type MoonSpec = (&'static str, f64, f64, f64, f64, f64, f64, f64, [f32; 4]);

/// A celestial body with Keplerian orbital elements.
#[derive(Clone, Copy, Debug)]
struct Body {
    name: &'static str,
    /// Index of the parent body and mesh. `None` for the Sun.
    parent: Option<usize>,
    /// Semi-major axis, interpreted using `distance_scale`.
    semi_major_axis: f64,
    eccentricity: f64,
    /// Inclination to the parent's orbital plane in degrees.
    inclination: f64,
    /// Longitude of the ascending node in degrees.
    longitude_ascending_node: f64,
    /// Argument of periapsis in degrees.
    argument_of_periapsis: f64,
    /// Mean anomaly at J2000 in degrees.
    mean_anomaly_at_epoch: f64,
    /// Orbital period in seconds. Negative for retrograde motion.
    period_seconds: f64,
    /// Scale from semi-major-axis units to world units.
    distance_scale: f64,
    /// Exaggerated visual radius in world units.
    visual_radius: f32,
    /// Approximate surface color.
    color: [f32; 4],
}

impl Body {
    /// Compute the position relative to the parent body's center.
    #[allow(clippy::many_single_char_names)]
    fn relative_position(self, time: f64) -> Vec3 {
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

        // Position in the orbital plane, with x pointing toward periapsis.
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

/// Shared Space Lab application used by desktop and browser platform shells.
pub struct SpaceLab {
    scene: SceneSnapshot,
    assets: AppAssets,
    bodies: Vec<Body>,
    epoch_offset: f64,
    paused: bool,
    time_multiplier: f64,
    simulation_seconds: f64,
    last_elapsed_seconds: Option<f64>,
    orbit: OrbitController,
}

impl SpaceLab {
    /// Build the solar-system scene at the current offset from the J2000 epoch.
    #[must_use]
    pub fn new() -> Self {
        let sphere_mesh = MeshHandle::new();
        let (vertices, indices) = build_sphere(1.0, 24, 24);
        let assets = AppAssets {
            meshes: vec![MeshAsset::new(sphere_mesh, vertices, indices)],
            ..AppAssets::default()
        };

        let mut scene = SceneSnapshot {
            camera: Camera {
                eye: Vec3::new(0.0, 25.0, 45.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            background_color: [0.0, 0.0, 0.0, 1.0],
            time_multiplier: TIME_MULTIPLIER,
            ..SceneSnapshot::default()
        };
        let bodies = populate_solar_system(&mut scene, sphere_mesh);
        let orbit = OrbitController::from_camera(&scene.camera);

        let mut app = Self {
            scene,
            assets,
            bodies,
            epoch_offset: current_j2000_offset(),
            paused: false,
            time_multiplier: TIME_MULTIPLIER,
            simulation_seconds: 0.0,
            last_elapsed_seconds: None,
            orbit,
        };
        app.update_body_positions();
        app.update_status();
        app
    }

    /// Return body names in their stable mesh-aligned insertion order.
    pub fn body_names(&self) -> impl ExactSizeIterator<Item = &'static str> + '_ {
        self.bodies.iter().map(|body| body.name)
    }

    fn effective_time(&self) -> f64 {
        self.epoch_offset + self.simulation_seconds
    }

    fn update_body_positions(&mut self) {
        let effective_time = self.effective_time();

        // Parents are inserted before children, and body indices are kept aligned
        // with mesh indices, so absolute positions can be resolved in one pass.
        for index in 0..self.bodies.len() {
            let body = self.bodies[index];
            let parent_position = body.parent.map_or(Vec3::ZERO, |parent| {
                self.scene.meshes[parent].transform.translation
            });
            let mesh = &mut self.scene.meshes[index];
            mesh.transform.translation = parent_position + body.relative_position(effective_time);
            mesh.transform.rotation = Quat::IDENTITY;
        }
    }

    fn update_status(&mut self) {
        self.scene.paused = self.paused;
        self.scene.time_multiplier = self.time_multiplier;
        let run_state = if self.paused { "paused" } else { "running" };
        self.scene.status_lines = vec![
            format!(
                "{} bodies · {run_state} at {}×",
                self.bodies.len(),
                self.time_multiplier
            ),
            format_simulation_time(self.effective_time()),
        ];
    }
}

impl Default for SpaceLab {
    fn default() -> Self {
        Self::new()
    }
}

impl WeaverApp for SpaceLab {
    fn title(&self) -> &'static str {
        "WVR | Space Lab"
    }

    fn scene(&self) -> &SceneSnapshot {
        &self.scene
    }

    fn scene_mut(&mut self) -> &mut SceneSnapshot {
        &mut self.scene
    }

    fn assets(&self) -> &AppAssets {
        &self.assets
    }

    fn pointer_mode(&self) -> PointerMode {
        PointerMode::OrbitDrag
    }

    fn update(&mut self, frame: FrameContext, input: &InputFrame) {
        for action in &input.actions {
            match *action {
                AppAction::TogglePause => self.paused = !self.paused,
                AppAction::SetTimeMultiplier(multiplier)
                    if multiplier.is_finite() && multiplier >= 0.0 =>
                {
                    self.time_multiplier = multiplier;
                }
                _ => {}
            }
        }

        let wall_delta_seconds = self
            .last_elapsed_seconds
            .replace(frame.elapsed_seconds)
            .map_or_else(
                || f64::from(frame.delta_seconds),
                |last| (frame.elapsed_seconds - last).max(0.0),
            );
        if !self.paused {
            self.simulation_seconds += wall_delta_seconds * self.time_multiplier;
        }

        self.update_body_positions();
        self.orbit.update(&mut self.scene.camera, input);
        self.update_status();
    }
}

/// Start Space Lab in a browser after the generated WASM module loads.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    weaver_platform_web::run(
        Box::new(SpaceLab::new()),
        weaver_platform_web::WebConfig::default(),
    )
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
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

fn current_j2000_offset() -> f64 {
    let j2000 = Utc
        .with_ymd_and_hms(2000, 1, 1, 12, 0, 0)
        .single()
        .expect("J2000 epoch is valid");
    Utc::now().signed_duration_since(j2000).num_seconds() as f64
}

fn format_simulation_time(effective_time: f64) -> String {
    let j2000 = Utc
        .with_ymd_and_hms(2000, 1, 1, 12, 0, 0)
        .single()
        .expect("J2000 epoch is valid");
    let datetime: DateTime<Utc> = j2000 + chrono::Duration::seconds(effective_time as i64);
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn spawn_body(
    scene: &mut SceneSnapshot,
    bodies: &mut Vec<Body>,
    sphere_mesh: MeshHandle,
    body: Body,
    emissive: f32,
) -> usize {
    let index = bodies.len();
    debug_assert_eq!(scene.meshes.len(), index);
    debug_assert!(body.parent.is_none_or(|parent| parent < index));
    bodies.push(body);
    scene.meshes.push(MeshInstance {
        mesh: sphere_mesh,
        transform: RenderTransform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: body.visual_radius,
        },
        color: body.color,
        emissive,
    });
    index
}

#[allow(clippy::unreadable_literal)]
fn populate_solar_system(scene: &mut SceneSnapshot, sphere_mesh: MeshHandle) -> Vec<Body> {
    let mut bodies = Vec::new();

    let sun = spawn_body(
        scene,
        &mut bodies,
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
        1.0,
    );

    spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    let earth = spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    let mars = spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    let jupiter = spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    let jovian_moons: &[MoonSpec] = &[
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
    for &(name, a, e, i, lan, arg, ma, period, color) in jovian_moons {
        spawn_body(
            scene,
            &mut bodies,
            sphere_mesh,
            Body {
                name,
                parent: Some(jupiter),
                semi_major_axis: a,
                eccentricity: e,
                inclination: i,
                longitude_ascending_node: lan,
                argument_of_periapsis: arg,
                mean_anomaly_at_epoch: ma,
                period_seconds: period,
                distance_scale: KM_SCALE,
                visual_radius: 0.03,
                color,
            },
            0.0,
        );
    }

    let saturn = spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    let saturnian_moons: &[MoonSpec] = &[
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
    for &(name, a, e, i, lan, arg, ma, period, color) in saturnian_moons {
        spawn_body(
            scene,
            &mut bodies,
            sphere_mesh,
            Body {
                name,
                parent: Some(saturn),
                semi_major_axis: a,
                eccentricity: e,
                inclination: i,
                longitude_ascending_node: lan,
                argument_of_periapsis: arg,
                mean_anomaly_at_epoch: ma,
                period_seconds: period,
                distance_scale: KM_SCALE,
                visual_radius: 0.03,
                color,
            },
            0.0,
        );
    }

    let uranus = spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    let uranian_moons: &[MoonSpec] = &[
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
    for &(name, a, e, i, lan, arg, ma, period, color) in uranian_moons {
        spawn_body(
            scene,
            &mut bodies,
            sphere_mesh,
            Body {
                name,
                parent: Some(uranus),
                semi_major_axis: a,
                eccentricity: e,
                inclination: i,
                longitude_ascending_node: lan,
                argument_of_periapsis: arg,
                mean_anomaly_at_epoch: ma,
                period_seconds: period,
                distance_scale: KM_SCALE,
                visual_radius: 0.03,
                color,
            },
            0.0,
        );
    }

    let neptune = spawn_body(
        scene,
        &mut bodies,
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
        0.0,
    );

    spawn_body(
        scene,
        &mut bodies,
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
            period_seconds: -5.876854 * SECONDS_PER_DAY,
            distance_scale: KM_SCALE,
            visual_radius: 0.04,
            color: [0.75, 0.8, 0.85, 1.0],
        },
        0.0,
    );

    bodies
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
                normal: normal.to_array(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_order_stays_aligned_with_meshes_and_parents() {
        let app = SpaceLab::new();
        let names: Vec<_> = app.body_names().collect();

        assert_eq!(app.bodies.len(), app.scene.meshes.len());
        assert_eq!(app.bodies.len(), 29);
        assert_eq!(&names[..5], &["Sun", "Mercury", "Venus", "Earth", "Moon"]);
        assert_eq!(names.last(), Some(&"Triton"));
        for (index, body) in app.bodies.iter().enumerate() {
            assert!(body.parent.is_none_or(|parent| parent < index));
        }
    }

    #[test]
    fn orbital_math_handles_circular_and_parent_relative_positions() {
        assert!((solve_kepler(1.25, 0.0) - 1.25).abs() < 1e-12);
        assert!((compute_true_anomaly(0.75, 0.0) - 0.75).abs() < 1e-12);

        let mut app = SpaceLab::new();
        app.epoch_offset = 0.0;
        app.simulation_seconds = 0.0;
        app.update_body_positions();

        let earth_index = app.body_names().position(|name| name == "Earth").unwrap();
        let moon_index = app.body_names().position(|name| name == "Moon").unwrap();
        let earth_position = app.scene.meshes[earth_index].transform.translation;
        let moon_position = app.scene.meshes[moon_index].transform.translation;
        let expected_relative = app.bodies[moon_index].relative_position(0.0);
        assert!((moon_position - earth_position - expected_relative).length() < 1e-5);
    }

    #[test]
    fn sphere_mesh_has_expected_topology_and_unit_normals() {
        let (vertices, indices) = build_sphere(1.0, 24, 24);
        assert_eq!(vertices.len(), 25 * 25);
        assert_eq!(indices.len(), 24 * 24 * 6);
        assert!(
            indices
                .iter()
                .all(|&index| usize::from(index) < vertices.len())
        );
        assert!(vertices.iter().all(|vertex| {
            (Vec3::from_array(vertex.normal).length() - 1.0).abs() < f32::EPSILON * 4.0
        }));
    }

    #[test]
    fn actions_pause_and_change_simulation_speed() {
        let mut app = SpaceLab::new();
        app.epoch_offset = 0.0;
        app.simulation_seconds = 0.0;

        app.update(
            FrameContext {
                delta_seconds: 2.0,
                elapsed_seconds: 2.0,
            },
            &InputFrame {
                actions: vec![AppAction::SetTimeMultiplier(2.0)],
                ..InputFrame::default()
            },
        );
        assert_eq!(app.simulation_seconds, 4.0);
        assert_eq!(app.scene.time_multiplier, 2.0);

        app.update(
            FrameContext {
                delta_seconds: 2.0,
                elapsed_seconds: 4.0,
            },
            &InputFrame {
                actions: vec![AppAction::TogglePause],
                ..InputFrame::default()
            },
        );
        assert_eq!(app.simulation_seconds, 4.0);
        assert!(app.paused);
        assert!(app.scene.paused);
    }

    #[test]
    fn simulation_uses_uncapped_elapsed_time_between_frames() {
        let mut app = SpaceLab::new();
        app.epoch_offset = 0.0;
        app.simulation_seconds = 0.0;
        app.update(
            FrameContext {
                delta_seconds: 0.1,
                elapsed_seconds: 1.0,
            },
            &InputFrame::default(),
        );
        app.update(
            FrameContext {
                delta_seconds: 0.1,
                elapsed_seconds: 3.0,
            },
            &InputFrame::default(),
        );
        assert!((app.simulation_seconds - 2.1).abs() < 1e-6);
    }
}
