//! Sprite instances and handles.

use crate::{RenderResourceId, RenderTransform};
use glam::Vec2;

/// Handle to a texture / sprite resource owned by the renderer backend.
pub type SpriteHandle = RenderResourceId;

/// Whether a sprite is expressed in world or screen space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpriteSpace {
    /// World-space billboard or oriented quad.
    World,
    /// Screen-space overlay.
    Screen,
}

/// One instanced draw of a sprite.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteInstance {
    /// Sprite texture resource.
    pub sprite: SpriteHandle,
    /// Local transform.
    pub transform: RenderTransform,
    /// UV offset/scale for atlas support.
    pub uv_rect: [f32; 4],
    /// Tint color.
    pub tint: [f32; 4],
    /// Layer order for transparent sorting.
    pub layer: i32,
    /// World or screen space.
    pub space: SpriteSpace,
    /// Screen-space pivot in normalized [0,1] coordinates.
    pub pivot: Vec2,
}

impl Default for SpriteInstance {
    fn default() -> Self {
        Self {
            sprite: SpriteHandle::new(),
            transform: RenderTransform::default(),
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            layer: 0,
            space: SpriteSpace::World,
            pivot: Vec2::new(0.5, 0.5),
        }
    }
}
