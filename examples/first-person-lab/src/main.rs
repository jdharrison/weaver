//! Native desktop entry point for First-Person Lab.

#[cfg(not(target_arch = "wasm32"))]
use anyhow::Context;
#[cfg(not(target_arch = "wasm32"))]
use weaver_woven::{ConnectivityMode, WovenConfig, WovenRealtimeDriver};

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let app = Box::new(first_person_lab::FirstPersonLab::new());
    let platform = weaver_platform_desktop::DesktopConfig::default();
    if let Some(driver) = realtime_driver_from_env()? {
        weaver_platform_desktop::run_with_realtime(app, platform, Box::new(driver))?;
    } else {
        weaver_platform_desktop::run(app, platform)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn realtime_driver_from_env() -> anyhow::Result<Option<WovenRealtimeDriver>> {
    let Some(endpoint) = std::env::var("FIRST_PERSON_WOVEN_URL").ok() else {
        return Ok(None);
    };
    let target = std::env::var("FIRST_PERSON_WOVEN_TARGET").unwrap_or_else(|_| "local".to_owned());
    let mut config = WovenConfig {
        endpoint: Some(endpoint),
        namespace_id: optional_id("FIRST_PERSON_WOVEN_NAMESPACE_ID", 1)?,
        session_id: optional_id("FIRST_PERSON_WOVEN_SESSION_ID", 1)?,
        space_id: optional_id("FIRST_PERSON_WOVEN_SPACE_ID", 1)?,
        space_epoch: optional_id("FIRST_PERSON_WOVEN_SPACE_EPOCH", 1)?,
        ..WovenConfig::default()
    };
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
    Ok(Some(WovenRealtimeDriver::connect(config)?))
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
