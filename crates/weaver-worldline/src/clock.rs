//! Simulation clock configuration and control.

use simengine::{FidelityLevel, SimulationConfig};
use std::time::Duration;

/// Configuration for the Weaver-facing simulation clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationClockConfig {
    /// Target fixed simulation steps per second.
    pub steps_per_second: u32,
    /// Multiplier applied to simulation time advancement.
    pub time_multiplier: f64,
    /// Fidelity preset.
    pub fidelity: FidelityLevel,
    /// Whether to run in real-time mode.
    pub real_time_mode: bool,
}

impl Default for SimulationClockConfig {
    fn default() -> Self {
        Self {
            steps_per_second: 60,
            time_multiplier: 1.0,
            fidelity: FidelityLevel::Medium,
            real_time_mode: true,
        }
    }
}

impl From<SimulationClockConfig> for SimulationConfig {
    fn from(config: SimulationClockConfig) -> Self {
        Self {
            target_steps_per_second: config.steps_per_second,
            simulation_time_multiplier: config.time_multiplier,
            fidelity: config.fidelity,
            real_time_mode: config.real_time_mode,
        }
    }
}

impl From<SimulationConfig> for SimulationClockConfig {
    fn from(config: SimulationConfig) -> Self {
        Self {
            steps_per_second: config.target_steps_per_second,
            time_multiplier: config.simulation_time_multiplier,
            fidelity: config.fidelity,
            real_time_mode: config.real_time_mode,
        }
    }
}

/// State of the simulation clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockState {
    /// The clock is stopped.
    Stopped,
    /// The clock is running.
    Running,
    /// The clock is paused.
    Paused,
    /// The clock encountered an error.
    Error,
}

/// A deterministic, fixed-step simulation clock backed by Worldline types.
///
/// The adapter keeps Worldline's `SimEngine` for lifecycle fidelity and
/// configuration, but tracks absolute simulation time itself so that headless
/// tests can step deterministically without wall-clock drift.
#[derive(Debug)]
pub struct SimulationClock {
    engine: simengine::SimEngine,
    state: ClockState,
    sim_time_ns: u64,
    total_steps: u64,
    time_multiplier: f64,
}

impl SimulationClock {
    /// Create a clock from a configuration.
    #[must_use]
    pub fn new(config: SimulationClockConfig) -> Self {
        Self {
            engine: simengine::SimEngine::new(config.into()),
            state: ClockState::Stopped,
            sim_time_ns: 0,
            total_steps: 0,
            time_multiplier: config.time_multiplier,
        }
    }

    /// Start the clock.
    pub fn start(&mut self) {
        self.engine.set_state(simengine::EngineState::Running);
        self.state = ClockState::Running;
    }

    /// Pause the clock.
    pub fn pause(&mut self) {
        self.engine.set_state(simengine::EngineState::Paused);
        self.state = ClockState::Paused;
    }

    /// Resume the clock.
    pub fn resume(&mut self) {
        self.engine.set_state(simengine::EngineState::Running);
        self.state = ClockState::Running;
    }

    /// Stop the clock.
    pub fn stop(&mut self) {
        self.engine.set_state(simengine::EngineState::Stopped);
        self.state = ClockState::Stopped;
    }

    /// Reset simulation time to zero and stop.
    pub fn reset(&mut self) {
        self.engine.set_state(simengine::EngineState::Stopped);
        self.state = ClockState::Stopped;
        self.sim_time_ns = 0;
        self.total_steps = 0;
    }

    /// Advance one fixed simulation step deterministically.
    ///
    /// Returns the elapsed simulation time for the step, or `None` if the
    /// clock is not running.
    pub fn step(&mut self) -> Option<Duration> {
        if self.state != ClockState::Running {
            return None;
        }
        let step_ns = self.timestep_ns();
        let advanced_ns = (step_ns as f64 * self.time_multiplier).round() as u64;
        self.sim_time_ns += advanced_ns;
        self.total_steps += 1;
        Some(Duration::from_nanos(advanced_ns))
    }

    /// Advance by exactly one fixed step regardless of real time or state.
    #[must_use]
    pub fn step_deterministic(&mut self) -> Duration {
        let step_ns = self.timestep_ns();
        self.sim_time_ns += step_ns;
        self.total_steps += 1;
        Duration::from_nanos(step_ns)
    }

    /// Current simulation time in nanoseconds.
    #[must_use]
    pub fn simulation_time_ns(&self) -> u64 {
        self.sim_time_ns
    }

    /// Current simulation time in seconds.
    #[must_use]
    pub fn simulation_time_seconds(&self) -> f64 {
        self.sim_time_ns as f64 / 1_000_000_000.0
    }

    /// Current clock state.
    #[must_use]
    pub fn state(&self) -> ClockState {
        self.state
    }

    /// Current configuration.
    #[must_use]
    pub fn config(&self) -> SimulationClockConfig {
        let mut cfg: SimulationClockConfig = (*self.engine.config()).into();
        cfg.time_multiplier = self.time_multiplier;
        cfg
    }

    /// Fixed timestep duration.
    #[must_use]
    pub fn timestep(&self) -> Duration {
        Duration::from_nanos(self.timestep_ns())
    }

    fn timestep_ns(&self) -> u64 {
        1_000_000_000 / u64::from(self.engine.config().target_steps_per_second)
    }

    /// Set the time multiplier.
    pub fn set_time_multiplier(&mut self, multiplier: f64) {
        self.time_multiplier = multiplier;
    }

    /// Total number of steps advanced.
    #[must_use]
    pub fn total_steps(&self) -> u64 {
        self.total_steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_start_pause_resume_stop() {
        let mut clock = SimulationClock::new(SimulationClockConfig::default());
        assert_eq!(clock.state(), ClockState::Stopped);
        clock.start();
        assert_eq!(clock.state(), ClockState::Running);
        clock.pause();
        assert_eq!(clock.state(), ClockState::Paused);
        clock.resume();
        assert_eq!(clock.state(), ClockState::Running);
        clock.stop();
        assert_eq!(clock.state(), ClockState::Stopped);
    }

    #[test]
    fn deterministic_timestep() {
        let clock = SimulationClock::new(SimulationClockConfig {
            steps_per_second: 60,
            ..Default::default()
        });
        assert_eq!(clock.timestep(), Duration::from_nanos(16_666_666));
    }

    #[test]
    fn deterministic_step_accumulates_time() {
        let mut clock = SimulationClock::new(SimulationClockConfig {
            steps_per_second: 60,
            ..Default::default()
        });
        clock.start();
        for _ in 0..60 {
            clock.step();
        }
        assert_eq!(clock.total_steps(), 60);
        assert!(clock.simulation_time_ns() > 999_000_000);
    }
}
