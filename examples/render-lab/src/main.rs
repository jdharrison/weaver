//! Render Lab — integrated example for the Weaver bootstrap milestone.
//!
//! This example composes meshes, sprites, particles, text, and minimal UI to
//! make rendering failures visually obvious.

use glam::{Quat, Vec2, Vec3};
use std::f32::consts::PI;
use weaver_app::{
    Application, ApplicationConfig, HeadlessApp, Renderable, WeaverWorld, WorldConfig,
};
use weaver_render::{
    MeshHandle, MeshInstance, ParticleEmitter, RenderTransform, SpriteHandle, SpriteInstance,
    SpriteSpace,
};
use weaver_render_wgpu::Vertex;
use weaver_worldline::FrameId;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Run headless if requested via environment.
    if std::env::var("WEAVER_HEADLESS").is_ok() {
        let mut app = HeadlessApp::new(WorldConfig::default())?.with_max_steps(60);
        app.run()?;
        return Ok(());
    }

    let config = ApplicationConfig {
        title: "Weaver Render Lab".to_string(),
        width: 1280,
        height: 720,
        world: WorldConfig::default(),
        setup: Some(Box::new(populate_lab_scene)),
        update: None,
        tooltip: None,
        format_time: None,
        title_status: None,
        side_menu: None,
        present_mode: wgpu::PresentMode::AutoVsync,
    };

    let app = Application::new(config);
    app.run()?;
    Ok(())
}

