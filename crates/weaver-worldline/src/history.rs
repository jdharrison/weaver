//! Bounded trajectory history for transform interpolation.

use crate::frame::FrameId;
use glam::{Quat, Vec3};

/// A timestamped transform sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrajectorySample {
    /// Simulation time at which the sample was recorded (seconds).
    pub time_seconds: f64,
    /// Coordinate frame.
    pub frame: FrameId,
    /// Translation.
    pub translation: Vec3,
    /// Rotation.
    pub rotation: Quat,
    /// Uniform scale.
    pub scale: f32,
}

/// A short, bounded history of transform samples.
#[derive(Clone, Debug, PartialEq)]
pub struct TrajectoryHistory {
    samples: Vec<TrajectorySample>,
    capacity: usize,
}

impl Default for TrajectoryHistory {
    fn default() -> Self {
        Self::new(64)
    }
}

impl TrajectoryHistory {
    /// Create a history with the given capacity.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::new(),
            capacity,
        }
    }

    /// Record a new sample, dropping the oldest if over capacity.
    pub fn push(&mut self, sample: TrajectorySample) {
        if self.samples.len() == self.capacity {
            self.samples.remove(0);
        }
        self.samples.push(sample);
    }

    /// Number of stored samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Returns `true` if no samples are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Borrow all samples in chronological order.
    #[must_use]
    pub fn samples(&self) -> &[TrajectorySample] {
        &self.samples
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Find the two surrounding samples for interpolation.
    #[must_use]
    pub fn surrounding(&self, time_seconds: f64) -> Option<(usize, usize)> {
        if self.samples.len() < 2 {
            return None;
        }
        for i in 0..self.samples.len() - 1 {
            let a = self.samples[i].time_seconds;
            let b = self.samples[i + 1].time_seconds;
            if a <= time_seconds && time_seconds <= b {
                return Some((i, i + 1));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_respects_capacity() {
        let mut history = TrajectoryHistory::new(4);
        for i in 0..10 {
            history.push(TrajectorySample {
                time_seconds: f64::from(i),
                frame: FrameId::ROOT,
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: 1.0,
            });
        }
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn surrounding_samples() {
        let mut history = TrajectoryHistory::new(8);
        for i in 0..5 {
            history.push(TrajectorySample {
                time_seconds: f64::from(i),
                frame: FrameId::ROOT,
                translation: Vec3::new(f32::from(i as i16), 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: 1.0,
            });
        }
        let (a, b) = history.surrounding(2.5).unwrap();
        assert_eq!(history.samples()[a].time_seconds, 2.0);
        assert_eq!(history.samples()[b].time_seconds, 3.0);
    }
}
