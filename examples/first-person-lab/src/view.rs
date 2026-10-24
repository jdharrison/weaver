//! Local camera presentation, independent of the player's transported eye pose.

use weaver_app_core::InputFrame;
use weaver_render::Camera;

use crate::theater::{self, Collider};

pub(crate) const DEFAULT_DISTANCE: f32 = 2.0;
pub(crate) const MAX_DISTANCE: f32 = 6.0;
pub(crate) const ZOOM_STEP: f32 = 0.5;
const ZOOM_RESPONSE: f32 = 12.0;
pub(crate) const CAMERA_CLEARANCE: f32 = 0.2;

#[derive(Clone)]
pub(crate) struct ViewRig {
    pub(crate) player: Camera,
    distance: f32,
    target_distance: f32,
    remembered_distance: f32,
}

impl ViewRig {
    pub(crate) fn new(player: Camera) -> Self {
        Self {
            player,
            distance: DEFAULT_DISTANCE,
            target_distance: DEFAULT_DISTANCE,
            remembered_distance: DEFAULT_DISTANCE,
        }
    }

    #[cfg(test)]
    pub(crate) fn preferences(&self) -> (f32, f32, f32) {
        (
            self.distance,
            self.target_distance,
            self.remembered_distance,
        )
    }

    pub(crate) fn force_first_person(&mut self) {
        self.distance = 0.0;
        self.target_distance = 0.0;
    }

    pub(crate) fn third_person(&self) -> bool {
        self.target_distance > 0.0
    }

    pub(crate) fn apply_input(&mut self, input: &InputFrame) {
        if input.toggle_perspective {
            if self.third_person() {
                self.remembered_distance = self.target_distance;
                self.target_distance = 0.0;
            } else {
                self.target_distance = self.remembered_distance;
            }
        }
        if input.zoom_delta.is_finite() {
            self.target_distance =
                (self.target_distance - input.zoom_delta * ZOOM_STEP).clamp(0.0, MAX_DISTANCE);
            if self.third_person() {
                self.remembered_distance = self.target_distance;
            }
        }
        // Zero explicitly selects first person, with no residual camera offset.
        if !self.third_person() {
            self.distance = 0.0;
        }
    }

    pub(crate) fn camera(&mut self, delta_seconds: f32, colliders: &[Collider]) -> Camera {
        let blend = 1.0 - (-ZOOM_RESPONSE * delta_seconds.max(0.0)).exp();
        self.distance += (self.target_distance - self.distance) * blend;
        let mut camera = self.player.clone();
        if self.third_person() {
            let forward = (self.player.target - self.player.eye).normalize_or_zero();
            let desired = self.player.eye - forward * self.distance;
            camera.eye =
                theater::clip_camera(self.player.eye, desired, CAMERA_CLEARANCE, colliders);
            // Keep the player's aim direction, not the camera's position, authoritative.
            camera.target = self.player.target;
        }
        camera
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_two_meters_with_consistent_zoom_and_clips_without_a_frame_step() {
        let mut rig = ViewRig::new(Camera {
            eye: theater::SPAWN,
            target: theater::SPAWN + glam::Vec3::NEG_Z,
            ..Camera::default()
        });
        assert!(rig.third_person());
        assert_eq!(DEFAULT_DISTANCE, 2.0);
        assert_eq!(rig.distance, DEFAULT_DISTANCE);
        assert_eq!(rig.target_distance, DEFAULT_DISTANCE);
        assert_eq!(rig.remembered_distance, DEFAULT_DISTANCE);
        let colliders = theater::Theater::new().colliders;
        let camera = rig.camera(0.0, &colliders);
        assert!((camera.eye.distance(rig.player.eye) - 1.8).abs() < 0.0001);
        assert_eq!(rig.player.eye, theater::SPAWN);
        assert_eq!(rig.distance, DEFAULT_DISTANCE);
        assert_eq!(rig.target_distance, DEFAULT_DISTANCE);
        assert_eq!(rig.remembered_distance, DEFAULT_DISTANCE);
        rig.player.eye.z -= 3.0;
        rig.player.target.z -= 3.0;
        assert_eq!(
            rig.camera(0.0, &colliders).eye.distance(rig.player.eye),
            DEFAULT_DISTANCE
        );
    }

    #[test]
    fn seated_override_and_restore_keep_all_zoom_preferences() {
        let mut rig = ViewRig::new(Camera::default());
        rig.apply_input(&InputFrame {
            zoom_delta: -3.0,
            ..InputFrame::default()
        });
        rig.camera(0.05, &[]);
        let saved = rig.clone();
        rig.force_first_person();
        assert_eq!(rig.camera(0.0, &[]).eye, rig.player.eye);
        assert!(!rig.third_person());
        rig = saved.clone();
        assert_eq!(rig.distance, saved.distance);
        assert_eq!(rig.target_distance, saved.target_distance);
        assert_eq!(rig.remembered_distance, saved.remembered_distance);
        rig.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        let saved_first = rig.clone();
        rig.force_first_person();
        rig = saved_first;
        rig.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        assert_eq!(rig.target_distance, saved.target_distance);
    }

    #[test]
    fn zoom_is_bounded_smooth_and_toggle_remembers_distance() {
        let mut rig = ViewRig::new(Camera::default());
        assert!(rig.third_person());
        rig.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        assert!(!rig.third_person());
        assert_eq!(rig.camera(0.01, &[]).eye, rig.player.eye);
        rig.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        assert_eq!(rig.target_distance, DEFAULT_DISTANCE);
        let camera = rig.camera(0.01, &[]);
        assert!(camera.eye.distance(rig.player.eye) > 0.0);
        assert!(camera.eye.distance(rig.player.eye) < DEFAULT_DISTANCE);
        rig.apply_input(&InputFrame {
            zoom_delta: -100.0,
            ..InputFrame::default()
        });
        assert_eq!(rig.target_distance, MAX_DISTANCE);
        rig.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        assert!(!rig.third_person());
        assert_eq!(rig.camera(0.1, &[]).eye, rig.player.eye);
        rig.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        assert_eq!(rig.target_distance, MAX_DISTANCE);
        rig.apply_input(&InputFrame {
            zoom_delta: 100.0,
            ..InputFrame::default()
        });
        assert!(!rig.third_person());
        rig.apply_input(&InputFrame {
            zoom_delta: f32::NAN,
            ..InputFrame::default()
        });
        assert_eq!(rig.target_distance, 0.0);
        rig.apply_input(&InputFrame {
            zoom_delta: -1.0,
            ..InputFrame::default()
        });
        assert_eq!(rig.target_distance, ZOOM_STEP);
    }
}
