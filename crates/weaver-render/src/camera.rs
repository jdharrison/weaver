//! Camera and viewport definitions.

use glam::{Mat4, Vec3};

/// Projection mode for a camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Perspective projection with vertical field of view (radians).
    Perspective {
        /// Vertical field of view in radians.
        fov_y: f32,
        /// Near clipping plane.
        near: f32,
        /// Far clipping plane.
        far: f32,
    },
    /// Orthographic projection.
    Orthographic {
        /// Half-width of the view volume.
        half_width: f32,
        /// Half-height of the view volume.
        half_height: f32,
        /// Near clipping plane.
        near: f32,
        /// Far clipping plane.
        far: f32,
    },
}

impl Default for Projection {
    fn default() -> Self {
        Self::Perspective {
            fov_y: std::f32::consts::FRAC_PI_4,
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl Projection {
    /// Compute the projection matrix for the given aspect ratio.
    #[must_use]
    pub fn matrix(&self, aspect: f32) -> Mat4 {
        match *self {
            Self::Perspective { fov_y, near, far } => {
                Mat4::perspective_rh(fov_y, aspect, near, far)
            }
            Self::Orthographic {
                half_width,
                half_height,
                near,
                far,
            } => Mat4::orthographic_rh(
                -half_width,
                half_width,
                -half_height,
                half_height,
                near,
                far,
            ),
        }
    }
}

/// A camera placed in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// World-space eye position.
    pub eye: Vec3,
    /// World-space look-at target.
    pub target: Vec3,
    /// World-space up vector.
    pub up: Vec3,
    /// Projection.
    pub projection: Projection,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(0.0, 2.0, 5.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            projection: Projection::default(),
        }
    }
}

impl Camera {
    /// View matrix.
    #[must_use]
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye, self.target, self.up)
    }

    /// View-projection matrix.
    #[must_use]
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection.matrix(aspect) * self.view_matrix()
    }
}

/// A viewport within the swapchain target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// X offset in pixels.
    pub x: f32,
    /// Y offset in pixels.
    pub y: f32,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
    /// Minimum depth.
    pub min_depth: f32,
    /// Maximum depth.
    pub max_depth: f32,
}

impl Viewport {
    /// Create a viewport covering the full extent.
    #[must_use]
    pub const fn full(width: f32, height: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width,
            height,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }

    /// Aspect ratio (width / height).
    #[must_use]
    pub fn aspect(&self) -> f32 {
        if self.height == 0.0 {
            1.0
        } else {
            self.width / self.height
        }
    }
}
