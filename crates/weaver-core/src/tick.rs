//! Simulation ticks and tick rate configuration.

use std::time::Duration;

/// A single simulation tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tick {
    /// Incrementing tick counter.
    pub number: u64,
    /// Elapsed simulation time since the previous tick.
    pub delta: Duration,
}

/// Desired simulation tick rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickRate {
    /// Ticks per second.
    pub ticks_per_second: u32,
}

impl TickRate {
    /// A common 60 Hz tick rate.
    pub const SIXTY: Self = Self::new(60);

    /// Create a tick rate.
    #[must_use]
    pub const fn new(ticks_per_second: u32) -> Self {
        Self { ticks_per_second }
    }

    /// Duration of one tick at this rate.
    #[must_use]
    pub const fn period(&self) -> Duration {
        Duration::from_nanos(1_000_000_000 / self.ticks_per_second as u64)
    }
}

impl Default for TickRate {
    fn default() -> Self {
        Self::SIXTY
    }
}
