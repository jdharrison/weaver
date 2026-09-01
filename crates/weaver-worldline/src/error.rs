//! Errors from the Worldline adapter.

use thiserror::Error;

/// Errors returned by the Worldline adapter.
#[derive(Debug, Error)]
pub enum WorldlineError {
    /// The clock is in an invalid state for the requested operation.
    #[error("clock state error: {0}")]
    ClockState(String),
    /// A frame relationship is invalid.
    #[error("frame error: {0}")]
    Frame(String),
    /// The requested interpolation sample is missing.
    #[error("interpolation failed: {0}")]
    Interpolation(String),
    /// A wrapped error from the underlying engine.
    #[error("simengine error: {0}")]
    Engine(String),
}
