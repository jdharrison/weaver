//! Woven adapter configuration.

use crate::mode::ConnectivityMode;
use std::path::PathBuf;

/// Configuration for a Woven connection.
#[derive(Clone, PartialEq)]
pub struct WovenConfig {
    /// Connectivity mode.
    pub mode: ConnectivityMode,
    /// Explicit QUIC URL, required by loopback and remote modes. Never put credentials here.
    pub endpoint: Option<String>,
    /// CA PEM bundle path, required in remote mode (at most 1 MiB).
    pub ca_pem_file: Option<PathBuf>,
    /// Static credential file path, required in remote mode; no development fallback.
    pub token_file: Option<PathBuf>,
    /// Optional monotonic deadline for remote traffic operations (not a hard process kill).
    /// Stop/Drop cleanup has its own two-second transport-close budget after this deadline.
    pub run_deadline: Option<std::time::Instant>,
    /// Namespace identifier.
    pub namespace_id: u64,
    /// Session identifier.
    pub session_id: u64,
    /// Space identifier.
    pub space_id: u64,
    /// Space epoch.
    pub space_epoch: u64,
    /// Development authentication token used by the embedded local node.
    pub dev_token: String,
    /// Maximum Woven protocol frame size advertised by the client.
    pub max_frame_bytes: u32,
    /// Maximum Woven protocol payload size advertised by the client.
    pub max_payload_bytes: u32,
}

impl std::fmt::Debug for WovenConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Omit URLs and paths too: malformed configuration may contain credentials.
        f.debug_struct("WovenConfig")
            .field("mode", &self.mode)
            .field("namespace_id", &self.namespace_id)
            .field("session_id", &self.session_id)
            .field("space_id", &self.space_id)
            .field("space_epoch", &self.space_epoch)
            .field("dev_token", &"[REDACTED]")
            .field("max_frame_bytes", &self.max_frame_bytes)
            .field("max_payload_bytes", &self.max_payload_bytes)
            .finish_non_exhaustive()
    }
}

impl WovenConfig {
    /// Validate explicit endpoints and mode-specific file requirements without connecting.
    pub fn validate(&self) -> Result<(), crate::WovenAdapterError> {
        use crate::WovenAdapterError::InitializationFailed;
        if self.mode != ConnectivityMode::EmbeddedLocalNode {
            let endpoint = self.endpoint.as_deref().ok_or_else(|| {
                InitializationFailed("explicit mode requires a QUIC endpoint".to_owned())
            })?;
            if self.mode == ConnectivityMode::RemoteQuic {
                validate_endpoint(endpoint)?;
            }
        }
        if self.mode == ConnectivityMode::RemoteQuic {
            if self
                .ca_pem_file
                .as_ref()
                .is_none_or(|path| path.as_os_str().is_empty())
                || self
                    .token_file
                    .as_ref()
                    .is_none_or(|path| path.as_os_str().is_empty())
            {
                return Err(InitializationFailed(
                    "remote QUIC requires CA PEM and token file paths".to_owned(),
                ));
            }
        } else if self.ca_pem_file.is_some() || self.token_file.is_some() {
            return Err(InitializationFailed(
                "remote credential paths require RemoteQuic mode".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_endpoint(endpoint: &str) -> Result<(), crate::WovenAdapterError> {
    let invalid = || {
        crate::WovenAdapterError::InitializationFailed(
            "expected quic://host:port without credentials, path, query or fragment".to_owned(),
        )
    };
    let authority = endpoint.strip_prefix("quic://").ok_or_else(invalid)?;
    if authority.len() > 260 || authority.contains(['@', '/', '?', '#', '%']) {
        return Err(invalid());
    }
    let (host, port) = authority.rsplit_once(':').ok_or_else(invalid)?;
    if port.parse::<u16>().ok().is_none_or(|port| port == 0) {
        return Err(invalid());
    }
    let host = if host.starts_with('[') && host.ends_with(']') {
        &host[1..host.len() - 1]
    } else {
        host
    };
    let ip = host.parse::<std::net::IpAddr>();
    if ip.is_err()
        && !host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(invalid());
    }
    if host.contains(':') && !authority.starts_with('[') {
        return Err(invalid());
    }
    Ok(())
}

impl Default for WovenConfig {
    fn default() -> Self {
        Self {
            mode: ConnectivityMode::EmbeddedLocalNode,
            endpoint: None,
            ca_pem_file: None,
            token_file: None,
            run_deadline: None,
            namespace_id: 1,
            session_id: 1,
            space_id: 1,
            space_epoch: 1,
            dev_token: "dev-token".to_owned(),
            max_frame_bytes: 65_536,
            max_payload_bytes: 65_536,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote() -> WovenConfig {
        WovenConfig {
            mode: ConnectivityMode::RemoteQuic,
            endpoint: Some("quic://woven.example.test:8081".to_owned()),
            ca_pem_file: Some("ca.pem".into()),
            token_file: Some("token".into()),
            ..WovenConfig::default()
        }
    }

    #[test]
    fn remote_requires_explicit_settings_and_quic() {
        assert!(WovenConfig::default().validate().is_ok());
        assert!(remote().validate().is_ok());
        for endpoint in ["quic://127.0.0.1:8081", "quic://[::1]:8081"] {
            assert!(
                WovenConfig {
                    endpoint: Some(endpoint.into()),
                    ..remote()
                }
                .validate()
                .is_ok()
            );
        }
        for endpoint in [
            "",
            "quic://host",
            "https://host:8081",
            "quic://token@host:8081",
            "quic://host:0",
            "quic://host:8081/path",
            "quic://host:8081?token=secret",
            "quic://host:8081#secret",
            "quic://::1:8081",
        ] {
            let config = WovenConfig {
                endpoint: Some(endpoint.into()),
                ..remote()
            };
            assert!(config.validate().is_err(), "{endpoint}");
        }
        assert!(
            WovenConfig {
                endpoint: None,
                ..remote()
            }
            .validate()
            .is_err()
        );
        assert!(
            WovenConfig {
                ca_pem_file: None,
                ..remote()
            }
            .validate()
            .is_err()
        );
        assert!(
            WovenConfig {
                token_file: None,
                ..remote()
            }
            .validate()
            .is_err()
        );
        assert!(
            WovenConfig {
                mode: ConnectivityMode::Loopback,
                ..remote()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn debug_omits_credentials_paths_and_endpoint() {
        let secret = "must-not-appear";
        let config = WovenConfig {
            dev_token: secret.into(),
            endpoint: Some(secret.into()),
            ca_pem_file: Some(secret.into()),
            token_file: Some(secret.into()),
            ..remote()
        };
        assert!(!format!("{config:?}").contains(secret));
        assert!(!config.validate().unwrap_err().to_string().contains(secret));
    }
}
