//! Allocation-free conservative CPU visibility tests for WGPU clip depth `0..w`.

use glam::{Mat4, Vec3, Vec4};

/// Immutable local-space axis-aligned bounds. Invalid/empty input has no bounds.
#[derive(Clone, Copy, Debug)]
pub struct MeshBounds {
    min: Vec3,
    max: Vec3,
}

impl MeshBounds {
    /// Compute bounds once from all uploaded positions, including unreferenced vertices.
    pub fn from_positions(positions: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for position in positions {
            let p = Vec3::from_array(position);
            if !p.is_finite() {
                return None;
            }
            min = min.min(p);
            max = max.max(p);
        }
        (min.is_finite() && max.is_finite()).then_some(Self { min, max })
    }
}

/// Six inward-facing homogeneous world planes; invalid cameras disable rejection.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: Option<[Vec4; 6]>,
}

impl Frustum {
    /// Extract WGPU planes: `-w <= x,y <= w`, `0 <= z <= w`.
    pub fn from_view_projection(matrix: Mat4) -> Self {
        let rows = matrix.transpose();
        let planes = [
            rows.w_axis + rows.x_axis,
            rows.w_axis - rows.x_axis,
            rows.w_axis + rows.y_axis,
            rows.w_axis - rows.y_axis,
            rows.z_axis,
            rows.w_axis - rows.z_axis,
        ];
        Self {
            planes: (matrix.is_finite()
                && matrix.determinant().is_finite()
                && matrix.determinant() != 0.0
                && planes.iter().all(|p| p.is_finite()))
            .then_some(planes),
        }
    }

    /// Keep boxes intersecting any boundary, including boxes surrounding the camera.
    /// Pulling planes into local space handles rotation, shear and negative/nonuniform
    /// scale without approximating a sphere radius. Missing/invalid bounds fail open.
    pub fn is_visible(&self, bounds: Option<MeshBounds>, model: Mat4) -> bool {
        let (Some(planes), Some(bounds)) = (self.planes, bounds) else {
            return true;
        };
        if !model.is_finite()
            || model.x_axis.w != 0.0
            || model.y_axis.w != 0.0
            || model.z_axis.w != 0.0
            || model.w_axis.w != 1.0
        {
            return true;
        }
        let local = planes.map(|p| model.transpose() * p);
        // Validate all planes before rejecting against any one of them.
        if local.iter().any(|p| !p.is_finite()) {
            return true;
        }
        let mut outside = false;
        for p in local {
            let support = Vec3::new(
                if p.x >= 0.0 {
                    bounds.max.x
                } else {
                    bounds.min.x
                },
                if p.y >= 0.0 {
                    bounds.max.y
                } else {
                    bounds.min.y
                },
                if p.z >= 0.0 {
                    bounds.max.z
                } else {
                    bounds.min.z
                },
            );
            let terms = p.truncate() * support;
            let distance = terms.element_sum() + p.w;
            // Relative slack protects boundary contact from f32 matrix/dot rounding.
            let slack = 32.0 * f32::EPSILON * (terms.abs().element_sum() + p.w.abs()).max(1.0);
            if !distance.is_finite() || !slack.is_finite() {
                return true;
            }
            outside |= distance < -slack;
        }
        !outside
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn point() -> Option<MeshBounds> {
        MeshBounds::from_positions([[0.0; 3]])
    }

    #[test]
    fn wgpu_six_boundaries_and_outside() {
        let f = Frustum::from_view_projection(Mat4::IDENTITY);
        for p in [
            Vec3::new(-1.0, 0.0, 0.5),
            Vec3::new(1.0, 0.0, 0.5),
            Vec3::new(0.0, -1.0, 0.5),
            Vec3::new(0.0, 1.0, 0.5),
            Vec3::ZERO,
            Vec3::Z,
        ] {
            assert!(f.is_visible(point(), Mat4::from_translation(p)));
            let outside = p + (p - Vec3::new(0.0, 0.0, 0.5)).normalize() * 0.01;
            assert!(!f.is_visible(point(), Mat4::from_translation(outside)));
        }
    }

    #[test]
    fn perspective_aspect_rotation_and_orthographic() {
        for aspect in [0.5, 1.0, 2.0] {
            let projection = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, aspect, 1.0, 10.0);
            let view =
                Mat4::look_at_rh(Vec3::new(3.0, 2.0, 1.0), Vec3::new(4.0, 2.0, 1.0), Vec3::Y);
            let f = Frustum::from_view_projection(projection * view);
            for p in [
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(0.0, 0.0, -10.0),
                Vec3::new(2.0 * aspect, 0.0, -2.0),
            ] {
                assert!(f.is_visible(point(), view.inverse() * Mat4::from_translation(p)));
            }
            for p in [
                Vec3::new(0.0, 0.0, 2.0),
                Vec3::new(0.0, 0.0, -0.5),
                Vec3::new(0.0, 0.0, -11.0),
                Vec3::new(2.1 * aspect, 0.0, -2.0),
            ] {
                assert!(!f.is_visible(point(), view.inverse() * Mat4::from_translation(p)));
            }
        }
        let f =
            Frustum::from_view_projection(Mat4::orthographic_rh(-2.0, 2.0, -1.0, 1.0, 1.0, 10.0));
        assert!(f.is_visible(point(), Mat4::from_translation(Vec3::new(2.0, 1.0, -1.0))));
        assert!(!f.is_visible(point(), Mat4::from_translation(Vec3::new(2.1, 0.0, -5.0))));
    }

