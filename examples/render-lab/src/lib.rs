//! Shared renderer feature lab for desktop and browser shells.

use glam::{Quat, Vec2, Vec3};
use std::f32::consts::PI;
use weaver_app_core::{
    AppAction, AppAssets, FrameContext, InputFrame, MeshAsset, OrbitController, PointerMode,
    TextureAsset, WeaverApp,
};
use weaver_core::EntityId;
use weaver_render::{
    Camera, MeshHandle, MeshInstance, Particle, ParticleEmitter, RenderTransform, SceneSnapshot,
    SpriteHandle, SpriteInstance, SpriteSpace,
};
use weaver_render_wgpu::Vertex;

/// Renderer feature scene shared by native and browser platform shells.
pub struct RenderLab {
    scene: SceneSnapshot,
    assets: AppAssets,
    orbit: OrbitController,
}

impl RenderLab {
    /// Build the meshes, sprites, particles, and camera used by the lab.
    #[must_use]
    pub fn new() -> Self {
        let cube_mesh = MeshHandle::new();
        let small_mesh = MeshHandle::new();
        let sprite = SpriteHandle::new();
        let (cube_vertices, cube_indices) = build_cube();

        let mut scene = SceneSnapshot {
            camera: Camera {
                eye: Vec3::new(0.0, 3.0, 7.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            background_color: [0.01, 0.015, 0.025, 1.0],
            ..SceneSnapshot::default()
        };
        scene.meshes.push(MeshInstance::new(
            cube_mesh,
            RenderTransform {
                translation: Vec3::ZERO,
                rotation: Quat::from_axis_angle(Vec3::Y, PI / 8.0),
                scale: 1.0,
            },
        ));
        for index in 0..8 {
            let angle = index as f32 * (PI * 2.0 / 8.0);
            scene.meshes.push(MeshInstance {
                mesh: small_mesh,
                transform: RenderTransform {
                    translation: Vec3::new(angle.cos() * 2.5, 0.5, angle.sin() * 2.5),
                    rotation: Quat::from_axis_angle(Vec3::Y, angle),
                    scale: 0.3,
                },
                color: [0.2 + index as f32 * 0.1, 0.6, 0.8, 1.0],
                emissive: 0.0,
            });
        }

        scene
            .sprites
            .push(world_sprite(sprite, Vec3::new(2.0, 1.0, 0.0), 1.0, 1.0, 0));
        for index in 0..5 {
            scene.sprites.push(world_sprite(
                sprite,
                Vec3::new(-2.0 + index as f32 * 0.5, 0.5, 1.0 + index as f32 * 0.3),
                0.5,
                0.5,
                index,
            ));
        }

        scene.particles.push((
            EntityId::new(),
            ParticleEmitter::default(),
            build_particles(),
        ));
        let orbit = OrbitController::from_camera(&scene.camera);
        let assets = AppAssets {
            meshes: vec![
                MeshAsset::new(cube_mesh, cube_vertices.clone(), cube_indices.clone()),
                MeshAsset::new(small_mesh, cube_vertices, cube_indices),
            ],
            textures: vec![TextureAsset {
                handle: sprite,
                width: 64,
                height: 64,
                rgba: build_checker_rgba(64),
            }],
        };

        Self {
            scene,
            assets,
            orbit,
        }
    }
}

impl Default for RenderLab {
    fn default() -> Self {
        Self::new()
    }
}

impl WeaverApp for RenderLab {
    fn title(&self) -> &'static str {
        "WVR | Render Lab"
    }

    fn scene(&self) -> &SceneSnapshot {
        &self.scene
    }

    fn scene_mut(&mut self) -> &mut SceneSnapshot {
        &mut self.scene
    }

    fn assets(&self) -> &AppAssets {
        &self.assets
    }

    fn pointer_mode(&self) -> PointerMode {
        PointerMode::OrbitDrag
    }

    fn update(&mut self, _frame: FrameContext, input: &InputFrame) {
        self.orbit.update(&mut self.scene.camera, input);
        for action in &input.actions {
            match action {
                AppAction::TogglePause => self.scene.paused = !self.scene.paused,
                AppAction::SetTimeMultiplier(multiplier) => {
                    self.scene.time_multiplier = *multiplier;
                }
                AppAction::ToggleCoordinateFrames => {
                    self.scene.show_coordinate_frames = !self.scene.show_coordinate_frames;
                }
                AppAction::ToggleTrajectoryHistory => {
                    self.scene.show_trajectory_history = !self.scene.show_trajectory_history;
                }
            }
        }
    }
}

/// Start Render Lab in its browser shell.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    std::panic::set_hook(Box::new(|panic| {
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&panic.to_string()));
    }));
    weaver_platform_web::run(
        Box::new(RenderLab::new()),
        weaver_platform_web::WebConfig::default(),
    )
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

