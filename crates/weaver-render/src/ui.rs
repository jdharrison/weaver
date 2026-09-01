//! Minimal UI vocabulary.

use crate::RenderResourceId;
use crate::text::TextRun;
use glam::Vec2;

/// Handle to a texture used by a UI image.
pub type UiImageHandle = RenderResourceId;

/// A filled or bordered rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct UiRect {
    /// Position in screen pixels from the anchor.
    pub position: Vec2,
    /// Size in screen pixels.
    pub size: Vec2,
    /// Background color.
    pub background: [f32; 4],
    /// Border color.
    pub border_color: [f32; 4],
    /// Border thickness in pixels.
    pub border_width: f32,
    /// Corner radius in pixels.
    pub corner_radius: f32,
    /// Anchor point.
    pub anchor: super::text::TextAnchor,
    /// Layer for draw ordering.
    pub layer: i32,
    /// Whether this rect is interactive (clickable hit region).
    pub interactive: bool,
}

impl Default for UiRect {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            size: Vec2::new(100.0, 40.0),
            background: [0.2, 0.2, 0.25, 1.0],
            border_color: [0.5, 0.5, 0.6, 1.0],
            border_width: 1.0,
            corner_radius: 4.0,
            anchor: super::text::TextAnchor::TopLeft,
            layer: 0,
            interactive: false,
        }
    }
}

/// A UI image.
#[derive(Clone, Debug, PartialEq)]
pub struct UiImage {
    /// Texture resource.
    pub image: UiImageHandle,
    /// Position in screen pixels from the anchor.
    pub position: Vec2,
    /// Size in screen pixels.
    pub size: Vec2,
    /// UV rectangle for atlas support.
    pub uv_rect: [f32; 4],
    /// Tint color.
    pub tint: [f32; 4],
    /// Anchor point.
    pub anchor: super::text::TextAnchor,
    /// Layer.
    pub layer: i32,
}

impl Default for UiImage {
    fn default() -> Self {
        Self {
            image: UiImageHandle::new(),
            position: Vec2::ZERO,
            size: Vec2::new(64.0, 64.0),
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            anchor: super::text::TextAnchor::TopLeft,
            layer: 0,
        }
    }
}

/// A UI text element.
#[derive(Clone, Debug, PartialEq)]
pub struct UiText {
    /// Text run.
    pub run: TextRun,
    /// Anchor point.
    pub anchor: super::text::TextAnchor,
    /// Layer.
    pub layer: i32,
}

/// A clipping rectangle for UI composition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiClip {
    /// Screen-space origin.
    pub origin: Vec2,
    /// Size.
    pub size: Vec2,
}

/// A minimal UI element.
#[derive(Clone, Debug, PartialEq)]
pub enum UiElement {
    /// Filled/bordered panel.
    Rect(UiRect),
    /// Image.
    Image(UiImage),
    /// Text label.
    Text(UiText),
    /// Clipping group containing child elements.
    Clip {
        /// Clipping region.
        clip: UiClip,
        /// Children.
        children: Vec<UiElement>,
    },
}
