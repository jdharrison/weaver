//! Text runs for screen- and world-space labels.

use glam::Vec2;

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAnchor {
    /// Top-left corner.
    TopLeft,
    /// Top-center.
    TopCenter,
    /// Top-right.
    TopRight,
    /// Center-left.
    CenterLeft,
    /// Center.
    Center,
    /// Center-right.
    CenterRight,
    /// Bottom-left.
    BottomLeft,
    /// Bottom-center.
    BottomCenter,
    /// Bottom-right.
    BottomRight,
}

impl TextAnchor {
    /// Normalized offset to apply based on the anchor.
    #[must_use]
    pub const fn offset(&self) -> Vec2 {
        match self {
            Self::TopLeft => Vec2::new(0.0, 0.0),
            Self::TopCenter => Vec2::new(0.5, 0.0),
            Self::TopRight => Vec2::new(1.0, 0.0),
            Self::CenterLeft => Vec2::new(0.0, 0.5),
            Self::Center => Vec2::new(0.5, 0.5),
            Self::CenterRight => Vec2::new(1.0, 0.5),
            Self::BottomLeft => Vec2::new(0.0, 1.0),
            Self::BottomCenter => Vec2::new(0.5, 1.0),
            Self::BottomRight => Vec2::new(1.0, 1.0),
        }
    }
}

/// A text run to be rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// Text content.
    pub text: String,
    /// Screen position in pixels for screen-space text, or world position for
    /// world-space labels.
    pub position: Vec2,
    /// Font size in pixels.
    pub size: f32,
    /// Text color.
    pub color: [f32; 4],
    /// Anchor point.
    pub anchor: TextAnchor,
    /// Optional layout bounds.
    pub bounds: Option<Vec2>,
    /// Whether this label is in world space.
    pub world_space: bool,
    /// World-space height of the label when `world_space` is true.
    pub world_height: f32,
}

impl Default for TextRun {
    fn default() -> Self {
        Self {
            text: String::new(),
            position: Vec2::ZERO,
            size: 16.0,
            color: [1.0, 1.0, 1.0, 1.0],
            anchor: TextAnchor::TopLeft,
            bounds: None,
            world_space: false,
            world_height: 1.0,
        }
    }
}