/// Populate the render lab scene in the given world and upload resources to
/// the renderer.
#[allow(clippy::too_many_lines)]
fn populate_lab_scene(world: &mut WeaverWorld, renderer: &mut weaver_render_wgpu::WgpuRenderer) {
    world.camera_mut().eye = Vec3::new(0.0, 3.0, 7.0);
    world.camera_mut().target = Vec3::ZERO;

    // Central textured mesh (a cube).
    let cube_mesh = MeshHandle::new();
    let (cube_vertices, cube_indices) = build_cube();
    renderer
        .upload_mesh(cube_mesh, &cube_vertices, &cube_indices)
        .ok();
    world.spawn(Renderable {
        mesh: Some(MeshInstance::new(
            cube_mesh,
            RenderTransform {
                translation: Vec3::ZERO,
                rotation: Quat::from_axis_angle(Vec3::Y, PI / 8.0),
                scale: 1.0,
            },
        )),
        sprite: None,
        emitter: None,
        label: Some("Cube".to_string()),
        frame: FrameId::ROOT,
    });

    // Instanced meshes orbiting the center.
    let small_mesh = MeshHandle::new();
    renderer
        .upload_mesh(small_mesh, &cube_vertices, &cube_indices)
        .ok();
    for i in 0..8 {
        let angle = i as f32 * (PI * 2.0 / 8.0);
        let x = angle.cos() * 2.5;
        let z = angle.sin() * 2.5;
        world.spawn(Renderable {
            mesh: Some(MeshInstance {
                mesh: small_mesh,
                transform: RenderTransform {
                    translation: Vec3::new(x, 0.5, z),
                    rotation: Quat::from_axis_angle(Vec3::Y, angle),
                    scale: 0.3,
                },
                color: [0.2 + i as f32 * 0.1, 0.6, 0.8, 1.0],
                emissive: 0.0,
            }),
            sprite: None,
            emitter: None,
            label: None,
            frame: FrameId::ROOT,
        });
    }

    // Animated sprite.
    let sprite_tex = SpriteHandle::new();
    let checker = build_checker_rgba(64);
    renderer
        .upload_texture_rgba(sprite_tex, 64, 64, &checker)
        .ok();
    world.spawn(Renderable {
        mesh: None,
        sprite: Some(SpriteInstance {
            sprite: sprite_tex,
            transform: RenderTransform {
                translation: Vec3::new(2.0, 1.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: 1.0,
            },
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            layer: 0,
            space: SpriteSpace::World,
            pivot: Vec2::new(0.5, 0.5),
        }),
        emitter: None,
        label: Some("Sprite".to_string()),
        frame: FrameId::ROOT,
    });

    // Transparent world-space sprites.
    for i in 0..5 {
        world.spawn(Renderable {
            mesh: None,
            sprite: Some(SpriteInstance {
                sprite: sprite_tex,
                transform: RenderTransform {
                    translation: Vec3::new(-2.0 + i as f32 * 0.5, 0.5, 1.0 + i as f32 * 0.3),
                    rotation: Quat::IDENTITY,
                    scale: 0.5,
                },
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                tint: [1.0, 1.0, 1.0, 0.5],
                layer: i,
                space: SpriteSpace::World,
                pivot: Vec2::new(0.5, 0.5),
            }),
            emitter: None,
            label: None,
            frame: FrameId::ROOT,
        });
    }

    // Particle emitter.
    world.spawn(Renderable {
        mesh: None,
        sprite: None,
        emitter: Some(ParticleEmitter::default()),
        label: Some("Particles".to_string()),
        frame: FrameId::ROOT,
    });

    // UI panel.
    world.spawn(Renderable {
        mesh: None,
        sprite: None,
        emitter: None,
        label: None,
        frame: FrameId::ROOT,
    });
    // The UI panel is added by the world snapshot extraction; additional
    // interactive widgets can be appended to the snapshot here in future slices.
}

#[allow(clippy::too_many_lines)]
fn build_cube() -> (Vec<Vertex>, Vec<u16>) {
    let vertices = vec![
        // Front
        Vertex {
            position: [-0.5, -0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            color: [1.0, 0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            color: [1.0, 0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            color: [1.0, 0.0, 0.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.5],
            normal: [0.0, 0.0, 1.0],
            color: [1.0, 0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
        // Back
        Vertex {
            position: [-0.5, -0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            color: [0.0, 1.0, 0.0, 1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [-0.5, 0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            color: [0.0, 1.0, 0.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, 0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            color: [0.0, 1.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, -0.5],
            normal: [0.0, 0.0, -1.0],
            color: [0.0, 1.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        // Top
        Vertex {
            position: [-0.5, 0.5, -0.5],
            normal: [0.0, 1.0, 0.0],
            color: [0.0, 0.0, 1.0, 1.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            color: [0.0, 0.0, 1.0, 1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            color: [0.0, 0.0, 1.0, 1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, -0.5],
            normal: [0.0, 1.0, 0.0],
            color: [0.0, 0.0, 1.0, 1.0],
            uv: [1.0, 1.0],
        },
        // Bottom
        Vertex {
            position: [-0.5, -0.5, -0.5],
            normal: [0.0, -1.0, 0.0],
            color: [1.0, 1.0, 0.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, -0.5],
            normal: [0.0, -1.0, 0.0],
            color: [1.0, 1.0, 0.0, 1.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.5],
            normal: [0.0, -1.0, 0.0],
            color: [1.0, 1.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [-0.5, -0.5, 0.5],
            normal: [0.0, -1.0, 0.0],
            color: [1.0, 1.0, 0.0, 1.0],
            uv: [1.0, 0.0],
        },
        // Right
        Vertex {
            position: [0.5, -0.5, -0.5],
            normal: [1.0, 0.0, 0.0],
            color: [1.0, 0.0, 1.0, 1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, -0.5],
            normal: [1.0, 0.0, 0.0],
            color: [1.0, 0.0, 1.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            color: [1.0, 0.0, 1.0, 1.0],
            uv: [0.0, 1.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
            color: [1.0, 0.0, 1.0, 1.0],
            uv: [0.0, 0.0],
        },
        // Left
        Vertex {
            position: [-0.5, -0.5, 0.5],
            normal: [-1.0, 0.0, 0.0],
            color: [0.0, 1.0, 1.0, 1.0],
            uv: [0.0, 0.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.5],
            normal: [-1.0, 0.0, 0.0],
            color: [0.0, 1.0, 1.0, 1.0],
            uv: [1.0, 0.0],
        },
        Vertex {
            position: [-0.5, 0.5, -0.5],
            normal: [-1.0, 0.0, 0.0],
            color: [0.0, 1.0, 1.0, 1.0],
            uv: [1.0, 1.0],
        },
        Vertex {
            position: [-0.5, -0.5, -0.5],
            normal: [-1.0, 0.0, 0.0],
            color: [0.0, 1.0, 1.0, 1.0],
            uv: [0.0, 1.0],
        },
    ];

    let indices: Vec<u16> = vec![
        0, 1, 2, 0, 2, 3, // front
        4, 5, 6, 4, 6, 7, // back
        8, 9, 10, 8, 10, 11, // top
        12, 13, 14, 12, 14, 15, // bottom
        16, 17, 18, 16, 18, 19, // right
        20, 21, 22, 20, 22, 23, // left
    ];

    (vertices, indices)
}

fn build_checker_rgba(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let c = if (x / 8 + y / 8) % 2 == 0 { 220 } else { 40 };
            data.extend_from_slice(&[c, c / 2, c / 3, 255]);
        }
    }
    data
}
