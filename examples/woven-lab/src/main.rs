//! Woven Lab — opt-in local/verified-remote multi-client replication visualizer.
//!
//! Start one Woven node, then open this example in multiple terminals. Each
//! window publishes its authoritative cube rotation on Woven's state channel
//! and renders a grid cell for every observed server-assigned entity.

use anyhow::Context;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use weaver_app::{
    Application, ApplicationConfig, HeadlessApp, Renderable, ShutdownSignal, WeaverWorld,
    WorldConfig,
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
    #[serde(default = "default_publish_rate_hz")]
    publish_rate_hz: f64,
}

fn default_publish_rate_hz() -> f64 {
    10.0
}

fn valid_rate(rate_hz: f64) -> bool {
    rate_hz.is_finite() && rate_hz > 0.0 && rate_hz <= 120.0
}

fn valid_state(state: &CubeState) -> bool {
    valid_rate(state.publish_rate_hz)
        && state.rotation_radians.is_finite()
        && state.angular_speed.is_finite()
        && (0.0..=20.0).contains(&state.angular_speed)
        && state.published_at_seconds.is_finite()
        && state.published_at_seconds >= 0.0
}

/// Three declared publish intervals, with a floor for jitter and a hard horizon
/// even for extremely slow publishers. Older payloads use the lab's 10 Hz default.
fn peer_timeout(rate_hz: f64) -> Duration {
    Duration::from_secs_f64((3.0 / rate_hz).clamp(2.0, 30.0))
}

#[derive(Clone, Debug)]
struct LabConfig {
    woven: WovenConfig,
    duration: Option<Duration>,
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
    received_at: Instant,
    displayed_at: Instant,
    receipt_rotation_radians: f32,
}

impl ObservedCube {
    fn new(state: &CubeState, now: Instant, wall_seconds: f64) -> Self {
        let rotation = rotation_at(state, wall_seconds);
        Self {
            state: state.clone(),
            displayed_rotation_radians: rotation,
            received_at: now,
            displayed_at: now,
            receipt_rotation_radians: rotation,
        }
    }

    fn is_stale(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.received_at) >= peer_timeout(self.state.publish_rate_hz)
    }

    fn advance(&mut self, now: Instant) {
        let until = now.min(self.received_at + peer_timeout(self.state.publish_rate_hz));
        let elapsed = until
            .saturating_duration_since(self.displayed_at)
            .as_secs_f32();
        if elapsed == 0.0 {
            return;
        }
        let predicted = (self.displayed_rotation_radians + elapsed * self.state.angular_speed)
            .rem_euclid(std::f32::consts::TAU);
        let age = until
            .saturating_duration_since(self.received_at)
            .as_secs_f32();
        let desired = (self.receipt_rotation_radians + age * self.state.angular_speed)
            .rem_euclid(std::f32::consts::TAU);
        let maximum = MAX_ROTATION_CORRECTION_RADIANS_PER_SECOND * elapsed;
        self.displayed_rotation_radians = (predicted
            + shortest_rotation_delta(desired, predicted).clamp(-maximum, maximum))
        .rem_euclid(std::f32::consts::TAU);
        self.displayed_at = until;
    }

    fn receive(&mut self, state: &CubeState, now: Instant, wall_seconds: f64) {
        // Finish the old bounded prediction before replacing its velocity. Never
        // apply time spent frozen to a recovery correction or snap to the packet.
        self.advance(now);
        self.state = state.clone();
        self.received_at = now;
        self.displayed_at = now;
        self.receipt_rotation_radians = rotation_at(state, wall_seconds);
    }
}

impl LabView {
    fn receive_peer(
        &mut self,
        entity: u64,
        sequence: u64,
        state: &CubeState,
        now: Instant,
        wall_seconds: f64,
    ) -> bool {
        if Some(entity) == self.local_woven_entity
            || !valid_state(state)
            || self
                .received_sequences
                .get(&entity)
                .is_some_and(|old| sequence <= *old)
            || self
                .states
                .get(&entity)
                .is_some_and(|old| state.tick <= old.state.tick)
        {
            return false;
        }
        match self.states.get_mut(&entity) {
            Some(current) => current.receive(state, now, wall_seconds),
            None => {
                self.states
                    .insert(entity, ObservedCube::new(state, now, wall_seconds));
            }
        }
        self.received_sequences.insert(entity, sequence);
        self.metrics.peer_updates_received += 1;
        true
    }

    fn forget_peer(&mut self, entity: u64) {
        self.states.remove(&entity);
        self.received_sequences.remove(&entity);
    }

