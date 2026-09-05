//! Woven Lab — local multi-client replication visualizer.
//!
//! Start one Woven node, then open this example in multiple terminals. Each
//! window publishes its authoritative cube rotation on Woven's state channel
//! and renders a grid cell for every observed server-assigned entity.

use anyhow::Context;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use weaver_app::{
    Application, ApplicationConfig, HeadlessApp, Renderable, WeaverWorld, WorldConfig,
};
use weaver_render::{MeshHandle, MeshInstance, RenderTransform};
use weaver_render_wgpu::Vertex;
use weaver_worldline::{FrameId, SimulationClockConfig};
use weaver_woven::{ConnectivityMode, DeliveryClass, Payload, PersistenceClass, WovenConfig};

const STATE_CHANNEL: u64 = 2;
const GRID_COLUMNS: usize = 4;
const MAX_ROTATION_CORRECTION_RADIANS_PER_SECOND: f32 = 1.0;
const MAX_PUBLISH_CATCH_UP_PER_UPDATE: u32 = 8;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CubeState {
    client: String,
    rotation_radians: f32,
    angular_speed: f32,
    published_at_seconds: f64,
    tick: u64,
}

#[derive(Clone, Debug)]
struct LabConfig {
    endpoint: String,
    client_name: String,
    rate_hz: f64,
    steps_per_second: u32,
    angular_speed: f32,
    present_mode: wgpu::PresentMode,
}

struct LabView {
    started_at: Instant,
    local_woven_entity: Option<u64>,
    next_publish_time: f64,
    tick: u64,
    cells: Vec<weaver_core::EntityId>,
    states: BTreeMap<u64, ObservedCube>,
    received_sequences: BTreeMap<u64, u64>,
    metrics: LabMetrics,
}

struct LabMetrics {
    window_started_at: Instant,
    publish_attempts: u64,
    publish_accepted: u64,
    publish_errors: u64,
    peer_updates_received: u64,
    server_confirms: u64,
    last_confirmed_sequence: u64,
    last_confirm_at: Option<Instant>,
    summary: String,
}

impl LabView {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            local_woven_entity: None,
            next_publish_time: 0.0,
            tick: 0,
            cells: Vec::new(),
            states: BTreeMap::new(),
            received_sequences: BTreeMap::new(),
            metrics: LabMetrics {
                window_started_at: Instant::now(),
                publish_attempts: 0,
                publish_accepted: 0,
                publish_errors: 0,
                peer_updates_received: 0,
                server_confirms: 0,
                last_confirmed_sequence: 0,
                last_confirm_at: None,
                summary: "warming up metrics...".to_owned(),
            },
        }
    }
}

/// Received protocol state plus the independently-smoothed visual pose.
struct ObservedCube {
    state: CubeState,
    displayed_rotation_radians: f32,
    displayed_at_seconds: f64,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = read_config()?;
    let woven = WovenConfig {
        mode: ConnectivityMode::Loopback,
        endpoint: Some(config.endpoint.clone()),
        ..WovenConfig::default()
    };

    if std::env::var("WEAVER_HEADLESS").is_ok() {
        let mut app = HeadlessApp::new(WorldConfig {
            woven,
            clock: lab_clock(&config),
            ..WorldConfig::default()
        })?
        .with_max_steps(60);
        app.run()?;
        return Ok(());
    }

    let view = Arc::new(Mutex::new(LabView::new()));
    let view_setup = Arc::clone(&view);
    let view_update = Arc::clone(&view);
    let view_metrics = Arc::clone(&view);
    let view_title = Arc::clone(&view);
    let config_update = config.clone();
    let app = Application::new(ApplicationConfig {
        title: format!(
            "Woven Lab | {} | {} Hz tick | {:?}",
            config.client_name, config.steps_per_second, config.present_mode
        ),
        width: 1100,
        height: 720,
        world: WorldConfig {
            woven,
            clock: lab_clock(&config),
            background_color: [0.025, 0.035, 0.06, 1.0],
            ..WorldConfig::default()
        },
        setup: Some(Box::new(move |world, renderer| {
            setup(world, renderer, &view_setup)
        })),
        update: Some(Box::new(move |world, time| {
            update(world, time, &config_update, &view_update)
        })),
        tooltip: None,
        format_time: Some(Box::new(move |_time| {
            view_metrics
                .lock()
                .expect("lab state poisoned")
                .metrics
                .summary
                .clone()
        })),
        title_status: Some(Box::new(move || {
            view_title
                .lock()
                .expect("lab state poisoned")
                .metrics
                .summary
                .clone()
        })),
        side_menu: None,
        present_mode: config.present_mode,
    });
    app.run()?;
    Ok(())
}

