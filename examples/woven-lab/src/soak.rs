//! Bounded managed headless soak runner, independent of rendering and simulation time.

use super::{CubeState, LabConfig, read_config};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use weaver_woven::{
    ConnectivityMode, DeliveryClass, Payload, PersistenceClass, WovenAdapter, WovenAdapterError,
};

const CHANNEL: u64 = 1;
const DEFAULT_STARTUP_SECONDS: u64 = 60;
const MAX_STARTUP_SECONDS: u64 = 15 * 60;
const DEFAULT_FINAL_DRAIN_SECONDS: u64 = 5;
const MAX_FINAL_DRAIN_SECONDS: u64 = 30;
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TerminalStatus {
    Passed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TerminalReason {
    Passed,
    InvalidConfiguration,
    StartupFailed,
    PublishFailed,
    ServerRejection,
    TransportFailure,
    InvalidEcho,
    SustainedConfirmationLoss,
    IncompleteDuration,
    ZeroSends,
    ZeroConfirmations,
    FinalConfirmationLoss,
}

#[derive(Debug, Serialize)]
struct ResolvedSoakConfig {
    target: String,
    connectivity: &'static str,
    transport: &'static str,
    endpoint: &'static str,
    credentials: &'static str,
    namespace_id: &'static str,
    session_id: &'static str,
    space_id: u64,
    space_epoch: u64,
    channel: u64,
    delivery: &'static str,
    persistence: &'static str,
    publish_rate_hz: f64,
    startup_timeout_seconds: u64,
    final_drain_seconds: u64,
}

impl ResolvedSoakConfig {
    fn unresolved(target: String) -> Self {
        Self {
            target,
            connectivity: "managed_quic",
            transport: "quic",
            endpoint: "redacted",
            credentials: "redacted_files",
            namespace_id: "redacted",
            session_id: "redacted",
            space_id: 1,
            space_epoch: 1,
            channel: CHANNEL,
            delivery: "reliable_ordered",
            persistence: "ephemeral",
            publish_rate_hz: 0.0,
            startup_timeout_seconds: 0,
            final_drain_seconds: 0,
        }
    }
}

#[derive(Debug, Default, Serialize)]
struct SoakCounts {
    attempted: u64,
    sent: u64,
    confirmed: u64,
    peer_received: u64,
    errors: u64,
    disconnects: u64,
}

#[derive(Debug, Serialize)]
struct SourceIdentifier {
    commit: String,
    worktree_dirty: Option<bool>,
}

#[derive(Debug, Serialize)]
struct SoakResult {
    schema_version: u32,
    result_type: &'static str,
    status: TerminalStatus,
    reason: TerminalReason,
    client_name: String,
    weaver: SourceIdentifier,
    woven: SourceIdentifier,
    config: ResolvedSoakConfig,
    process_started_at: String,
    active_started_at: Option<String>,
    active_ended_at: Option<String>,
    finished_at: String,
    requested_active_duration_seconds: Option<u64>,
    actual_active_duration_seconds: f64,
    counts: SoakCounts,
}

impl SoakResult {
    fn new() -> Self {
        let target = sanitized_target();
        let client_name = std::env::var("WOVEN_LAB_CLIENT").unwrap_or_else(|_| "lab".to_owned());
        let client_name = if valid_client_name(&client_name) {
            client_name
        } else {
            "invalid".to_owned()
        };
        let (weaver, woven) = source_identifiers();
        Self {
            schema_version: 1,
            result_type: "woven_lab_managed_soak",
            status: TerminalStatus::Failed,
            reason: TerminalReason::InvalidConfiguration,
            client_name,
            weaver,
            woven,
            config: ResolvedSoakConfig::unresolved(target),
            process_started_at: timestamp(),
            active_started_at: None,
            active_ended_at: None,
            finished_at: String::new(),
            requested_active_duration_seconds: parse_duration_value(
                std::env::var("WOVEN_LAB_DURATION_SECONDS").ok().as_deref(),
                600,
            )
            .ok(),
            actual_active_duration_seconds: 0.0,
            counts: SoakCounts::default(),
        }
    }

    fn finish(&mut self, reason: TerminalReason) {
        self.reason = reason;
        self.status = if reason == TerminalReason::Passed {
            TerminalStatus::Passed
        } else {
            TerminalStatus::Failed
        };
        self.finished_at = timestamp();
    }
}

#[derive(Clone, Copy, Debug)]
struct SummaryInput {
    requested_duration: Duration,
    actual_duration: Duration,
    active_completed: bool,
    attempted: u64,
    sent: u64,
    confirmed: u64,
    last_sent_sequence: u64,
    last_confirmed_sequence: u64,
    errors: u64,
    disconnects: u64,
    prior_failure: Option<TerminalReason>,
}

struct ActiveState {
    active_started: Instant,
    active_deadline: Instant,
    final_deadline: Instant,
    next_publish_at: Instant,
    interval: Duration,
    local_entity: u64,
    last_sent_sequence: u64,
    last_confirmed_sequence: u64,
    first_unconfirmed_at: Option<Instant>,
    last_confirm_at: Option<Instant>,
    confirmation_timeout: Duration,
}

/// Execute the requested soak and emit exactly one terminal JSON record to stdout.
pub(super) fn run() -> bool {
    let mut result = SoakResult::new();
    let reason = run_inner(&mut result);
    result.finish(reason);
    match serde_json::to_string(&result) {
        Ok(json) => println!("{json}"),
        Err(_) => {
            // Every field has an infallible JSON representation. Retain a machine-readable
            // fallback rather than exposing configuration through a formatting error.
            println!(
                "{{\"schema_version\":1,\"result_type\":\"woven_lab_managed_soak\",\"status\":\"failed\",\"reason\":\"result_serialization_failed\"}}"
            );
        }
    }
    result.status == TerminalStatus::Passed
}

fn run_inner(result: &mut SoakResult) -> TerminalReason {
    let soak_value = std::env::var("WOVEN_LAB_SOAK").ok();
    let target = std::env::var("WOVEN_LAB_TARGET").ok();
    let headless_present = std::env::var_os("WEAVER_HEADLESS").is_some();
    let Ok(config) = read_config() else {
        result.counts.errors = 1;
        return TerminalReason::InvalidConfiguration;
    };
    let Some(duration) = config.duration else {
        result.counts.errors = 1;
        return TerminalReason::InvalidConfiguration;
    };
    if validate_soak_selection(
        soak_value.as_deref(),
        target.as_deref(),
        config.woven.mode,
        Some(duration),
        headless_present,
    )
    .is_err()
        || !valid_client_name(&config.client_name)
    {
        result.counts.errors = 1;
        return TerminalReason::InvalidConfiguration;
    }
    let Ok(startup_seconds) = bounded_env_seconds(
        "WOVEN_LAB_SOAK_STARTUP_SECONDS",
        DEFAULT_STARTUP_SECONDS,
        MAX_STARTUP_SECONDS,
    ) else {
        result.counts.errors = 1;
        return TerminalReason::InvalidConfiguration;
    };
    let Ok(final_drain_seconds) = bounded_env_seconds(
        "WOVEN_LAB_SOAK_FINAL_DRAIN_SECONDS",
        DEFAULT_FINAL_DRAIN_SECONDS,
        MAX_FINAL_DRAIN_SECONDS,
    ) else {
        result.counts.errors = 1;
        return TerminalReason::InvalidConfiguration;
    };

    result.client_name.clone_from(&config.client_name);
    result.requested_active_duration_seconds = Some(duration.as_secs());
    result.config = ResolvedSoakConfig {
        target: target.unwrap_or_else(|| "invalid".to_owned()),
        connectivity: "managed_quic",
        transport: "quic",
        endpoint: "redacted",
        credentials: "redacted_files",
        namespace_id: "redacted",
        session_id: "redacted",
        space_id: config.woven.space_id,
        space_epoch: config.woven.space_epoch,
        channel: CHANNEL,
        delivery: "reliable_ordered",
        persistence: "ephemeral",
        publish_rate_hz: config.rate_hz,
        startup_timeout_seconds: startup_seconds,
        final_drain_seconds,
    };

    let mut woven_config = config.woven.clone();
    woven_config.run_deadline = Some(Instant::now() + Duration::from_secs(startup_seconds));
    let Ok(mut adapter) = WovenAdapter::new(woven_config) else {
        result.counts.errors = 1;
        return TerminalReason::StartupFailed;
    };
    if adapter.start().is_err() {
        result.counts.errors = 1;
        return TerminalReason::StartupFailed;
    }
    let Some(local_entity) = adapter.entity_id() else {
        result.counts.errors = 1;
        adapter.stop();
        return TerminalReason::StartupFailed;
    };

    let active_started = Instant::now();
    let active_deadline = active_started + duration;
    let final_deadline = active_deadline + Duration::from_secs(final_drain_seconds);
    adapter.set_run_deadline(Some(final_deadline));
    result.active_started_at = Some(timestamp());
    let mut state = ActiveState {
        active_started,
        active_deadline,
        final_deadline,
        next_publish_at: active_started,
        interval: Duration::from_secs_f64(1.0 / config.rate_hz),
        local_entity,
        last_sent_sequence: 0,
        last_confirmed_sequence: 0,
        first_unconfirmed_at: None,
        last_confirm_at: None,
        confirmation_timeout: confirmation_timeout(config.rate_hz),
    };

    let mut prior_failure = run_active(&config, &mut adapter, &mut state, result);
    let active_ended = Instant::now();
    result.actual_active_duration_seconds = active_ended
        .saturating_duration_since(active_started)
        .as_secs_f64();
    result.active_ended_at = Some(timestamp());
    let active_completed = active_ended >= active_deadline;

    if prior_failure.is_none() && active_completed {
        prior_failure = drain_final(&mut adapter, &mut state, result);
    }
    adapter.stop();

    evaluate_summary(SummaryInput {
        requested_duration: duration,
        actual_duration: active_ended.saturating_duration_since(active_started),
        active_completed,
        attempted: result.counts.attempted,
        sent: result.counts.sent,
        confirmed: result.counts.confirmed,
        last_sent_sequence: state.last_sent_sequence,
        last_confirmed_sequence: state.last_confirmed_sequence,
        errors: result.counts.errors,
        disconnects: result.counts.disconnects,
        prior_failure,
    })
}

fn run_active(
    config: &LabConfig,
    adapter: &mut WovenAdapter,
    state: &mut ActiveState,
    result: &mut SoakResult,
) -> Option<TerminalReason> {
    while Instant::now() < state.active_deadline {
        if let Err(error) = drain_once(adapter, state, result) {
            result.counts.errors += 1;
            if is_disconnect(&error) {
                result.counts.disconnects += 1;
            }
            return Some(classify_drain_error(&error));
        }

        let now = Instant::now();
        if confirmation_is_sustained_loss(state, now) {
            result.counts.errors += 1;
            return Some(TerminalReason::SustainedConfirmationLoss);
        }
        if now >= state.next_publish_at && now < state.active_deadline {
            result.counts.attempted += 1;
            let sequence = state.last_sent_sequence + 1;
            let elapsed = now
                .saturating_duration_since(state.active_started)
                .as_secs_f64();
            let payload = Payload {
                body: CubeState {
                    client: config.client_name.clone(),
                    rotation_radians: ((elapsed * f64::from(config.angular_speed))
                        .rem_euclid(f64::from(std::f32::consts::TAU)))
                        as f32,
                    angular_speed: config.angular_speed,
                    published_at_seconds: unix_time_seconds(),
                    tick: sequence - 1,
                    publish_rate_hz: config.rate_hz,
                },
                sequence,
                revision: sequence,
            };
            if let Err(error) = adapter.publish(
                CHANNEL,
                None,
                &payload,
                DeliveryClass::ReliableOrdered,
                PersistenceClass::Ephemeral,
            ) {
                result.counts.errors += 1;
                if is_disconnect(&error) {
                    result.counts.disconnects += 1;
                }
                return Some(TerminalReason::PublishFailed);
            }
            result.counts.sent += 1;
            state.last_sent_sequence = sequence;
            state.first_unconfirmed_at.get_or_insert(Instant::now());
            // Never burst to catch up after a slow operation: every successful publish
            // is followed by a full wall-clock interval.
            state.next_publish_at = Instant::now() + state.interval;
        }
        sleep_until(state.next_publish_at.min(state.active_deadline));
    }
    None
}

fn drain_final(
    adapter: &mut WovenAdapter,
    state: &mut ActiveState,
    result: &mut SoakResult,
) -> Option<TerminalReason> {
    while state.last_confirmed_sequence < state.last_sent_sequence
        && Instant::now() < state.final_deadline
    {
        if let Err(error) = drain_once(adapter, state, result) {
            result.counts.errors += 1;
            if is_disconnect(&error) {
                result.counts.disconnects += 1;
            }
            return Some(classify_drain_error(&error));
        }
        sleep_until((Instant::now() + DRAIN_POLL_INTERVAL).min(state.final_deadline));
    }
    None
}

fn drain_once(
    adapter: &mut WovenAdapter,
    state: &mut ActiveState,
    result: &mut SoakResult,
) -> Result<(), WovenAdapterError> {
    for envelope in adapter.drain_envelopes()? {
        if envelope.channel != CHANNEL {
            continue;
        }
        if envelope.entity == Some(state.local_entity) {
            let Ok(echo) = serde_json::from_str::<CubeState>(&envelope.body_json) else {
                return Err(WovenAdapterError::UnexpectedMessage(
                    "invalid local publish echo".to_owned(),
                ));
            };
            if envelope.sequence == 0
                || envelope.sequence > state.last_sent_sequence
                || echo.tick + 1 != envelope.sequence
                || echo.client != result.client_name
            {
                return Err(WovenAdapterError::UnexpectedMessage(
                    "invalid local publish echo".to_owned(),
                ));
            }
            if envelope.sequence > state.last_confirmed_sequence {
                result.counts.confirmed += 1;
                state.last_confirmed_sequence = envelope.sequence;
                state.last_confirm_at = Some(Instant::now());
                if state.last_confirmed_sequence == state.last_sent_sequence {
                    state.first_unconfirmed_at = None;
                }
            }
        } else {
            result.counts.peer_received += 1;
        }
    }
    Ok(())
}

fn confirmation_is_sustained_loss(state: &ActiveState, now: Instant) -> bool {
    if state.last_confirmed_sequence >= state.last_sent_sequence {
        return false;
    }
    let since = state.last_confirm_at.or(state.first_unconfirmed_at);
    since.is_some_and(|at| now.saturating_duration_since(at) >= state.confirmation_timeout)
}

fn confirmation_timeout(rate_hz: f64) -> Duration {
    Duration::from_secs_f64((3.0 / rate_hz).max(2.0))
}

fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(
            deadline
                .saturating_duration_since(now)
                .min(DRAIN_POLL_INTERVAL),
        );
    }
}

