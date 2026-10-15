//! Sealed booth facade and inexpensive, movie-textured projection haze.

use super::*;

#[cfg(test)]
pub(crate) const MESH_COUNT: usize = 8;
const LENS: Vec3 = Vec3::new(SCREEN_CENTER_X, 8.0, -0.22);
const SEGMENTS: usize = 24;
const STEPS: usize = 8;

pub(super) fn append(scene: &mut Theater) {
    let center = Vec3::new(LENS.x, LENS.y, -0.015);
    let corner = |x: f32, y: f32, z: f32| center + Vec3::new(x, y, z);
    scene.quad(
        [
            corner(0.8, -0.45, 0.0),
            corner(-0.8, -0.45, 0.0),
            corner(-0.8, 0.45, 0.0),
            corner(0.8, 0.45, 0.0),
        ],
        [[0.0; 2]; 4],
        [0.008, 0.009, 0.012, 1.0],
        None,
    );
    // The bevel projects into the auditorium; the original wall stays sealed.
    let outer = [(0.8, -0.45), (-0.8, -0.45), (-0.8, 0.45), (0.8, 0.45)];
    let inner = [(0.69, -0.34), (-0.69, -0.34), (-0.69, 0.34), (0.69, 0.34)];
    for i in 0..4 {
        let j = (i + 1) % 4;
        scene.quad(
            [
                corner(outer[i].0, outer[i].1, -0.09),
                corner(outer[j].0, outer[j].1, -0.09),
                corner(inner[j].0, inner[j].1, -0.025),
                corner(inner[i].0, inner[i].1, -0.025),
            ],
            [[0.0; 2]; 4],
            [0.24, 0.21, 0.17, 1.0],
            None,
        );
    }

    let mut barrel = Geometry::default();
    for i in 0..SEGMENTS {
        let a = radial(i);
        let b = radial(i + 1);
        for (r0, z0, r1, z1) in [(0.14, 0.12, 0.14, 0.0), (0.14, 0.0, 0.075, 0.0)] {
            barrel.quad(
                [
                    LENS + a * r0 + Vec3::Z * z0,
                    LENS + b * r0 + Vec3::Z * z0,
                    LENS + b * r1 + Vec3::Z * z1,
                    LENS + a * r1 + Vec3::Z * z1,
                ],
                [[0.5; 2]; 4],
                [1.0; 4],
            );
        }
    }
    let mut housing = barrel.finish(scene);
    housing.color = [0.28, 0.24, 0.18, 1.0];
    housing.lighting = Some(MeshLighting {
        position: Vec3::new(SCREEN_CENTER_X, HEIGHT - 0.3, MIN_Z * 0.5),
        ambient: 0.35,
        intensity: 0.65,
    });
    scene.instances.push(housing);

    let mut halo = Geometry::default();
    let origin = LENS - Vec3::Z * 0.005;
    for i in 0..SEGMENTS {
        let a = radial(i);
        let b = radial(i + 1);
        halo.triangle(
            [origin, origin + a * 0.06, origin + b * 0.06],
            [[0.5; 2]; 3],
            [1.0, 0.8, 0.8],
        );
        for (r0, alpha0, r1, alpha1) in [(0.06, 0.8, 0.12, 0.16), (0.12, 0.16, 0.32, 0.0)] {
            halo.quad(
                [
                    origin + a * r0,
                    origin + a * r1,
                    origin + b * r1,
                    origin + b * r0,
                ],
                [[0.5; 2]; 4],
                [alpha0, alpha1, alpha1, alpha0],
            );
        }
    }
    let mut glow = halo.finish(scene);
    projected_material(&mut glow, scene.assets.textures[4].handle);
    // Fixed center sampling intentionally does not participate in screen fitting.
    scene.instances.push(glow);

    let mut beam = Geometry::default();
    for direction in [Vec2::X, Vec2::Y, Vec2::ONE, Vec2::new(1.0, -1.0)] {
        for along in 0..STEPS {
            for across in 0..STEPS {
                let coordinates = [
                    (along, across),
                    (along, across + 1),
                    (along + 1, across + 1),
                    (along + 1, across),
                ];
                let mut positions = [Vec3::ZERO; 4];
                let mut uvs = [[0.0; 2]; 4];
                let mut alpha = [0.0; 4];
                for (i, (l, w)) in coordinates.into_iter().enumerate() {
                    // Concentrate subdivisions where the aperture expands fastest.
                    let t = (l as f32 / STEPS as f32).powi(2);
                    let s = w as f32 / STEPS as f32 * 2.0 - 1.0;
                    let offset = direction * s;
                    let near = LENS
                        + Vec3::new(offset.x * 0.065, offset.y * 0.065 / SCREEN_ASPECT, -0.015);
                    let far = Vec3::new(
                        SCREEN_CENTER_X + offset.x * SCREEN_WIDTH * 0.5,
                        SCREEN_BOTTOM + SCREEN_HEIGHT * (offset.y + 1.0) * 0.5,
                        MIN_Z + 0.14,
                    );
                    positions[i] = near.lerp(far, t);
                    // A ray keeps its image coordinate, rather than repeating the
                    // movie along the beam. Subdivision limits affine UV distortion.
                    uvs[i] = [(offset.x + 1.0) * 0.5, (1.0 - offset.y) * 0.5];
                    let edge = (1.0 - s * s).powi(2);
                    let ends = (t * 8.0).min(1.0) * ((1.0 - t) * 4.0).min(1.0);
                    alpha[i] = 0.045 * edge * ends;
                }
                beam.quad(positions, uvs, alpha);
            }
        }
    }
    let mut light = beam.finish(scene);
    projected_material(&mut light, scene.assets.textures[4].handle);
    light.texture_fit = Some(MeshTextureFit {
        surface_aspect: SCREEN_ASPECT,
        content_aspect: DEFAULT_VIDEO_ASPECT,
    });
    scene.instances.push(light);
}