fn read_config() -> anyhow::Result<LabConfig> {
    let endpoint = std::env::var("WOVEN_LAB_URL")
        .context("set WOVEN_LAB_URL, for example quic://127.0.0.1:8081")?;
    let client_name = std::env::var("WOVEN_LAB_CLIENT").unwrap_or_else(|_| "lab".to_owned());
    let rate_hz = std::env::var("WOVEN_LAB_RATE_HZ")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10.0_f64);
    anyhow::ensure!(
        rate_hz.is_finite() && rate_hz > 0.0 && rate_hz <= 120.0,
        "WOVEN_LAB_RATE_HZ must be in 0..=120"
    );
    let minimum_steps_per_second = rate_hz.ceil() as u32;
    let steps_per_second = std::env::var("WOVEN_LAB_STEPS_PER_SECOND")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(minimum_steps_per_second.max(60));
    anyhow::ensure!(
        steps_per_second >= minimum_steps_per_second && steps_per_second <= 240,
        "WOVEN_LAB_STEPS_PER_SECOND must be in {minimum_steps_per_second}..=240 to service WOVEN_LAB_RATE_HZ"
    );
    let present_mode = match std::env::var("WOVEN_LAB_PRESENT_MODE").as_deref() {
        Ok("vsync") | Err(_) => wgpu::PresentMode::AutoVsync,
        Ok("no-vsync") => wgpu::PresentMode::AutoNoVsync,
        Ok(value) => anyhow::bail!(
            "WOVEN_LAB_PRESENT_MODE must be `vsync` or `no-vsync`, received `{value}`"
        ),
    };
    let angular_speed = std::env::var("WOVEN_LAB_ANGULAR_SPEED")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(1.0_f32);
    anyhow::ensure!(
        angular_speed.is_finite() && angular_speed >= 0.0 && angular_speed <= 20.0,
        "WOVEN_LAB_ANGULAR_SPEED must be in 0..=20"
    );
    Ok(LabConfig {
        endpoint,
        client_name,
        rate_hz,
        steps_per_second,
        angular_speed,
        present_mode,
    })
}

fn lab_clock(config: &LabConfig) -> SimulationClockConfig {
    SimulationClockConfig {
        steps_per_second: config.steps_per_second,
        ..SimulationClockConfig::default()
    }
}

fn setup(
    world: &mut WeaverWorld,
    renderer: &mut weaver_render_wgpu::WgpuRenderer,
    _view: &Mutex<LabView>,
) {
    world.camera_mut().eye = Vec3::new(0.0, 0.0, 15.0);
    world.camera_mut().target = Vec3::ZERO;
    let mesh = MeshHandle::new();
    renderer
        .upload_mesh(mesh, &cube_vertices(), &cube_indices())
        .ok();
    for slot in 0..16 {
        let id = world.spawn(Renderable {
            mesh: Some(MeshInstance {
                mesh,
                transform: RenderTransform {
                    translation: grid_position(slot),
                    ..RenderTransform::default()
                },
                color: [0.48, 0.5, 0.56, 1.0],
                emissive: 0.08,
            }),
            sprite: None,
            emitter: None,
            label: Some("idle".to_owned()),
            frame: FrameId::ROOT,
        });
        _view.lock().expect("lab state poisoned").cells.push(id);
    }
}

