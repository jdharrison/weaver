//! Cosmetic locomotion only: never changes physical, camera, or network poses.

use glam::Vec3;

const MAX_SAMPLE_GAP: f64 = 0.25;
const MAX_WALK_SPEED: f32 = 12.0;
const MIN_WALK_SPEED: f32 = 0.08;
const STRIDE_METERS: f32 = 1.6;
pub(crate) const MAX_ROCK: f32 = 0.02;

#[derive(Clone, Debug, Default)]
pub(crate) struct WalkWobble {
    previous: Option<(Vec3, f64)>,
    phase: f32,
}

impl WalkWobble {
    /// Sample once per simulation frame, including when the avatar is hidden.
    /// Actual horizontal travel drives the gait; intent alone cannot animate walls.
    pub(crate) fn sample(&mut self, position: Vec3, now: f64, walking: bool) -> f32 {
        if !position.is_finite() || !now.is_finite() {
            *self = Self::default();
            return 0.0;
        }
        let previous = self.previous.replace((position, now));
        let Some((previous, last)) = previous else {
            return 0.0;
        };
        let dt = now - last;
        let distance = ((position - previous) * Vec3::new(1.0, 0.0, 1.0)).length();
        let speed = distance / dt as f32;
        if !walking
            || !(0.0..=MAX_SAMPLE_GAP).contains(&dt)
            || dt == 0.0
            || !(MIN_WALK_SPEED..=MAX_WALK_SPEED).contains(&speed)
        {
            self.phase = 0.0;
            return 0.0;
        }
        self.phase = (self.phase + distance * std::f32::consts::TAU / STRIDE_METERS)
            .rem_euclid(std::f32::consts::TAU);
        self.phase.sin() * MAX_ROCK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_rocks_slightly_but_idle_intent_and_vertical_motion_do_not() {
        let mut wobble = WalkWobble::default();
        assert_eq!(wobble.sample(Vec3::ZERO, 0.0, true), 0.0);
        let rock = wobble.sample(Vec3::X * 0.1, 0.025, true);
        assert!(rock > 0.0 && rock <= MAX_ROCK);
        assert_eq!(wobble.sample(Vec3::X * 0.1, 0.05, true), 0.0);
        assert_eq!(wobble.sample(Vec3::X * 0.1 + Vec3::Y, 0.075, true), 0.0);
        assert_eq!(wobble.sample(Vec3::X * 0.2, 0.1, false), 0.0);
    }

    #[test]
    fn phase_depends_on_distance_not_frame_rate() {
        let mut dense = WalkWobble::default();
        let mut sparse = WalkWobble::default();
        dense.sample(Vec3::ZERO, 0.0, true);
        sparse.sample(Vec3::ZERO, 0.0, true);
        dense.sample(Vec3::X * 0.1, 0.025, true);
        let a = dense.sample(Vec3::X * 0.2, 0.05, true);
        let b = sparse.sample(Vec3::X * 0.2, 0.05, true);
        assert!((a - b).abs() < 0.000_001);
        for step in 3..1000 {
            let rock = dense.sample(Vec3::X * (step as f32 * 0.1), f64::from(step) * 0.025, true);
            assert!(rock.abs() <= MAX_ROCK);
        }
    }

    #[test]
    fn spawn_corrections_gaps_and_bad_clocks_cannot_start_a_gait() {
        for (position, now) in [
            (Vec3::X * 5.0, 0.02),
            (Vec3::X * 0.1, 1.0),
            (Vec3::X * 0.1, 0.0),
            (Vec3::X * 0.1, -0.1),
            (Vec3::X * 0.1, f64::NAN),
            (Vec3::splat(f32::NAN), 0.02),
        ] {
            let mut wobble = WalkWobble::default();
            wobble.sample(Vec3::ZERO, 0.0, true);
            assert_eq!(wobble.sample(position, now, true), 0.0);
        }
    }
}