fn validate_soak_selection(
    soak_value: Option<&str>,
    target: Option<&str>,
    mode: ConnectivityMode,
    duration: Option<Duration>,
    headless_present: bool,
) -> Result<(), ()> {
    if soak_value != Some("1")
        || headless_present
        || !matches!(target, Some("managed-local" | "cloud"))
        || mode != ConnectivityMode::ManagedQuic
        || duration.is_none()
    {
        return Err(());
    }
    Ok(())
}

fn valid_client_name(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn bounded_env_seconds(name: &str, default: u64, maximum: u64) -> Result<u64, ()> {
    match std::env::var(name) {
        Ok(value) => parse_duration_value(Some(&value), maximum),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(_) => Err(()),
    }
}

fn parse_duration_value(value: Option<&str>, maximum: u64) -> Result<u64, ()> {
    value
        .ok_or(())?
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=maximum).contains(seconds))
        .ok_or(())
}

fn evaluate_summary(input: SummaryInput) -> TerminalReason {
    if let Some(reason) = input.prior_failure {
        return reason;
    }
    if !input.active_completed || input.actual_duration < input.requested_duration {
        return TerminalReason::IncompleteDuration;
    }
    if input.errors > 0 {
        return TerminalReason::TransportFailure;
    }
    if input.disconnects > 0 {
        return TerminalReason::TransportFailure;
    }
    if input.attempted == 0 || input.sent == 0 {
        return TerminalReason::ZeroSends;
    }
    if input.confirmed == 0 {
        return TerminalReason::ZeroConfirmations;
    }
    if input.confirmed != input.sent || input.last_confirmed_sequence != input.last_sent_sequence {
        return TerminalReason::FinalConfirmationLoss;
    }
    TerminalReason::Passed
}