fn update(world: &mut WeaverWorld, time: f64, config: &LabConfig, view: &Mutex<LabView>) {
    let mut view = view.lock().expect("lab state poisoned");
    for entity in world.drain_departed_woven_entities() {
        release_cube(world, &mut view, entity);
    }
    let mut publishes_this_update = 0;
    while time + 1e-9 >= view.next_publish_time
        && publishes_this_update < MAX_PUBLISH_CATCH_UP_PER_UPDATE
    {
        let published_at_seconds = unix_time_seconds();
        let local_elapsed_seconds = view.started_at.elapsed().as_secs_f64();
        let state = CubeState {
            client: config.client_name.clone(),
            rotation_radians: ((local_elapsed_seconds * f64::from(config.angular_speed))
                .rem_euclid(f64::from(std::f32::consts::TAU))) as f32,
            angular_speed: config.angular_speed,
            published_at_seconds,
            tick: view.tick,
        };
        let payload = Payload {
            body: state.clone(),
            sequence: view.tick + 1,
            revision: world.revision().get(),
        };
        view.metrics.publish_attempts += 1;
        match world.publish(
            STATE_CHANNEL,
            None,
            &payload,
            DeliveryClass::LatestValue,
            PersistenceClass::Stateful,
        ) {
            Ok(()) => {
                view.metrics.publish_accepted += 1;
                if let Some(entity) = world
                    .woven()
                    .and_then(weaver_woven::WovenAdapter::entity_id)
                {
                    view.local_woven_entity = Some(entity);
                    apply_cube_state(world, &mut view, entity, &state);
                }
            }
            Err(error) => {
                view.metrics.publish_errors += 1;
                tracing::warn!("Woven Lab publish failed: {error}");
            }
        }
        view.tick += 1;
        view.next_publish_time += 1.0 / config.rate_hz;
        publishes_this_update += 1;
    }
    if publishes_this_update == MAX_PUBLISH_CATCH_UP_PER_UPDATE
        && time + 1e-9 >= view.next_publish_time
    {
        tracing::warn!(
            configured_rate_hz = config.rate_hz,
            "publish scheduler is behind its bounded catch-up limit"
        );
    }

    let local_woven_entity = world
        .woven()
        .and_then(weaver_woven::WovenAdapter::entity_id);
    for envelope in world.replicated_payloads().to_vec() {
        let Some(woven_entity) = envelope.entity else {
            continue;
        };
        // The local cube is already driven by its render-time authoritative
        // state. Ignoring its looped-back packet makes a one-client run a
        // true rendering/frame-pacing test rather than a publish-cadence test.
        if Some(woven_entity) == local_woven_entity {
            // The looped-back echo is our only server-receipt confirmation:
            // the server accepted the publish and re-broadcast it. Track it
            // so the window title shows a live confirmed-recv tick and can
            // flag a silent disconnect.
            if envelope.sequence > view.metrics.last_confirmed_sequence {
                view.metrics.last_confirmed_sequence = envelope.sequence;
                view.metrics.server_confirms += 1;
                view.metrics.last_confirm_at = Some(Instant::now());
            }
            continue;
        }
        if view
            .received_sequences
            .get(&woven_entity)
            .is_some_and(|sequence| *sequence >= envelope.sequence)
        {
            continue;
        }
        let Ok(state) = serde_json::from_str::<CubeState>(&envelope.body_json) else {
            continue;
        };
        view.received_sequences
            .insert(woven_entity, envelope.sequence);
        view.metrics.peer_updates_received += 1;
        apply_cube_state(world, &mut view, woven_entity, &state);
    }

    // Animation is derived from the timestamped state, not packet arrival time.
    // This keeps all windows smooth between publishes and compensates local latency.
    layout_cubes(world, &mut view, unix_time_seconds());
    report_metrics(&mut view, config.rate_hz);
}

/// Human-readable server-receipt confirmation status for the window title.
///
/// The looped-back echo of our own published state is the only signal that
/// the server received and re-broadcast our tick. If echoes stop arriving —
/// disconnect, blackholed transport, or a dead server — the title must say so
/// instead of silently showing a healthy publish rate.
fn confirm_status(metrics: &LabMetrics, rate_hz: f64, confirms_per_second: f64) -> String {
    match metrics.last_confirm_at {
        Some(at) => {
            let silence_seconds = at.elapsed().as_secs_f64();
            // Allow a few missed publish cadences before declaring the server
            // round trip dead; low publish rates confirm less often.
            let lost_after_seconds = (3.0 / rate_hz).max(2.0);
            if silence_seconds > lost_after_seconds {
                format!(
                    "SERVER RECV LOST {silence_seconds:.0}s (last seq {})",
                    metrics.last_confirmed_sequence
                )
            } else {
                format!(
                    "srv-recv {confirms_per_second:.0} Hz seq {}",
                    metrics.last_confirmed_sequence
                )
            }
        }
        None => "srv-recv pending".to_owned(),
    }
}

fn report_metrics(view: &mut LabView, rate_hz: f64) {
    let elapsed = view.metrics.window_started_at.elapsed();
    if elapsed < std::time::Duration::from_secs(1) {
        return;
    }
    let seconds = elapsed.as_secs_f64();
    let publish_attempts_per_second = view.metrics.publish_attempts as f64 / seconds;
    let publish_accepted_per_second = view.metrics.publish_accepted as f64 / seconds;
    let peer_updates_per_second = view.metrics.peer_updates_received as f64 / seconds;
    let server_confirms_per_second = view.metrics.server_confirms as f64 / seconds;
    let confirm_status = confirm_status(&view.metrics, rate_hz, server_confirms_per_second);
    view.metrics.summary = format!(
        "pub {publish_accepted_per_second:.0}/{publish_attempts_per_second:.0} Hz | {confirm_status} | recv {peer_updates_per_second:.0} Hz | active {} | errors {}",
        view.states.len(),
        view.metrics.publish_errors,
    );
    tracing::info!(
        elapsed_seconds = view.started_at.elapsed().as_secs_f64(),
        publish_attempts_per_second,
        publish_accepted_per_second,
        publish_errors = view.metrics.publish_errors,
        peer_updates_per_second,
        server_confirms_per_second,
        last_confirmed_sequence = view.metrics.last_confirmed_sequence,
        active_entities = view.states.len(),
        "woven_lab_metrics"
    );
    view.metrics.window_started_at = Instant::now();
    view.metrics.publish_attempts = 0;
    view.metrics.publish_accepted = 0;
    view.metrics.publish_errors = 0;
    view.metrics.peer_updates_received = 0;
    view.metrics.server_confirms = 0;
}

