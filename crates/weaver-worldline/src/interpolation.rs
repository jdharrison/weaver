//! Transform interpolation for presentation-time smoothing.

use crate::history::{TrajectoryHistory, TrajectorySample};
use glam::{Quat, Vec3};

/// Linear interpolation between two `Vec3` values.
#[must_use]
pub fn lerp_vec3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

/// Spherical linear interpolation between two quaternions.
#[must_use]
pub fn slerp_quat(a: Quat, b: Quat, t: f32) -> Quat {
    a.slerp(b, t)
}

/// Interpolate a transform at `time_seconds` from a bounded history.
///
/// Returns `None` if there are not enough samples or if the requested time
/// falls outside the recorded range.
#[must_use]
pub fn interpolate_transform(
    history: &TrajectoryHistory,
    time_seconds: f64,
) -> Option<TrajectorySample> {
    let (a_idx, b_idx) = history.surrounding(time_seconds)?;
    let a = history.samples()[a_idx];
    let b = history.samples()[b_idx];

    if a.time_seconds == b.time_seconds {
        return Some(a);
    }

    let t = ((time_seconds - a.time_seconds) / (b.time_seconds - a.time_seconds)) as f32;
    Some(TrajectorySample {
        time_seconds,
        frame: a.frame,
        translation: lerp_vec3(a.translation, b.translation, t),
        rotation: slerp_quat(a.rotation, b.rotation, t),
        scale: lerp(a.scale, b.scale, t),
    })
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameId;

    fn sample(time: f64, x: f32) -> TrajectorySample {
        TrajectorySample {
            time_seconds: time,
            frame: FrameId::ROOT,
            translation: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: 1.0,
        }
    }

    #[test]
    fn interpolate_midpoint() {
        let mut history = TrajectoryHistory::new(8);
        history.push(sample(0.0, 0.0));
        history.push(sample(1.0, 10.0));
        let result = interpolate_transform(&history, 0.5).unwrap();
        assert!((result.translation.x - 5.0).abs() < 0.001);
    }

    #[test]
    fn interpolate_outside_range() {
        let mut history = TrajectoryHistory::new(8);
        history.push(sample(0.0, 0.0));
        assert!(interpolate_transform(&history, 0.5).is_none());
    }
}
