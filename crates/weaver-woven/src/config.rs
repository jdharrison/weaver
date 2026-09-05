//! Woven adapter configuration.

use crate::mode::ConnectivityMode;

/// Configuration for a Woven connection.
#[derive(Clone, Debug, PartialEq)]
pub struct WovenConfig {
    /// Connectivity mode.
    pub mode: ConnectivityMode,
    /// Explicit loopback QUIC URL, required by [`ConnectivityMode::Loopback`].
    pub endpoint: Option<String>,
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

impl Default for WovenConfig {
    fn default() -> Self {
        Self {
            mode: ConnectivityMode::EmbeddedLocalNode,
            endpoint: None,
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
