//! Mesh instances and handles.

use crate::{RenderResourceId, RenderTransform};

/// Handle to a mesh resource owned by the renderer backend.
pub type MeshHandle = RenderResourceId;

/// One instanced draw of a mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshInstance {
    /// Mesh resource.
    pub mesh: MeshHandle,
    /// Local-to-world transform.
    pub transform: RenderTransform,
    /// Optional override color.
    pub color: [f32; 4],
    /// Emissive intensity. Values above 0.0 add unlit glow to the surface.
    pub emissive: f32,
}

impl MeshInstance {
    /// Create a mesh instance with default color and no emission.
    #[must_use]
    pub const fn new(mesh: MeshHandle, transform: RenderTransform) -> Self {
        Self {
            mesh,
            transform,
            color: [1.0, 1.0, 1.0, 1.0],
            emissive: 0.0,
        }
    }
}