fn classify_drain_error(error: &WovenAdapterError) -> TerminalReason {
    match error {
        WovenAdapterError::ServerRejected(_) => TerminalReason::ServerRejection,
        WovenAdapterError::UnexpectedMessage(_) | WovenAdapterError::Serialization(_) => {
            TerminalReason::InvalidEcho
        }
        _ => TerminalReason::TransportFailure,
    }
}

fn is_disconnect(error: &WovenAdapterError) -> bool {
    matches!(
        error,
        WovenAdapterError::ClientFailed(_) | WovenAdapterError::NotRunning
    )
}

fn sanitized_target() -> String {
    match std::env::var("WOVEN_LAB_TARGET").as_deref() {
        Ok("managed-local") => "managed-local".to_owned(),
        Ok("cloud") => "cloud".to_owned(),
        _ => "invalid".to_owned(),
    }
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn unix_time_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn source_identifiers() -> (SourceIdentifier, SourceIdentifier) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let weaver_root = manifest.parent().and_then(Path::parent).unwrap_or(manifest);
    let woven_root = weaver_root
        .parent()
        .map_or_else(|| weaver_root.to_path_buf(), |parent| parent.join("woven"));
    (
        source_identifier(weaver_root),
        source_identifier(&woven_root),
    )
}

fn source_identifier(directory: &Path) -> SourceIdentifier {
    let commit = git_output(directory, &["rev-parse", "HEAD"])
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unavailable".to_owned());
    let worktree_dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(directory)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty());
    SourceIdentifier {
        commit,
        worktree_dirty,
    }
}

