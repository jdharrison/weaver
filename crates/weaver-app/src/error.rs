//! Application-level errors.

use thiserror::Error;

/// Errors from the application runner.
#[derive(Debug, Error)]
pub enum AppError {
    /// The renderer failed.
    #[error("render error: {0}")]
    Render(String),
    /// The world simulation failed.
    #[error("world error: {0}")]
    World(String),
    /// The Signalweave adapter failed.
    #[error("signalweave error: {0}")]
    Signalweave(String),
    /// Window or event loop failure.
    #[error("window error: {0}")]
    Window(String),
    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<weaver_render::RenderError> for AppError {
    fn from(err: weaver_render::RenderError) -> Self {
        Self::Render(err.to_string())
    }
}

impl From<weaver_worldline::WorldlineError> for AppError {
    fn from(err: weaver_worldline::WorldlineError) -> Self {
        Self::World(err.to_string())
    }
}

impl From<weaver_signalweave::SignalweaveAdapterError> for AppError {
    fn from(err: weaver_signalweave::SignalweaveAdapterError) -> Self {
        Self::Signalweave(err.to_string())
    }
}
