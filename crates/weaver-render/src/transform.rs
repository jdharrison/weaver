//! Transform representation independent of any renderer.

use glam::{Mat4, Quat, Vec3};

/// A frame-addressed transform for a renderable object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderTransform {
    /// Translation.
    pub translation: Vec3,
    /// Rotation.
    pub rotation: Quat,
    /// Uniform scale.
    pub scale: f32,
}

impl Default for RenderTransform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: 1.0,
        }
    }
}

impl RenderTransform {
    /// Identity transform.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: 1.0,
    };

    /// Build a model matrix from this transform.
    #[must_use]
    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.translation)
            * Mat4::from_quat(self.rotation)
            * Mat4::from_scale(Vec3::splat(self.scale))
    }
}
