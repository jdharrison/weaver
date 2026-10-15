//! Mesh instances and handles.

use crate::{RenderResourceId, RenderTransform, SpriteHandle};

/// Handle to a mesh resource owned by the renderer backend.
pub type MeshHandle = RenderResourceId;

/// Independent point lighting for a mesh instance, without distance attenuation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshLighting {
    /// World-space point light position.
    pub position: glam::Vec3,
    /// Ambient light multiplier.
    pub ambient: f32,
    /// Diffuse light multiplier.
    pub intensity: f32,
}

/// Centered contain fitting for a texture on a mesh's unit `[0, 1]` UV surface.
///
/// Both aspect ratios are width divided by height. The whole image is fitted
/// without changing geometry, cropping, or stretching; unused UV space is opaque
/// black, independent of color, lighting, and emission. UV orientation is unchanged.
/// Non-finite or non-positive ratios disable fitting in the renderer backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshTextureFit {
    /// Aspect ratio of the surface represented by the mesh's unit UV rectangle.
    pub surface_aspect: f32,
    /// Native aspect ratio of the texture content.
    pub content_aspect: f32,
}

/// Framebuffer blending and depth policy for a mesh instance.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MeshRenderMode {
    /// Ordinary alpha-blended, back-face-culled surface with depth writes.
    #[default]
    Surface,
    /// Two-sided, alpha-weighted additive RGB with destination alpha preserved.
    /// Tests against scene depth without writing it; drawn after surfaces and
    /// before sprites. Lighting and emission still use the ordinary mesh shader.
    Additive,
}

/// One instanced draw of a mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshInstance {
    /// Mesh resource.
    pub mesh: MeshHandle,
    /// CPU-side pipeline selection, independent of emissive fragment lighting.
    pub render_mode: MeshRenderMode,
    /// Optional color texture. `None` uses solid white.
    pub texture: Option<SpriteHandle>,
    /// Optional centered contain fit with opaque black bars. `None` preserves
    /// repeating UV sampling; invalid aspect ratios also fall back to that path.
    pub texture_fit: Option<MeshTextureFit>,
    /// Local-to-world transform.
    pub transform: RenderTransform,
    /// Optional override color.
    pub color: [f32; 4],
    /// Emissive intensity. Values above 0.0 add unlit glow to the surface.
    pub emissive: f32,
    /// Optional point lighting. `None` preserves origin lighting with ambient 0.08
    /// and diffuse intensity 1.0.
    pub lighting: Option<MeshLighting>,
}

impl MeshInstance {
    /// Create an untextured mesh instance with default color and no emission.
    #[must_use]
    pub const fn new(mesh: MeshHandle, transform: RenderTransform) -> Self {
        Self {
            mesh,
            render_mode: MeshRenderMode::Surface,
            texture: None,
            texture_fit: None,
            transform,
            color: [1.0, 1.0, 1.0, 1.0],
            emissive: 0.0,
            lighting: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_mesh_is_untextured_white_and_non_emissive() {
        let mesh = MeshHandle::new();
        let transform = RenderTransform::default();
        let mut instance = MeshInstance::new(mesh, transform);
        assert_eq!(instance.mesh, mesh);
        assert_eq!(MeshRenderMode::default(), MeshRenderMode::Surface);
        assert_eq!(instance.render_mode, MeshRenderMode::Surface);
        assert_eq!(instance.transform, transform);
        assert_eq!(instance.texture, None);
        assert_eq!(instance.texture_fit, None);
        assert_eq!(instance.color, [1.0; 4]);
        assert_eq!(instance.emissive, 0.0);
        assert_eq!(instance.lighting, None);

        let texture = SpriteHandle::new();
        instance.texture = Some(texture);
        assert_eq!(instance.texture, Some(texture));
        assert_eq!(instance.texture_fit, None);
    }

    #[test]
    fn texture_fit_is_optional_instance_metadata_not_a_geometry_transform() {
        let transform = RenderTransform::default();
        let mut instance = MeshInstance::new(MeshHandle::new(), transform);
        let fit = MeshTextureFit {
            surface_aspect: 2.39,
            content_aspect: 16.0 / 9.0,
        };
        instance.texture_fit = Some(fit);
        assert_eq!(instance.texture_fit, Some(fit));
        assert_eq!(instance.clone(), instance);
        assert_eq!(instance.transform, transform);
        assert_eq!(instance.texture, None);
    }
}