fn world_sprite(
    sprite: SpriteHandle,
    translation: Vec3,
    scale: f32,
    alpha: f32,
    layer: i32,
) -> SpriteInstance {
    SpriteInstance {
        sprite,
        transform: RenderTransform {
            translation,
            rotation: Quat::IDENTITY,
            scale,
        },
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        tint: [1.0, 1.0, 1.0, alpha],
        layer,
        space: SpriteSpace::World,
        pivot: Vec2::splat(0.5),
    }
}

fn build_particles() -> Vec<Particle> {
    (0..48)
        .map(|index| {
            let angle = index as f32 * 2.399_963_1;
            let ring = 0.35 + (index % 9) as f32 * 0.07;
            Particle {
                position: Vec3::new(
                    angle.cos() * ring,
                    1.0 + index as f32 * 0.025,
                    angle.sin() * ring,
                ),
                velocity: Vec3::ZERO,
                age: 0.0,
                lifetime: 1.0,
                size: 0.06 + (index % 3) as f32 * 0.025,
                color: [1.0, 0.45 + (index % 5) as f32 * 0.08, 0.15, 0.85],
            }
        })
        .collect()
}

fn build_cube() -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    push_face(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-0.5, -0.5, 0.5),
            Vec3::new(0.5, -0.5, 0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(-0.5, 0.5, 0.5),
        ],
        Vec3::Z,
        [1.0, 0.0, 0.0, 1.0],
    );
    push_face(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(0.5, 0.5, -0.5),
            Vec3::new(0.5, -0.5, -0.5),
        ],
        Vec3::NEG_Z,
        [0.0, 1.0, 0.0, 1.0],
    );
    push_face(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(-0.5, 0.5, 0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.5, 0.5, -0.5),
        ],
        Vec3::Y,
        [0.0, 0.0, 1.0, 1.0],
    );
    push_face(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(0.5, -0.5, -0.5),
            Vec3::new(0.5, -0.5, 0.5),
            Vec3::new(-0.5, -0.5, 0.5),
        ],
        Vec3::NEG_Y,
        [1.0, 1.0, 0.0, 1.0],
    );
    push_face(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(0.5, -0.5, -0.5),
            Vec3::new(0.5, 0.5, -0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.5, -0.5, 0.5),
        ],
        Vec3::X,
        [1.0, 0.0, 1.0, 1.0],
    );
    push_face(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-0.5, -0.5, 0.5),
            Vec3::new(-0.5, 0.5, 0.5),
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(-0.5, -0.5, -0.5),
        ],
        Vec3::NEG_X,
        [0.0, 1.0, 1.0, 1.0],
    );
    (vertices, indices)
}

fn push_face(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    positions: [Vec3; 4],
    normal: Vec3,
    color: [f32; 4],
) {
    let base = u16::try_from(vertices.len()).expect("cube mesh fits in u16 indices");
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (position, uv) in positions.into_iter().zip(uvs) {
        vertices.push(Vertex {
            position: position.to_array(),
            normal: normal.to_array(),
            color,
            uv,
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn build_checker_rgba(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let color = if (x / 8 + y / 8) % 2 == 0 { 220 } else { 40 };
            data.extend_from_slice(&[color, color / 2, color / 3, 255]);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_contains_expected_renderer_features() {
        let app = RenderLab::new();
        assert_eq!(app.assets.meshes.len(), 2);
        assert_eq!(app.assets.textures.len(), 1);
        assert_eq!(app.scene.meshes.len(), 9);
        assert_eq!(app.scene.sprites.len(), 6);
        assert_eq!(app.scene.particles.len(), 1);
        assert_eq!(app.scene.particles[0].2.len(), 48);
    }

    #[test]
    fn cube_mesh_has_six_faces() {
        let (vertices, indices) = build_cube();
        assert_eq!(vertices.len(), 24);
        assert_eq!(indices.len(), 36);
    }
}
