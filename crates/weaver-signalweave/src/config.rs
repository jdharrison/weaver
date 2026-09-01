//! Signalweave adapter configuration.

use crate::mode::ConnectivityMode;

/// Configuration for the Signalweave adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalweaveConfig {
    /// Connectivity mode.
    pub mode: ConnectivityMode,
    /// Maximum pending commands in the embedded worker harness.
    pub harness_capacity: usize,
    /// Namespace identifier to use in embedded mode.
    pub namespace_id: u64,
    /// Session identifier to use in embedded mode.
    pub session_id: u64,
    /// Space identifier to use in embedded mode.
    pub space_id: u64,
    /// Space epoch to use in embedded mode.
    pub space_epoch: u64,
    /// Development authentication token.
    pub dev_token: String,
}

impl Default for SignalweaveConfig {
    fn default() -> Self {
        Self {
            mode: ConnectivityMode::OfflineEmbedded,
            harness_capacity: 1024,
            namespace_id: 1,
            session_id: 1,
            space_id: 1,
            space_epoch: 1,
            dev_token: "weaver-dev".to_string(),
        }
    }
}
