//! Weaver-facing adapter for Worldline (currently the `simengine` crate).
//!
//! This crate exposes time-aware simulation concepts without leaking
//! Worldline's internal clock types throughout Weaver.

#![warn(missing_docs)]

pub mod clock;
pub mod error;
pub mod frame;
pub mod history;
pub mod interpolation;
pub mod runtime;

pub use clock::{SimulationClock, SimulationClockConfig};
pub use error::WorldlineError;
pub use frame::{FrameId, FrameRegistry};
pub use history::{TrajectoryHistory, TrajectorySample};
pub use interpolation::interpolate_transform;
pub use runtime::{SimulationControl, SimulationRuntime};