fn git_output(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_summary() -> SummaryInput {
        SummaryInput {
            requested_duration: Duration::from_secs(600),
            actual_duration: Duration::from_secs(600),
            active_completed: true,
            attempted: 6000,
            sent: 6000,
            confirmed: 6000,
            last_sent_sequence: 6000,
            last_confirmed_sequence: 6000,
            errors: 0,
            disconnects: 0,
            prior_failure: None,
        }
    }

    #[test]
    fn soak_mode_is_explicit_managed_and_distinct_from_headless_smoke() {
        for target in ["managed-local", "cloud"] {
            assert!(
                validate_soak_selection(
                    Some("1"),
                    Some(target),
                    ConnectivityMode::ManagedQuic,
                    Some(Duration::from_secs(600)),
                    false,
                )
                .is_ok()
            );
        }
        for (soak, target, mode, duration, headless) in [
            (
                None,
                Some("cloud"),
                ConnectivityMode::ManagedQuic,
                Some(Duration::from_secs(600)),
                false,
            ),
            (
                Some("0"),
                Some("cloud"),
                ConnectivityMode::ManagedQuic,
                Some(Duration::from_secs(600)),
                false,
            ),
            (
                Some("1"),
                Some("local"),
                ConnectivityMode::Loopback,
                Some(Duration::from_secs(600)),
                false,
            ),
            (
                Some("1"),
                Some("remote"),
                ConnectivityMode::RemoteQuic,
                Some(Duration::from_secs(600)),
                false,
            ),
            (
                Some("1"),
                Some("cloud"),
                ConnectivityMode::ManagedQuic,
                None,
                false,
            ),
            (
                Some("1"),
                Some("cloud"),
                ConnectivityMode::ManagedQuic,
                Some(Duration::from_secs(600)),
                true,
            ),
        ] {
            assert!(validate_soak_selection(soak, target, mode, duration, headless).is_err());
        }
    }

    #[test]
    fn soak_durations_are_bounded_without_waiting() {
        assert_eq!(parse_duration_value(Some("1"), 600), Ok(1));
        assert_eq!(parse_duration_value(Some("600"), 600), Ok(600));
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("601"),
            Some("1.5"),
            Some("-1"),
        ] {
            assert_eq!(parse_duration_value(value, 600), Err(()));
        }
        assert_eq!(confirmation_timeout(120.0), Duration::from_secs(2));
        assert_eq!(confirmation_timeout(1.0), Duration::from_secs(3));
        assert!(Duration::from_secs_f64(1.0 / 120.0) >= Duration::from_micros(8_333));
    }

    #[test]
    fn summary_passes_only_complete_fully_confirmed_runs() {
        assert_eq!(evaluate_summary(passing_summary()), TerminalReason::Passed);

        let cases = [
            (
                SummaryInput {
                    active_completed: false,
                    ..passing_summary()
                },
                TerminalReason::IncompleteDuration,
            ),
            (
                SummaryInput {
                    actual_duration: Duration::from_secs(599),
                    ..passing_summary()
                },
                TerminalReason::IncompleteDuration,
            ),
            (
                SummaryInput {
                    attempted: 0,
                    sent: 0,
                    confirmed: 0,
                    last_sent_sequence: 0,
                    last_confirmed_sequence: 0,
                    ..passing_summary()
                },
                TerminalReason::ZeroSends,
            ),
            (
                SummaryInput {
                    confirmed: 0,
                    last_confirmed_sequence: 0,
                    ..passing_summary()
                },
                TerminalReason::ZeroConfirmations,
            ),
            (
                SummaryInput {
                    confirmed: 5999,
                    last_confirmed_sequence: 5999,
                    ..passing_summary()
                },
                TerminalReason::FinalConfirmationLoss,
            ),
            (
                SummaryInput {
                    prior_failure: Some(TerminalReason::SustainedConfirmationLoss),
                    ..passing_summary()
                },
                TerminalReason::SustainedConfirmationLoss,
            ),
            (
                SummaryInput {
                    errors: 1,
                    ..passing_summary()
                },
                TerminalReason::TransportFailure,
            ),
            (
                SummaryInput {
                    disconnects: 1,
                    ..passing_summary()
                },
                TerminalReason::TransportFailure,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(evaluate_summary(input), expected);
        }
    }

    #[test]
    fn result_json_is_single_record_with_redacted_network_configuration() {
        let result = SoakResult {
            schema_version: 1,
            result_type: "woven_lab_managed_soak",
            status: TerminalStatus::Passed,
            reason: TerminalReason::Passed,
            client_name: "soak-1".to_owned(),
            weaver: SourceIdentifier {
                commit: "a".repeat(40),
                worktree_dirty: Some(false),
            },
            woven: SourceIdentifier {
                commit: "b".repeat(40),
                worktree_dirty: Some(false),
            },
            config: ResolvedSoakConfig {
                target: "cloud".to_owned(),
                connectivity: "managed_quic",
                transport: "quic",
                endpoint: "redacted",
                credentials: "redacted_files",
                namespace_id: "redacted",
                session_id: "redacted",
                space_id: 1,
                space_epoch: 1,
                channel: 1,
                delivery: "reliable_ordered",
                persistence: "ephemeral",
                publish_rate_hz: 10.0,
                startup_timeout_seconds: 60,
                final_drain_seconds: 5,
            },
            process_started_at: "2026-01-01T00:00:00.000Z".to_owned(),
            active_started_at: Some("2026-01-01T00:00:01.000Z".to_owned()),
            active_ended_at: Some("2026-01-01T00:10:01.000Z".to_owned()),
            finished_at: "2026-01-01T00:10:02.000Z".to_owned(),
            requested_active_duration_seconds: Some(600),
            actual_active_duration_seconds: 600.0,
            counts: SoakCounts {
                attempted: 6000,
                sent: 6000,
                confirmed: 6000,
                peer_received: 2,
                errors: 0,
                disconnects: 0,
            },
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains('\n'));
        assert!(json.contains("\"requested_active_duration_seconds\":600"));
        assert!(json.contains("\"endpoint\":\"redacted\""));
        assert!(json.contains("\"channel\":1"));
        assert!(!json.contains("quic://"));
        assert!(!json.contains("/tmp/"));
    }
}
