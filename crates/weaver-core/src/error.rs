//! Core error types.

use thiserror::Error;

/// Errors that can occur in core runtime operations.
#[derive(Debug, Error)]
pub enum WeaverError {
    /// The runtime is not in the expected phase for the requested operation.
    #[error("invalid runtime phase: {0}")]
    InvalidPhase(String),
    /// A command or event could not be processed.
    #[error("command rejected: {0}")]
    CommandRejected(String),
    /// An entity reference was invalid.
    #[error("entity not found: {0}")]
    EntityNotFound(crate::EntityId),
    /// A generic I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
