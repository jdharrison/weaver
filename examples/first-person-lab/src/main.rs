//! Native desktop entry point for First-Person Lab.

#[cfg(not(target_arch = "wasm32"))]
use anyhow::Context;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};
#[cfg(not(target_arch = "wasm32"))]
use weaver_app_core::{RealtimeCommand, RealtimeDriver, RealtimeEvent};
#[cfg(not(target_arch = "wasm32"))]
use weaver_woven::{ConnectivityMode, WovenConfig, WovenRealtimeDriver};

#[cfg(not(target_arch = "wasm32"))]
const CONNECTED_LOG_MESSAGE: &str = "First-Person Lab: connected successfully; delayed client logging test (3 seconds after connection).";
#[cfg(not(target_arch = "wasm32"))]
const CONNECTED_LOG_DELAY: Duration = Duration::from_secs(3);

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct ConnectionLogSchedule {
    connected: bool,
    deadline: Option<Instant>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ConnectionLogSchedule {
    fn poll(&mut self, events: &[RealtimeEvent], now: Instant) -> bool {
        for event in events {
            match event {
                RealtimeEvent::Connected { .. } if !self.connected => {
                    self.connected = true;
                    self.deadline = Some(now + CONNECTED_LOG_DELAY);
                }
                RealtimeEvent::Disconnected { .. } => {
                    self.connected = false;
                    self.deadline = None;
                }
                _ => {}
            }
        }
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            // Consume before enqueueing: even a full queue must not cause retries.
            self.deadline = None;
            return true;
        }
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct FirstPersonRealtimeDriver {
    driver: WovenRealtimeDriver,
    log_schedule: ConnectionLogSchedule,
}

#[cfg(not(target_arch = "wasm32"))]
impl RealtimeDriver for FirstPersonRealtimeDriver {
    fn poll(&mut self, commands: &[RealtimeCommand], events: &mut Vec<RealtimeEvent>) {
        let start = events.len();
        self.driver.poll(commands, events);
        if self.log_schedule.poll(&events[start..], Instant::now()) {
            // This optional diagnostic must not interfere with the simulation connection.
            let _ = self.driver.log_info(CONNECTED_LOG_MESSAGE);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let app = Box::new(match std::env::var("FIRST_PERSON_DISPLAY_NAME") {
        Ok(display_name) => first_person_lab::FirstPersonLab::with_display_name(&display_name),
        Err(_) => first_person_lab::FirstPersonLab::new(),
    });
    let platform = weaver_platform_desktop::DesktopConfig::default();
    if let Some(driver) = realtime_driver_from_env()? {
        weaver_platform_desktop::run_with_realtime(app, platform, Box::new(driver))?;
    } else {
        weaver_platform_desktop::run(app, platform)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn realtime_driver_from_env() -> anyhow::Result<Option<FirstPersonRealtimeDriver>> {
    let Some(endpoint) = std::env::var("FIRST_PERSON_WOVEN_URL").ok() else {
        return Ok(None);
    };
    let target = std::env::var("FIRST_PERSON_WOVEN_TARGET").unwrap_or_else(|_| "local".to_owned());
    let mut config = WovenConfig {
        endpoint: Some(endpoint),
        namespace_id: optional_id("FIRST_PERSON_WOVEN_NAMESPACE_ID", 2)?,
        session_id: optional_id("FIRST_PERSON_WOVEN_SESSION_ID", 2)?,
        space_id: optional_id("FIRST_PERSON_WOVEN_SPACE_ID", 3)?,
        space_epoch: optional_id("FIRST_PERSON_WOVEN_SPACE_EPOCH", 1)?,
        ..WovenConfig::default()
    };
    anyhow::ensure!(
        config.space_id >= 3,
        "FIRST_PERSON_WOVEN_SPACE_ID must identify a preconfigured spatial subspace (ID 3 or greater)"
    );
    match target.as_str() {
        "local" => {
            config.mode = ConnectivityMode::Loopback;
            config.dev_token = std::env::var("FIRST_PERSON_WOVEN_DEV_TOKEN")
                .unwrap_or_else(|_| "dev-token".to_owned());
        }
        "managed" => {
            config.mode = ConnectivityMode::ManagedQuic;
            config.ca_pem_file = Some(
                std::env::var_os("FIRST_PERSON_WOVEN_CA_PEM_FILE")
                    .context("managed Woven requires FIRST_PERSON_WOVEN_CA_PEM_FILE")?
                    .into(),
            );
            config.token_file = Some(
                std::env::var_os("FIRST_PERSON_WOVEN_TOKEN_FILE")
                    .context("managed Woven requires FIRST_PERSON_WOVEN_TOKEN_FILE")?
                    .into(),
            );
        }
        "remote" => {
            config.mode = ConnectivityMode::RemoteQuic;
            config.ca_pem_file = Some(
                std::env::var_os("FIRST_PERSON_WOVEN_CA_PEM_FILE")
                    .context("remote Woven requires FIRST_PERSON_WOVEN_CA_PEM_FILE")?
                    .into(),
            );
            config.token_file = Some(
                std::env::var_os("FIRST_PERSON_WOVEN_TOKEN_FILE")
                    .context("remote Woven requires FIRST_PERSON_WOVEN_TOKEN_FILE")?
                    .into(),
            );
        }
        _ => anyhow::bail!("FIRST_PERSON_WOVEN_TARGET must be local, managed, or remote"),
    }
    config.validate()?;
    Ok(Some(FirstPersonRealtimeDriver {
        driver: WovenRealtimeDriver::connect(config)?,
        log_schedule: ConnectionLogSchedule::default(),
    }))
}

#[cfg(not(target_arch = "wasm32"))]
fn optional_id(name: &str, default: u64) -> anyhow::Result<u64> {
    let value = std::env::var(name)
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()
        .with_context(|| format!("{name} must be a positive integer"))?
        .unwrap_or(default);
    anyhow::ensure!(value > 0, "{name} must be nonzero");
    Ok(value)
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn connected() -> RealtimeEvent {
        RealtimeEvent::Connected { entity_id: 7 }
    }

    fn disconnected() -> RealtimeEvent {
        RealtimeEvent::Disconnected {
            reason: "test disconnect".to_owned(),
        }
    }

    #[test]
    fn connected_log_waits_three_seconds_and_fires_exactly_once() {
        let mut schedule = ConnectionLogSchedule::default();
        let start = Instant::now();
        assert!(!schedule.poll(&[], start));
        assert!(!schedule.poll(&[connected()], start));
        assert!(!schedule.poll(
            &[],
            start + CONNECTED_LOG_DELAY.saturating_sub(Duration::from_nanos(1)),
        ));
        assert!(schedule.poll(&[], start + CONNECTED_LOG_DELAY));
        assert!(!schedule.poll(&[], start + Duration::from_secs(30)));
        assert!(!schedule.poll(&[connected()], start + Duration::from_secs(31)));
        assert!(!schedule.poll(&[], start + Duration::from_secs(60)));
        assert_eq!(
            CONNECTED_LOG_MESSAGE,
            "First-Person Lab: connected successfully; delayed client logging test (3 seconds after connection)."
        );
    }

    #[test]
    fn duplicate_connected_event_does_not_restart_delay() {
        let mut schedule = ConnectionLogSchedule::default();
        let start = Instant::now();
        assert!(!schedule.poll(&[connected()], start));
        assert!(!schedule.poll(&[connected()], start + Duration::from_secs(2)));
        assert!(schedule.poll(&[], start + CONNECTED_LOG_DELAY));
    }

    #[test]
    fn disconnect_at_deadline_cancels_and_reconnect_gets_a_fresh_delay() {
        let mut schedule = ConnectionLogSchedule::default();
        let start = Instant::now();
        assert!(!schedule.poll(&[connected()], start));
        assert!(!schedule.poll(&[disconnected()], start + CONNECTED_LOG_DELAY));
        assert!(!schedule.poll(&[], start + Duration::from_secs(30)));
        let reconnect = start + Duration::from_secs(31);
        assert!(!schedule.poll(&[connected()], reconnect));
        assert!(!schedule.poll(&[], reconnect + Duration::from_secs(2)));
        assert!(schedule.poll(&[], reconnect + CONNECTED_LOG_DELAY));
        assert!(!schedule.poll(&[], reconnect + Duration::from_secs(30)));
    }

    #[test]
    fn immediate_disconnect_cancels_and_reconnect_after_logging_rearms_once() {
        let mut schedule = ConnectionLogSchedule::default();
        let start = Instant::now();
        assert!(!schedule.poll(&[connected(), disconnected()], start));
        assert!(!schedule.poll(&[], start + CONNECTED_LOG_DELAY));
        let reconnect = start + Duration::from_secs(4);
        assert!(!schedule.poll(&[connected()], reconnect));
        assert!(schedule.poll(&[], reconnect + CONNECTED_LOG_DELAY));
        let next = start + Duration::from_secs(8);
        assert!(!schedule.poll(&[disconnected(), connected()], next));
        assert!(!schedule.poll(&[], next + Duration::from_secs(2)));
        assert!(schedule.poll(&[], next + CONNECTED_LOG_DELAY));
        assert!(!schedule.poll(&[], next + Duration::from_secs(30)));
    }
}
