//! Runtime lifecycle and configuration.

use crate::{Revision, WeaverError};
use std::time::Duration;

/// Configuration for a [`WeaverRuntime`].
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeConfig {
    /// Whether the runtime is allowed to run without a GPU renderer.
    pub headless_permitted: bool,
    /// Fixed timestep for simulation updates.
    pub fixed_timestep: Duration,
    /// Maximum number of commands queued per frame.
    pub max_command_queue_depth: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            headless_permitted: true,
            fixed_timestep: Duration::from_secs_f64(1.0 / 60.0),
            max_command_queue_depth: 4096,
        }
    }
}

/// Lifecycle phase of the runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePhase {
    /// The runtime has been constructed but not started.
    Constructed,
    /// The runtime is running simulation and render loops.
    Running,
    /// The runtime is paused but still accepting input.
    Paused,
    /// The runtime is shutting down.
    ShuttingDown,
    /// The runtime has stopped.
    Stopped,
}

/// Minimal runtime interface implemented by the application runner.
///
/// Concrete runtimes live in higher-level crates (`weaver-app`,
/// `weaver-worldline`, etc.). This trait defines the seam that the render
/// lab and tests can drive.
pub trait WeaverRuntime {
    /// Start the runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime cannot start (e.g. missing renderer
    /// when headless mode is disabled).
    fn start(&mut self) -> Result<(), WeaverError>;

    /// Pause the runtime.
    fn pause(&mut self);

    /// Resume the runtime.
    fn resume(&mut self);

    /// Step one fixed simulation tick. Useful for headless testing.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime is not in a steppable phase.
    fn step(&mut self) -> Result<(), WeaverError>;

    /// Shut the runtime down cleanly.
    fn shutdown(&mut self);

    /// Current lifecycle phase.
    fn phase(&self) -> RuntimePhase;

    /// Current world revision.
    fn revision(&self) -> Revision;
}