fn projected_material(instance: &mut MeshInstance, video: SpriteHandle) {
    instance.render_mode = MeshRenderMode::Additive;
    instance.texture = Some(video);
    instance.lighting = Some(MeshLighting {
        position: Vec3::ZERO,
        ambient: 1.0,
        intensity: 0.0,
    });
}

fn radial(index: usize) -> Vec3 {
    let angle = -(index as f32) * std::f32::consts::TAU / SEGMENTS as f32;
    Vec3::new(angle.cos(), angle.sin(), 0.0)
}

#[derive(Default)]
struct Geometry {
    vertices: Vec<Vertex>,
    indices: Vec<u16>,
}

impl Geometry {
    fn triangle(&mut self, points: [Vec3; 3], uvs: [[f32; 2]; 3], alpha: [f32; 3]) {
        let normal = (points[1] - points[0])
            .cross(points[2] - points[0])
            .normalize();
        for i in 0..3 {
            self.indices
                .push(u16::try_from(self.vertices.len()).expect("bounded projector mesh"));
            self.vertices.push(Vertex {
                position: points[i].to_array(),
                normal: normal.to_array(),
                color: [1.0, 0.97, 0.91, alpha[i]],
                uv: uvs[i],
            });
        }
    }

    fn quad(&mut self, points: [Vec3; 4], uvs: [[f32; 2]; 4], alpha: [f32; 4]) {
        for indices in [[0, 1, 2], [0, 2, 3]] {
            self.triangle(
                indices.map(|i| points[i]),
                indices.map(|i| uvs[i]),
                indices.map(|i| alpha[i]),
            );
        }
    }

    fn finish(self, scene: &mut Theater) -> MeshInstance {
        let handle = MeshHandle::new();
        scene
            .assets
            .meshes
            .push(MeshAsset::new(handle, self.vertices, self.indices));
        MeshInstance::new(handle, RenderTransform::IDENTITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booth_and_projection_are_bounded_sealed_and_share_one_video() {
        let mut scene = Theater::new();
        let collider_count = scene.colliders.len();
        let texture_count = scene.assets.textures.len();
        append(&mut scene);
        let start = scene.instances.len() - MESH_COUNT;
        let additions = &scene.assets.meshes[start..];
        assert_eq!(scene.colliders.len(), collider_count);
        assert_eq!(scene.assets.textures.len(), texture_count);
        assert_eq!(additions.len(), MESH_COUNT);
        assert!(additions.iter().map(|m| m.indices.len() / 3).sum::<usize>() < 1_000);
        for mesh in additions {
            for vertex in &mesh.vertices {
                let p = Vec3::from(vertex.position);
                assert!(p.is_finite() && Vec3::from(vertex.normal).is_normalized());
                assert!(p.x > AUDITORIUM_MIN.x && p.x < AUDITORIUM_MAX.x);
                assert!(p.y > floor_height(p) && p.y < HEIGHT);
                assert!(p.z > MIN_Z + 0.09 && p.z < 0.0);
            }
        }
        let glow = scene.instances.len() - 2;
        let beam = scene.instances.len() - 1;
        for index in [glow, beam] {
            assert_eq!(scene.instances[index].render_mode, MeshRenderMode::Additive);
            assert_eq!(
                scene.instances[index].texture,
                Some(scene.assets.textures[4].handle)
            );
        }
        let before = scene.instances[glow..].to_vec();
        apply_room_lighting(
            &mut scene.instances,
            0.0,
            0.0,
            scene.assets.textures[4].handle,
        );
        assert_eq!(&scene.instances[glow..], before);
        fit_screen(
            &mut scene.instances,
            scene.assets.textures[4].handle,
            4.0 / 3.0,
        );
        assert_eq!(scene.instances[glow], before[0]);
        assert_eq!(
            scene.instances[beam].texture_fit.unwrap().content_aspect,
            4.0 / 3.0
        );
        let mesh = &scene.assets.meshes[beam];
        assert!(mesh.vertices.iter().any(|v| v.color[3] > 0.0));
        assert!(mesh.vertices.iter().any(|v| v.color[3] == 0.0));
        assert!(
            mesh.vertices
                .iter()
                .filter(|v| v.position[2] < MIN_Z + 0.15)
                .all(|v| v.color[3] == 0.0)
        );
        assert_eq!(scene.assets.textures[4].rgba, [0, 0, 0, 255]);
    }
}