    #[test]
    fn transformed_box_and_camera_inside_are_conservative() {
        let bounds = MeshBounds::from_positions([[-1.0; 3], [1.0; 3]]);
        let f = Frustum::from_view_projection(Mat4::IDENTITY);
        let model = Mat4::from_scale_rotation_translation(
            Vec3::new(-2.0, 0.1, 0.2),
            Quat::from_rotation_z(0.7),
            Vec3::new(2.0, 0.0, 0.5),
        );
        assert!(f.is_visible(bounds, model));
        assert!(!f.is_visible(bounds, Mat4::from_translation(Vec3::X * 10.0) * model));
        let perspective = Frustum::from_view_projection(Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0));
        assert!(perspective.is_visible(bounds, Mat4::from_scale(Vec3::splat(1000.0))));
    }

    #[test]
    fn transformed_support_matches_eight_corner_reference() {
        let bounds = MeshBounds::from_positions([[-1.0; 3], [1.0; 3]]);
        for projection in [
            Mat4::perspective_rh(1.2, 1.7, 0.1, 100.0),
            Mat4::orthographic_rh(-5.0, 5.0, -3.0, 3.0, 0.1, 100.0),
        ] {
            let f = Frustum::from_view_projection(projection);
            for i in 0..200 {
                let angle = i as f32 * 0.137;
                let mut model = Mat4::from_scale_rotation_translation(
                    Vec3::new(-0.2 - angle * 0.1, 2.0, 0.3),
                    Quat::from_rotation_y(angle) * Quat::from_rotation_z(angle),
                    Vec3::new(angle.sin() * 20.0, angle.cos() * 10.0, -angle),
                );
                model.y_axis += model.x_axis * 0.3; // affine shear
                let corners: [Vec4; 8] = std::array::from_fn(|bits| {
                    let p = Vec3::new(
                        if bits & 1 == 0 { -1.0 } else { 1.0 },
                        if bits & 2 == 0 { -1.0 } else { 1.0 },
                        if bits & 4 == 0 { -1.0 } else { 1.0 },
                    );
                    projection * model * p.extend(1.0)
                });
                let outside = (0..6).any(|plane| {
                    corners.iter().all(|p| {
                        let d = match plane {
                            0 => p.w + p.x,
                            1 => p.w - p.x,
                            2 => p.w + p.y,
                            3 => p.w - p.y,
                            4 => p.z,
                            _ => p.w - p.z,
                        };
                        d < 0.0
                    })
                });
                // Conservative slack may retain extra boxes, never reject a reference hit.
                if !outside {
                    assert!(f.is_visible(bounds, model), "case {i}");
                }
            }
        }
    }

    #[test]
    fn invalid_data_fails_open() {
        assert!(MeshBounds::from_positions([]).is_none());
        assert!(MeshBounds::from_positions([[f32::NAN; 3]]).is_none());
        let outside = Mat4::from_translation(Vec3::splat(100.0));
        for matrix in [Mat4::ZERO, Mat4::from_cols_array(&[f32::NAN; 16])] {
            assert!(Frustum::from_view_projection(matrix).is_visible(point(), outside));
        }
        let f = Frustum::from_view_projection(Mat4::IDENTITY);
        assert!(f.is_visible(None, outside));
        assert!(f.is_visible(point(), Mat4::from_cols_array(&[f32::INFINITY; 16])));
    }
}
