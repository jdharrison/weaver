//! Opt-in camera-motion post-processing settings.

use crate::RenderError;

/// Depth-reprojected camera-motion blur, applied before foreground meshes and UI.
///
/// This estimates camera motion only, not independently moving object velocity.
/// It does not perform temporal anti-aliasing or correct uneven frame pacing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraMotionBlur {
    /// Shutter duration in seconds (0..=1/30). Zero keeps the image sharp.
    /// A fixed duration makes blur approximately independent of presentation rate.
    pub exposure_seconds: f32,
    /// Maximum blur length in physical render-target pixels (0..=64).
    pub max_blur_pixels: f32,
    /// Actual elapsed presentation time in seconds, finite and strictly positive.
    /// Do not supply fixed simulation time or a clamped simulation delta.
    /// The backend suppresses blur across stalls longer than 100 ms.
    pub frame_delta_seconds: f32,
    /// Suppress blur for this frame and establish a new camera baseline on present.
    /// Set on teleports, respawns, reconnects, and focus/capture transitions.
    pub reset_history: bool,
}

impl Default for CameraMotionBlur {
    fn default() -> Self {
        Self {
            exposure_seconds: 1.0 / 240.0,
            max_blur_pixels: 12.0,
            frame_delta_seconds: 1.0 / 60.0,
            reset_history: false,
        }
    }
}

impl CameraMotionBlur {
    /// Check finite, bounded settings before GPU resource creation.
    ///
    /// # Errors
    /// Returns an error for invalid shutter duration, pixel cap, or frame time.
    pub fn validate(&self) -> Result<(), RenderError> {
        if !self.exposure_seconds.is_finite()
            || !(0.0..=1.0 / 30.0).contains(&self.exposure_seconds)
            || !self.max_blur_pixels.is_finite()
            || !(0.0..=64.0).contains(&self.max_blur_pixels)
            || !self.frame_delta_seconds.is_finite()
            || self.frame_delta_seconds <= 0.0
        {
            return Err(RenderError::Internal(
                "camera motion blur requires exposure in 0..=1/30 seconds, a pixel cap in 0..=64, and finite positive frame time".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_disabled_strength_are_valid() {
        CameraMotionBlur::default().validate().unwrap();
        CameraMotionBlur {
            exposure_seconds: 0.0,
            max_blur_pixels: 0.0,
            ..CameraMotionBlur::default()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn rejects_nonfinite_and_out_of_range_settings() {
        for value in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(
                CameraMotionBlur {
                    exposure_seconds: value,
                    ..CameraMotionBlur::default()
                }
                .validate()
                .is_err()
            );
            assert!(
                CameraMotionBlur {
                    max_blur_pixels: value,
                    ..CameraMotionBlur::default()
                }
                .validate()
                .is_err()
            );
            assert!(
                CameraMotionBlur {
                    frame_delta_seconds: value,
                    ..CameraMotionBlur::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            CameraMotionBlur {
                exposure_seconds: 1.0,
                ..CameraMotionBlur::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            CameraMotionBlur {
                max_blur_pixels: 65.0,
                ..CameraMotionBlur::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            CameraMotionBlur {
                frame_delta_seconds: 0.0,
                ..CameraMotionBlur::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn snapshot_opt_in_validates_settings() {
        let mut scene = crate::SceneSnapshot::default();
        assert!(scene.motion_blur.is_none());
        assert!(scene.foreground_meshes.is_empty());
        scene.motion_blur = Some(CameraMotionBlur {
            frame_delta_seconds: 0.0,
            ..CameraMotionBlur::default()
        });
        assert!(scene.validate().is_err());
    }
}
