//! High-level simulation runtime interface.

use crate::clock::{ClockState, SimulationClock, SimulationClockConfig};
use crate::error::WorldlineError;
use crate::frame::FrameRegistry;
use crate::history::{TrajectoryHistory, TrajectorySample};
use crate::interpolation::interpolate_transform;
use std::collections::HashMap;
use std::time::Duration;

/// Control operations for the simulation.
pub trait SimulationControl {
    /// Start or resume the simulation.
    fn start(&mut self);
    /// Pause the simulation.
    fn pause(&mut self);
    /// Resume the simulation.
    fn resume(&mut self);
    /// Stop and reset the simulation.
    fn reset(&mut self);
    /// Step one fixed tick deterministically.
    fn step(&mut self) -> Option<Duration>;
    /// Set the time multiplier.
    fn set_time_multiplier(&mut self, multiplier: f64);
    /// Current simulation time in seconds.
    fn time_seconds(&self) -> f64;
    /// Current clock state.
    fn state(&self) -> ClockState;
}

/// A simulation runtime that combines a clock, frame registry, and bounded
/// trajectory history.
#[derive(Debug)]
pub struct SimulationRuntime {
    clock: SimulationClock,
    frames: FrameRegistry,
    histories: HashMap<weaver_core::EntityId, TrajectoryHistory>,
    history_capacity: usize,
}

impl SimulationRuntime {
    /// Create a new runtime from a clock configuration.
    #[must_use]
    pub fn new(config: SimulationClockConfig) -> Self {
        Self {
            clock: SimulationClock::new(config),
            frames: FrameRegistry::new(),
            histories: HashMap::new(),
            history_capacity: 64,
        }
    }

    /// Access the frame registry.
    #[must_use]
    pub fn frames(&self) -> &FrameRegistry {
        &self.frames
    }

    /// Mutable access to the frame registry.
    #[must_use]
    pub fn frames_mut(&mut self) -> &mut FrameRegistry {
        &mut self.frames
    }

    /// Record a transform sample for an entity.
    pub fn record_sample(&mut self, entity: weaver_core::EntityId, sample: TrajectorySample) {
        self.histories
            .entry(entity)
            .or_insert_with(|| TrajectoryHistory::new(self.history_capacity))
            .push(sample);
    }

    /// Borrow an entity's trajectory history.
    #[must_use]
    pub fn history(&self, entity: weaver_core::EntityId) -> Option<&TrajectoryHistory> {
        self.histories.get(&entity)
    }

    /// Interpolate an entity's transform at a presentation time.
    ///
    /// # Errors
    ///
    /// Returns an error if there is no history or the time is out of range.
    pub fn interpolate(
        &self,
        entity: weaver_core::EntityId,
        time_seconds: f64,
    ) -> Result<TrajectorySample, WorldlineError> {
        let history = self
            .histories
            .get(&entity)
            .ok_or_else(|| WorldlineError::Interpolation(format!("no history for {entity}")))?;
        interpolate_transform(history, time_seconds)
            .ok_or_else(|| WorldlineError::Interpolation("time out of history range".to_string()))
    }

    /// Set the maximum number of samples retained per entity.
    pub fn set_history_capacity(&mut self, capacity: usize) {
        self.history_capacity = capacity;
        for history in self.histories.values_mut() {
            while history.len() > capacity {
                let mut samples = history.samples().to_vec();
                samples.remove(0);
                history.clear();
                for sample in samples {
                    history.push(sample);
                }
            }
        }
    }

    /// Advance the simulation by one fixed step and record a sample.
    ///
    /// # Errors
    ///
    /// Returns an error if the clock is not running.
    pub fn advance_and_record(
        &mut self,
        entity: weaver_core::EntityId,
        sample_fn: impl FnOnce() -> TrajectorySample,
    ) -> Result<Duration, WorldlineError> {
        let delta = self
            .clock
            .step()
            .ok_or_else(|| WorldlineError::ClockState("clock is not running".to_string()))?;
        self.record_sample(entity, sample_fn());
        Ok(delta)
    }
}

impl SimulationControl for SimulationRuntime {
    fn start(&mut self) {
        self.clock.start();
    }

    fn pause(&mut self) {
        self.clock.pause();
    }

    fn resume(&mut self) {
        self.clock.resume();
    }

    fn reset(&mut self) {
        self.clock.reset();
        self.histories.clear();
    }

    fn step(&mut self) -> Option<Duration> {
        self.clock.step()
    }

    fn set_time_multiplier(&mut self, multiplier: f64) {
        self.clock.set_time_multiplier(multiplier);
    }

    fn time_seconds(&self) -> f64 {
        self.clock.simulation_time_seconds()
    }

    fn state(&self) -> ClockState {
        self.clock.state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FrameId;
    use glam::{Quat, Vec3};

    #[test]
    fn runtime_lifecycle() {
        let mut runtime = SimulationRuntime::new(SimulationClockConfig::default());
        assert_eq!(runtime.state(), ClockState::Stopped);
        runtime.start();
        assert_eq!(runtime.state(), ClockState::Running);
        runtime.pause();
        assert_eq!(runtime.state(), ClockState::Paused);
        runtime.resume();
        runtime.step();
        assert!(runtime.time_seconds() > 0.0);
        runtime.reset();
        assert_eq!(runtime.time_seconds(), 0.0);
        assert_eq!(runtime.state(), ClockState::Stopped);
    }

    #[test]
    fn record_and_interpolate() {
        let mut runtime = SimulationRuntime::new(SimulationClockConfig::default());
        let entity = weaver_core::EntityId::new();
        runtime.start();
        runtime.record_sample(
            entity,
            TrajectorySample {
                time_seconds: 0.0,
                frame: FrameId::ROOT,
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: 1.0,
            },
        );
        runtime.record_sample(
            entity,
            TrajectorySample {
                time_seconds: 1.0,
                frame: FrameId::ROOT,
                translation: Vec3::X,
                rotation: Quat::IDENTITY,
                scale: 1.0,
            },
        );
        let interp = runtime.interpolate(entity, 0.5).unwrap();
        assert!((interp.translation.x - 0.5).abs() < 0.001);
    }
}