fn apply_cube_state(
    world: &mut WeaverWorld,
    view: &mut LabView,
    woven_entity: u64,
    state: &CubeState,
) {
    let now_seconds = unix_time_seconds();
    match view.states.get_mut(&woven_entity) {
        Some(current) if state.tick >= current.state.tick => current.state = state.clone(),
        Some(_) => {}
        None => {
            view.states.insert(
                woven_entity,
                ObservedCube {
                    state: state.clone(),
                    displayed_rotation_radians: rotation_at(state, now_seconds),
                    displayed_at_seconds: now_seconds,
                },
            );
        }
    }
    layout_cubes(world, view, now_seconds);
}

fn release_cube(world: &mut WeaverWorld, view: &mut LabView, woven_entity: u64) {
    if view.states.remove(&woven_entity).is_some() {
        layout_cubes(world, view, unix_time_seconds());
    }
}

/// Keep every window's grid stable even if protocol messages arrive in a
/// different order: cells are always assigned by ascending server entity ID.
fn layout_cubes(world: &mut WeaverWorld, view: &mut LabView, now_seconds: f64) {
    for cell in &view.cells {
        set_idle_cube(world, *cell);
    }

    for (slot, (woven_entity, observed)) in view.states.iter_mut().enumerate() {
        let Some(cell) = view.cells.get(slot).copied() else {
            tracing::warn!(
                woven_entity,
                "Woven Lab grid is full; not rendering additional participant"
            );
            break;
        };
        if let Some(renderable) = world.get_mut(cell) {
            renderable.label = Some(format!("#{}\n{}", woven_entity, observed.state.client));
            if let Some(mesh) = renderable.mesh.as_mut() {
                if Some(*woven_entity) == view.local_woven_entity {
                    // `Instant` never moves backwards or jumps forward when the
                    // system clock is synchronized, so the local visual is a
                    // pure render-time animation independent of publish cadence.
                    observed.displayed_rotation_radians =
                        ((view.started_at.elapsed().as_secs_f64()
                            * f64::from(observed.state.angular_speed))
                        .rem_euclid(f64::from(std::f32::consts::TAU)))
                            as f32;
                    observed.displayed_at_seconds = now_seconds;
                } else {
                    let elapsed_seconds =
                        (now_seconds - observed.displayed_at_seconds).max(0.0) as f32;
                    let predicted_rotation = (observed.displayed_rotation_radians
                        + elapsed_seconds * observed.state.angular_speed)
                        .rem_euclid(std::f32::consts::TAU);
                    let desired_rotation = rotation_at(&observed.state, now_seconds);
                    let correction = shortest_rotation_delta(desired_rotation, predicted_rotation);
                    let maximum_correction =
                        MAX_ROTATION_CORRECTION_RADIANS_PER_SECOND * elapsed_seconds;
                    let rotation = predicted_rotation
                        + correction.clamp(-maximum_correction, maximum_correction);
                    observed.displayed_rotation_radians =
                        rotation.rem_euclid(std::f32::consts::TAU);
                    observed.displayed_at_seconds = now_seconds;
                }
                mesh.transform.rotation = Quat::from_axis_angle(
                    Vec3::new(0.35, 1.0, 0.2).normalize(),
                    observed.displayed_rotation_radians,
                );
                mesh.color = color_for(*woven_entity);
                mesh.emissive = 0.28;
            }
        }
    }
}

fn set_idle_cube(world: &mut WeaverWorld, cell: weaver_core::EntityId) {
    if let Some(renderable) = world.get_mut(cell) {
        renderable.label = Some("idle".to_owned());
        if let Some(mesh) = renderable.mesh.as_mut() {
            mesh.transform.rotation = Quat::IDENTITY;
            mesh.color = [0.48, 0.5, 0.56, 1.0];
            mesh.emissive = 0.08;
        }
    }
}

fn unix_time_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_secs_f64()
}