    fn stale_peers(&self, now: Instant) -> usize {
        self.states
            .iter()
            .filter(|(entity, observed)| {
                Some(**entity) != self.local_woven_entity && observed.is_stale(now)
            })
            .count()
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = read_config()?;
    let mut woven = config.woven.clone();
    woven.run_deadline = config.duration.map(|duration| Instant::now() + duration);
    let shutdown = woven
        .run_deadline
        .map_or_else(ShutdownSignal::default, ShutdownSignal::with_deadline);

    if std::env::var("WEAVER_HEADLESS").is_ok() {
        let mut app = HeadlessApp::new(WorldConfig {
            woven,
            clock: lab_clock(&config),
            ..WorldConfig::default()
        })?
        .with_max_steps(60)
        .with_shutdown_signal(shutdown);
        tracing::info!("headless smoke check: at most 60 steps, no lab publishing");
        app.run()?;
        return Ok(());
    }

    let view = Arc::new(Mutex::new(LabView::new()));
    let view_setup = Arc::clone(&view);
    let view_update = Arc::clone(&view);
    let view_metrics = Arc::clone(&view);
    let view_title = Arc::clone(&view);
    let config_update = config.clone();
    let shutdown_update = shutdown.clone();
    let app = Application::new(ApplicationConfig {
        title: format!(
            "Woven Lab | {} | {} | {} Hz tick | {:?}",
            config.woven.mode.description(),
            config.client_name,
            config.steps_per_second,
            config.present_mode
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
            setup(world, renderer, &view_setup);
        })),
        update: Some(Box::new(move |world, time| {
            if !shutdown_update.is_requested() {
                update(world, time, &config_update, &view_update);
            }
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
    })
    .with_shutdown_signal(shutdown);
    app.run()?;
    Ok(())
}

fn validate_target(target: Option<&str>) -> anyhow::Result<ConnectivityMode> {
    match target.unwrap_or("local") {
        "local" => Ok(ConnectivityMode::Loopback),
        "remote" | "cloud" => Ok(ConnectivityMode::RemoteQuic),
        _ => anyhow::bail!("WOVEN_LAB_TARGET must be `local`, `remote` or `cloud`"),
    }
}

fn validate_duration(
    mode: ConnectivityMode,
    value: Option<&str>,
) -> anyhow::Result<Option<Duration>> {
    let Some(value) = value else {
        anyhow::ensure!(
            mode != ConnectivityMode::RemoteQuic,
            "remote runs require WOVEN_LAB_DURATION_SECONDS (1..=300)"
        );
        return Ok(None);
    };
    let seconds = value
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=300).contains(seconds));
    Ok(Some(Duration::from_secs(seconds.context(
        "WOVEN_LAB_DURATION_SECONDS must be an integer in 1..=300",
    )?)))
}

