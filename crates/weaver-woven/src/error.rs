//! Errors from the Woven adapter.

use thiserror::Error;

/// Errors returned by the Woven adapter.
#[derive(Debug, Error)]
pub enum WovenAdapterError {
    /// The requested connectivity mode is not yet implemented.
    #[error("connectivity mode not supported: {0}")]
    UnsupportedMode(String),
    /// A local Woven node or client could not be initialized.
    #[error("Woven initialization failed: {0}")]
    InitializationFailed(String),
    /// The Woven client rejected a command or transport operation.
    #[error("Woven client operation failed: {0}")]
    ClientFailed(String),
    /// The local node returned an unexpected protocol message.
    #[error("unexpected Woven protocol message: {0}")]
    UnexpectedMessage(String),
    /// The requested delivery/persistence combination is not supported by the configured channel.
    #[error("channel policy is not supported by the embedded Woven development node")]
    UnsupportedChannelPolicy,
    /// Serialization failed.
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// A stale replicated payload was rejected locally.
    #[error("stale payload rejected")]
    StalePayload,
    /// The adapter is not running.
    #[error("adapter not running")]
    NotRunning,
}