fn rotation_at(state: &CubeState, now_seconds: f64) -> f32 {
    let elapsed_seconds = (now_seconds - state.published_at_seconds).max(0.0) as f32;
    (state.rotation_radians + elapsed_seconds * state.angular_speed)
        .rem_euclid(std::f32::consts::TAU)
}

fn shortest_rotation_delta(target: f32, current: f32) -> f32 {
    (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

fn grid_position(index: usize) -> Vec3 {
    let column = (index % GRID_COLUMNS) as f32;
    let row = (index / GRID_COLUMNS) as f32;
    Vec3::new((column - 1.5) * 3.0, (1.5 - row) * 2.2, 0.0)
}

fn color_for(entity: u64) -> [f32; 4] {
    let red = ((entity.wrapping_mul(73) % 101) as f32) / 100.0;
    let green = ((entity.wrapping_mul(151) % 101) as f32) / 100.0;
    let blue = ((entity.wrapping_mul(199) % 101) as f32) / 100.0;
    [
        0.48 + red * 0.52,
        0.4 + green * 0.6,
        0.48 + blue * 0.52,
        1.0,
    ]
}

fn cube_vertices() -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(24);
    let mut add_face = |normal: [f32; 3], color: [f32; 4], corners: [[f32; 3]; 4]| {
        for (position, uv) in
            corners
                .into_iter()
                .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        {
            vertices.push(Vertex {
                position,
                normal,
                color,
                uv,
            });
        }
    };
    let n = 0.75;
    add_face(
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
        [[-n, -n, n], [n, -n, n], [n, n, n], [-n, n, n]],
    );
    add_face(
        [0.0, 0.0, -1.0],
        [0.55, 0.72, 1.0, 1.0],
        [[n, -n, -n], [-n, -n, -n], [-n, n, -n], [n, n, -n]],
    );
    add_face(
        [1.0, 0.0, 0.0],
        [1.0, 0.58, 0.58, 1.0],
        [[n, -n, -n], [n, n, -n], [n, n, n], [n, -n, n]],
    );
    add_face(
        [-1.0, 0.0, 0.0],
        [0.62, 1.0, 0.65, 1.0],
        [[-n, -n, n], [-n, n, n], [-n, n, -n], [-n, -n, -n]],
    );
    add_face(
        [0.0, 1.0, 0.0],
        [1.0, 0.88, 0.35, 1.0],
        [[-n, n, -n], [-n, n, n], [n, n, n], [n, n, -n]],
    );
    add_face(
        [0.0, -1.0, 0.0],
        [0.75, 0.55, 1.0, 1.0],
        [[-n, -n, n], [-n, -n, -n], [n, -n, -n], [n, -n, n]],
    );
    vertices
}

fn cube_indices() -> Vec<u16> {
    (0..6)
        .flat_map(|face| {
            let start = face * 4;
            [start, start + 1, start + 2, start, start + 2, start + 3]
        })
        .map(|index| u16::try_from(index).expect("cube index fits in u16"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn metrics(last_confirm_at: Option<Instant>, last_confirmed_sequence: u64) -> LabMetrics {
        LabMetrics {
            window_started_at: Instant::now(),
            publish_attempts: 0,
            publish_accepted: 0,
            publish_errors: 0,
            peer_updates_received: 0,
            server_confirms: 0,
            last_confirmed_sequence,
            last_confirm_at,
            summary: String::new(),
        }
    }

    #[test]
    fn confirm_status_is_pending_before_first_echo() {
        assert_eq!(
            confirm_status(&metrics(None, 0), 10.0, 0.0),
            "srv-recv pending"
        );
    }

    #[test]
    fn confirm_status_reports_rate_and_sequence_while_echoes_flow() {
        let metrics = metrics(Some(Instant::now()), 42);
        assert_eq!(
            confirm_status(&metrics, 10.0, 10.0),
            "srv-recv 10 Hz seq 42"
        );
    }

    #[test]
    fn confirm_status_flags_silent_server_as_lost() {
        let metrics = metrics(Some(Instant::now() - Duration::from_secs(5)), 42);
        assert_eq!(
            confirm_status(&metrics, 10.0, 0.0),
            "SERVER RECV LOST 5s (last seq 42)"
        );
    }

    #[test]
    fn confirm_status_tolerates_slow_publish_cadence() {
        // At 1 Hz the silence threshold stretches past two seconds so a
        // healthy low-rate run is not misreported as lost.
        let metrics = metrics(Some(Instant::now() - Duration::from_millis(2_500)), 3);
        assert_eq!(confirm_status(&metrics, 1.0, 1.0), "srv-recv 1 Hz seq 3");
    }
}