fn read_config() -> anyhow::Result<LabConfig> {
    let target = match std::env::var("WOVEN_LAB_TARGET") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => anyhow::bail!("WOVEN_LAB_TARGET must be `local`, `remote` or `cloud`"),
    };
    let mode = validate_target(target.as_deref())?;
    let duration_value = std::env::var("WOVEN_LAB_DURATION_SECONDS").ok();
    let duration = validate_duration(mode, duration_value.as_deref())?;
    let mut woven = WovenConfig {
        mode,
        ..WovenConfig::default()
    };
    if mode == ConnectivityMode::RemoteQuic {
        // A local URL or the development token must never become a remote fallback.
        woven.endpoint = Some(std::env::var("WOVEN_LAB_REMOTE_URL")
            .or_else(|_| if target.as_deref() == Some("cloud") {
                std::env::var("WOVEN_LAB_CLOUD_URL")
            } else { Err(std::env::VarError::NotPresent) })
            .map_err(|_| anyhow::anyhow!("remote runs require WOVEN_LAB_REMOTE_URL (cloud alias also accepts WOVEN_LAB_CLOUD_URL)"))?);
        woven.ca_pem_file = Some(
            std::env::var_os("WOVEN_LAB_CA_PEM_FILE")
                .context("remote runs require WOVEN_LAB_CA_PEM_FILE")?
                .into(),
        );
        woven.token_file = Some(
            std::env::var_os("WOVEN_LAB_TOKEN_FILE")
                .context("remote runs require WOVEN_LAB_TOKEN_FILE")?
                .into(),
        );
        woven.dev_token.clear();
    } else {
        woven.endpoint = Some(std::env::var("WOVEN_LAB_URL").map_err(|_| {
            anyhow::anyhow!("set WOVEN_LAB_URL, for example quic://127.0.0.1:8081")
        })?);
    }
    woven.validate()?;
    let client_name = std::env::var("WOVEN_LAB_CLIENT").unwrap_or_else(|_| "lab".to_owned());
    anyhow::ensure!(
        mode != ConnectivityMode::RemoteQuic || (1..=64).contains(&client_name.len()),
        "remote WOVEN_LAB_CLIENT must contain 1..=64 UTF-8 bytes"
    );
    let rate_hz = std::env::var("WOVEN_LAB_RATE_HZ")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10.0_f64);
    anyhow::ensure!(valid_rate(rate_hz), "WOVEN_LAB_RATE_HZ must be in 0..=120");
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
        angular_speed.is_finite() && (0.0..=20.0).contains(&angular_speed),
        "WOVEN_LAB_ANGULAR_SPEED must be in 0..=20"
    );
    Ok(LabConfig {
        woven,
        duration,
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
    view: &Mutex<LabView>,
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
        view.lock().expect("lab state poisoned").cells.push(id);
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
            publish_rate_hz: config.rate_hz,
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
    view.local_woven_entity = local_woven_entity;
    for envelope in world.replicated_payloads().to_vec() {
        if envelope.channel != STATE_CHANNEL {
            continue;
        }
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
        let Ok(state) = serde_json::from_str::<CubeState>(&envelope.body_json) else {
            continue;
        };
        view.receive_peer(
            woven_entity,
            envelope.sequence,
            &state,
            Instant::now(),
            unix_time_seconds(),
        );
    }

    // Wall time compensates packet latency once; monotonic time bounds prediction.
    layout_cubes(world, &mut view, Instant::now());
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
    let stale_peers = view.stale_peers(Instant::now());
    view.metrics.summary = format!(
        "pub {publish_accepted_per_second:.0}/{publish_attempts_per_second:.0} Hz | {confirm_status} | recv {peer_updates_per_second:.0} Hz | active {} | stale {stale_peers} | errors {}",
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
        stale_peers,
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
    let now = Instant::now();
    view.states.insert(
        woven_entity,
        ObservedCube::new(state, now, unix_time_seconds()),
    );
    layout_cubes(world, view, now);
}

fn release_cube(world: &mut WeaverWorld, view: &mut LabView, woven_entity: u64) {
    view.forget_peer(woven_entity);
    layout_cubes(world, view, Instant::now());
}

/// Keep every window's grid stable even if protocol messages arrive in a
/// different order: cells are always assigned by ascending server entity ID.
fn layout_cubes(world: &mut WeaverWorld, view: &mut LabView, now: Instant) {
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
        let stale = Some(*woven_entity) != view.local_woven_entity && observed.is_stale(now);
        if let Some(renderable) = world.get_mut(cell) {
            let status = if stale { " (stale)" } else { "" };
            renderable.label = Some(format!(
                "#{}\n{}{status}",
                woven_entity, observed.state.client
            ));
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
                    observed.displayed_at = now;
                } else {
                    observed.advance(now);
                }
                mesh.transform.rotation = Quat::from_axis_angle(
                    Vec3::new(0.35, 1.0, 0.2).normalize(),
                    observed.displayed_rotation_radians,
                );
                mesh.color = if stale {
                    [0.38, 0.4, 0.44, 1.0]
                } else {
                    color_for(*woven_entity)
                };
                mesh.emissive = if stale { 0.04 } else { 0.28 };
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
    // Clamp wall-clock compensation too: delayed packets or clock skew cannot
    // introduce an unbounded extrapolation, and f64 arithmetic avoids f32 overflow.
    let elapsed_seconds = (now_seconds - state.published_at_seconds)
        .clamp(0.0, peer_timeout(state.publish_rate_hz).as_secs_f64());
    (f64::from(state.rotation_radians) + elapsed_seconds * f64::from(state.angular_speed))
        .rem_euclid(f64::from(std::f32::consts::TAU)) as f32
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

    #[test]
    fn target_defaults_to_local_and_accepts_explicit_local() {
        assert!(validate_target(None).is_ok());
        assert!(validate_target(Some("local")).is_ok());
    }

    #[test]
    fn remote_is_explicit_and_duration_is_bounded() {
        for target in ["remote", "cloud"] {
            let mode = validate_target(Some(target)).unwrap();
            assert_eq!(mode, ConnectivityMode::RemoteQuic);
            for value in [
                None,
                Some(""),
                Some("0"),
                Some("301"),
                Some("NaN"),
                Some("1.5"),
                Some("-1"),
            ] {
                assert!(validate_duration(mode, value).is_err());
            }
            for value in ["1", "300"] {
                assert!(validate_duration(mode, Some(value)).is_ok());
            }
        }
        assert_eq!(
            validate_duration(ConnectivityMode::Loopback, None).unwrap(),
            None
        );
    }

    #[test]
    fn invalid_targets_are_rejected() {
        for target in ["", "web", "LOCAL", " local "] {
            let error = validate_target(Some(target)).unwrap_err().to_string();
            assert!(error.contains("WOVEN_LAB_TARGET must be `local`, `remote` or `cloud`"));
        }
    }

    fn state(rate: f64, tick: u64) -> CubeState {
        CubeState {
            client: "peer".to_owned(),
            rotation_radians: 0.0,
            angular_speed: 1.0,
            published_at_seconds: 100.0,
            tick,
            publish_rate_hz: rate,
        }
    }

    #[test]
    fn timeout_tracks_cadence_with_a_floor_and_hard_cap() {
        for (rate, seconds) in [(120.0, 2), (10.0, 2), (1.0, 3), (0.2, 15), (0.01, 30)] {
            let now = Instant::now();
            let cube = ObservedCube::new(&state(rate, 1), now, 100.0);
            let timeout = Duration::from_secs(seconds);
            assert_eq!(peer_timeout(rate), timeout);
            assert!(
                !cube.is_stale(
                    (now + timeout)
                        .checked_sub(Duration::from_nanos(1))
                        .unwrap()
                )
            );
            assert!(cube.is_stale(now + timeout));
        }
        assert_eq!(peer_timeout(f64::MIN_POSITIVE), Duration::from_secs(30));
    }

    #[test]
    fn expiry_freezes_at_horizon_even_after_a_long_frame_gap() {
        let now = Instant::now();
        let mut cube = ObservedCube::new(&state(10.0, 1), now, 100.0);
        cube.advance(now + Duration::from_millis(500));
        assert_eq!(cube.displayed_rotation_radians, 0.5);
        cube.advance(now + Duration::from_secs(50));
        assert_eq!(cube.displayed_rotation_radians, 2.0);
        for seconds in [51, 100, 1000] {
            cube.advance(now + Duration::from_secs(seconds));
            assert_eq!(cube.displayed_rotation_radians, 2.0);
            assert!(cube.is_stale(now + Duration::from_secs(seconds)));
        }
    }

    #[test]
    fn cached_delayed_and_old_updates_do_not_refresh_or_count() {
        let now = Instant::now();
        let mut view = LabView::new();
        // An old sender timestamp is not a freshness clock. Only the first
        // locally observed new sequence/tick earns a bounded receipt window.
        let first = state(10.0, 10);
        assert!(view.receive_peer(7, 11, &first, now, 1000.0));
        for seconds in [1, 3, 10, 100] {
            let later = now + Duration::from_secs(seconds);
            assert!(!view.receive_peer(7, 11, &first, later, 1000.0));
            assert!(!view.receive_peer(7, 12, &first, later, 1000.0));
            assert!(!view.receive_peer(7, 10, &state(10.0, 11), later, 1000.0));
            assert!(!view.receive_peer(7, 12, &state(10.0, 9), later, 1000.0));
            assert_eq!(view.states[&7].received_at, now);
        }
        assert_eq!(view.stale_peers(now + Duration::from_secs(100)), 1);
        assert_eq!(view.metrics.peer_updates_received, 1);
        assert_eq!(view.received_sequences[&7], 11);
    }

    #[test]
    fn newer_state_recovers_without_snap_or_frozen_time_correction() {
        let now = Instant::now();
        let mut view = LabView::new();
        assert!(view.receive_peer(7, 1, &state(10.0, 1), now, 100.0));
        let recovery = now + Duration::from_secs(100);
        view.states.get_mut(&7).unwrap().advance(recovery);
        let frozen = view.states[&7].displayed_rotation_radians;
        let mut next = state(1.0, 2);
        next.rotation_radians = 4.0;
        next.angular_speed = 0.0;
        assert!(view.receive_peer(7, 2, &next, recovery, 100.0));
        assert_eq!(view.stale_peers(recovery), 0);
        let cube = view.states.get_mut(&7).unwrap();
        cube.advance(recovery);
        assert_eq!(cube.displayed_rotation_radians, frozen);
        cube.advance(recovery + Duration::from_millis(100));
        let correction = shortest_rotation_delta(cube.displayed_rotation_radians, frozen);
        assert!(correction > 0.0 && correction <= 0.100_001);
        assert!(!cube.is_stale(recovery + Duration::from_secs(2)));
        assert!(cube.is_stale(recovery + Duration::from_secs(3)));
        assert_eq!(view.metrics.peer_updates_received, 2);
    }

    #[test]
    fn old_payload_defaults_and_invalid_numbers_are_rejected_without_refresh() {
        let old = r#"{"client":"old","rotation_radians":0.0,"angular_speed":1.0,"published_at_seconds":100.0,"tick":1}"#;
        let decoded: CubeState = serde_json::from_str(old).unwrap();
        assert_eq!(decoded.publish_rate_hz, 10.0);
        assert!(valid_state(&decoded));
        let now = Instant::now();
        let mut view = LabView::new();
        assert!(view.receive_peer(7, 1, &decoded, now, 100.0));
        let mut invalid = Vec::new();
        for rate in [0.0, -1.0, 121.0, f64::NAN, f64::INFINITY] {
            invalid.push(state(rate, 2));
        }
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut next = state(10.0, 2);
            next.rotation_radians = value;
            invalid.push(next);
        }
        for value in [-1.0, 21.0, f32::NAN, f32::INFINITY] {
            let mut next = state(10.0, 2);
            next.angular_speed = value;
            invalid.push(next);
        }
        for value in [-1.0, f64::NAN, f64::INFINITY] {
            let mut next = state(10.0, 2);
            next.published_at_seconds = value;
            invalid.push(next);
        }
        for next in invalid {
            assert!(!view.receive_peer(7, 2, &next, now + Duration::from_secs(10), 100.0));
        }
        assert_eq!(view.states[&7].received_at, now);
        assert_eq!(view.received_sequences[&7], 1);
        assert_eq!(view.metrics.peer_updates_received, 1);
        let mut extreme = state(10.0, 2);
        extreme.rotation_radians = f32::MAX;
        assert!(rotation_at(&extreme, f64::MAX).is_finite());
        extreme.published_at_seconds = f64::MAX;
        assert!(rotation_at(&extreme, 100.0).is_finite());
    }

    #[test]
    fn stale_peers_stay_assigned_until_departure_cleans_state_and_sequence() {
        let now = Instant::now();
        let mut view = LabView::new();
        assert!(view.receive_peer(7, 1, &state(10.0, 1), now, 100.0));
        assert!(view.receive_peer(8, 1, &state(10.0, 1), now, 100.0));
        assert_eq!(view.stale_peers(now + Duration::from_secs(10)), 2);
        assert_eq!(view.states.keys().copied().collect::<Vec<_>>(), vec![7, 8]);
        view.forget_peer(7);
        view.forget_peer(7);
        assert!(!view.states.contains_key(&7));
        assert!(!view.received_sequences.contains_key(&7));
        assert!(view.states.contains_key(&8));
        assert!(view.received_sequences.contains_key(&8));
        assert_eq!(view.stale_peers(now + Duration::from_secs(10)), 1);
    }

    #[test]
    fn local_cube_and_echoes_are_not_peer_freshness_or_receive_metrics() {
        let now = Instant::now();
        let mut view = LabView::new();
        view.local_woven_entity = Some(7);
        view.states
            .insert(7, ObservedCube::new(&state(10.0, 1), now, 100.0));
        assert!(!view.receive_peer(7, 2, &state(10.0, 2), now + Duration::from_secs(10), 100.0));
        assert_eq!(view.stale_peers(now + Duration::from_secs(100)), 0);
        assert_eq!(view.metrics.peer_updates_received, 0);
        assert!(view.received_sequences.is_empty());
        assert_eq!(view.states[&7].state.tick, 1);
    }

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
        let metrics = metrics(
            Some(Instant::now().checked_sub(Duration::from_secs(5)).unwrap()),
            42,
        );
        assert_eq!(
            confirm_status(&metrics, 10.0, 0.0),
            "SERVER RECV LOST 5s (last seq 42)"
        );
    }

    #[test]
    fn confirm_status_tolerates_slow_publish_cadence() {
        // At 1 Hz the silence threshold stretches past two seconds so a
        // healthy low-rate run is not misreported as lost.
        let metrics = metrics(
            Some(
                Instant::now()
                    .checked_sub(Duration::from_millis(2_500))
                    .unwrap(),
            ),
            3,
        );
        assert_eq!(confirm_status(&metrics, 1.0, 1.0), "srv-recv 1 Hz seq 3");
    }
}
