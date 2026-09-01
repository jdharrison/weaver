//! Errors from the Signalweave adapter.

use thiserror::Error;

/// Errors returned by the Signalweave adapter.
#[derive(Debug, Error)]
pub enum SignalweaveAdapterError {
    /// The requested connectivity mode is not yet implemented.
    #[error("connectivity mode not supported: {0}")]
    UnsupportedMode(String),
    /// The embedded worker failed to initialize.
    #[error("embedded worker initialization failed: {0}")]
    InitializationFailed(String),
    /// A Signalweave command failed.
    #[error("signalweave command failed: {0}")]
    CommandFailed(String),
    /// Authentication failed.
    #[error("authentication failed")]
    AuthenticationFailed,
    /// Serialization failed.
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// A stale replicated payload was rejected.
    #[error("stale payload rejected")]
    StalePayload,
    /// The adapter is not running.
    #[error("adapter not running")]
    NotRunning,
}
