//! Sprite pipeline and instance buffer management.

use crate::error::WgpuRenderError;
use crate::pipeline::{
    SPRITE_SHADER, camera_bind_group_layout, create_shader, instance_uniform_layout,
};

const MAX_INSTANCES: usize = 160;
use crate::resource::GpuTexture;
use bytemuck::{Pod, Zeroable};
use weaver_render::{SpriteInstance, SpriteSpace};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteInstanceRaw {
    model: [[f32; 4]; 4],
    uv_rect: [f32; 4],
    tint: [f32; 4],
}

/// Sprite rendering pipeline and resources.
pub struct SpritePipeline {
    world_pipeline: wgpu::RenderPipeline,
    screen_pipeline: wgpu::RenderPipeline,
    _camera_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
}

impl SpritePipeline {
    /// Create world and screen sprite pipelines with effective scene samples.
    /// Straight-alpha blending and draw ordering remain unchanged; no alpha-to-coverage.
    ///
    /// # Errors
    ///
    /// Returns an error if pipeline creation fails.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Result<Self, WgpuRenderError> {
        let camera_layout = camera_bind_group_layout(device);
        let instance_layout = instance_uniform_layout(device);
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite-texture-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = create_shader(device, "sprite-shader", SPRITE_SHADER);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite-pipeline-layout"),
            bind_group_layouts: &[&camera_layout, &instance_layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let make_pipeline = |label, depth_compare| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: false,
                    depth_compare,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    ..Default::default()
                },
                multiview: None,
                cache: None,
            })
        };
        let world_pipeline = make_pipeline("world-sprite-pipeline", wgpu::CompareFunction::Less);
        let screen_pipeline =
            make_pipeline("screen-sprite-pipeline", wgpu::CompareFunction::Always);
        Ok(Self {
            world_pipeline,
            screen_pipeline,
            _camera_layout: camera_layout,
            instance_layout,
            texture_layout,
        })
    }

    /// Render ordered sprite draws, batching only adjacent matching textures/spaces.
    ///
    /// # Errors
    ///
    /// Returns an error if instance buffer allocation fails.
    pub fn render<'pass>(
        &'pass mut self,
        pass: &mut wgpu::RenderPass<'pass>,
        device: &wgpu::Device,
        camera_bind_group: &'pass wgpu::BindGroup,
        draws: &[(&GpuTexture, &SpriteInstance)],
    ) -> Result<(), WgpuRenderError> {
        if draws.is_empty() {
            return Ok(());
        }
        pass.set_bind_group(0, camera_bind_group, &[]);
        for chunk in draws.chunks(MAX_INSTANCES) {
            // Each chunk needs distinct storage: queue writes to a reused buffer
            // would make earlier draws read the final chunk at submission time.
            let mut raw = vec![SpriteInstanceRaw::zeroed(); MAX_INSTANCES];
            for (out, (texture, sprite)) in raw.iter_mut().zip(chunk) {
                *out = SpriteInstanceRaw {
                    model: sprite_model(sprite, texture.size).to_cols_array_2d(),
                    uv_rect: sprite.uv_rect,
                    tint: sprite.tint,
                };
            }
            let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sprite-instances"),
                contents: bytemuck::cast_slice(&raw),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sprite-instance-bind-group"),
                layout: &self.instance_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instance_buffer.as_entire_binding(),
                }],
            });
            pass.set_bind_group(1, &instance_bind_group, &[]);

            let mut start = 0;
            while start < chunk.len() {
                let (texture, sprite) = chunk[start];
                let mut end = start + 1;
                while end < chunk.len()
                    && std::ptr::eq(chunk[end].0, texture)
                    && chunk[end].1.space == sprite.space
                {
                    end += 1;
                }
                let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sprite-texture-bind-group"),
                    layout: &self.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&texture.sampler),
                        },
                    ],
                });
                pass.set_pipeline(match sprite.space {
                    SpriteSpace::World => &self.world_pipeline,
                    SpriteSpace::Screen => &self.screen_pipeline,
                });
                pass.set_bind_group(2, &texture_bind_group, &[]);
                pass.draw(0..4, start as u32..end as u32);
                start = end;
            }
        }
        Ok(())
    }
}

fn sprite_model(sprite: &SpriteInstance, texture_size: (u32, u32)) -> glam::Mat4 {
    let mut model = sprite.transform.model_matrix();
    if sprite.space == SpriteSpace::World {
        // Uniform scale remains the height; width follows the sampled atlas region.
        // Absolute extents keep UV flips independent of geometry orientation.
        let width = texture_size.0 as f32 * sprite.uv_rect[2].abs();
        let height = texture_size.1 as f32 * sprite.uv_rect[3].abs();
        let aspect = width / height;
        // Degenerate/non-finite regions retain the uncorrected square geometry.
        if width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
            && aspect.is_finite()
            && aspect > 0.0
        {
            model.x_axis *= aspect;
        }
    }
    model
}

pub(crate) fn ordered_sprites(
    sprites: &[SpriteInstance],
    view: glam::Mat4,
) -> Vec<&SpriteInstance> {
    let mut world: Vec<_> = sprites
        .iter()
        .filter(|s| s.space == SpriteSpace::World)
        .collect();
    // Right-handed view space looks along -Z. Stable sorting preserves ties.
    world.sort_by(|a, b| {
        let a_depth = view.transform_point3(a.transform.translation).z;
        let b_depth = view.transform_point3(b.transform.translation).z;
        a_depth.total_cmp(&b_depth)
    });
    world.extend(sprites.iter().filter(|s| s.space == SpriteSpace::Screen));
    world
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Quat, Vec3};
    use weaver_render::{RenderTransform, SpriteHandle};

    #[test]
    fn world_sprite_uses_texture_aspect_without_changing_height_or_pose() {
        let sprite = SpriteInstance {
            transform: RenderTransform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_y(0.7),
                scale: 2.0,
            },
            ..SpriteInstance::default()
        };
        let model = sprite_model(&sprite, (300, 400));
        assert!((model.x_axis.length() - 1.5).abs() < 1e-6);
        assert!((model.y_axis.length() - 2.0).abs() < 1e-6);
        assert_eq!(model.w_axis.truncate(), sprite.transform.translation);
        assert_eq!(model.y_axis, sprite.transform.model_matrix().y_axis);
        assert_eq!(model.z_axis, sprite.transform.model_matrix().z_axis);
    }

    #[test]
    fn world_sprite_square_atlas_region_remains_square() {
        let sprite = SpriteInstance {
            uv_rect: [0.5, 0.0, 0.5, 1.0],
            transform: RenderTransform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_y(0.7),
                scale: 2.0,
            },
            ..SpriteInstance::default()
        };
        assert_eq!(
            sprite_model(&sprite, (1024, 512)),
            sprite.transform.model_matrix()
        );
    }

    #[test]
    fn world_sprite_flipped_uv_regions_preserve_aspect_and_pose() {
        for (width, height) in [(0.25, 1.0), (-0.25, 1.0), (0.25, -1.0), (-0.25, -1.0)] {
            let sprite = SpriteInstance {
                uv_rect: [0.5, 0.5, width, height],
                ..SpriteInstance::default()
            };
            let model = sprite_model(&sprite, (1024, 512));
            let mut expected = sprite.transform.model_matrix();
            expected.x_axis *= 0.5;
            assert_eq!(model, expected);
        }
    }

    #[test]
    fn world_sprite_degenerate_regions_keep_uncorrected_geometry() {
        for (texture_size, width, height) in [
            ((1024, 512), 0.0, 1.0),
            ((1024, 512), 1.0, 0.0),
            ((1024, 512), 0.0, 0.0),
            ((0, 512), 1.0, 1.0),
            ((1024, 0), 1.0, 1.0),
            ((1024, 512), f32::NAN, 1.0),
            ((1024, 512), 1.0, f32::NAN),
            ((1024, 512), f32::INFINITY, 1.0),
            ((1024, 512), 1.0, f32::NEG_INFINITY),
            ((1024, 512), f32::MAX, 1.0),
            ((1024, 512), 1.0, f32::MAX),
            ((1, 1), f32::MAX, f32::MIN_POSITIVE),
            ((1, 1), f32::MIN_POSITIVE, f32::MAX),
        ] {
            let sprite = SpriteInstance {
                uv_rect: [0.0, 0.0, width, height],
                transform: RenderTransform {
                    translation: Vec3::new(1.0, 2.0, 3.0),
                    rotation: Quat::from_rotation_y(0.7),
                    scale: 2.0,
                },
                ..SpriteInstance::default()
            };
            assert_eq!(
                sprite_model(&sprite, texture_size),
                sprite.transform.model_matrix()
            );
        }
    }

    #[test]
    fn screen_sprite_model_is_unchanged_for_portrait_texture() {
        let sprite = SpriteInstance {
            space: SpriteSpace::Screen,
            transform: RenderTransform {
                translation: Vec3::new(100.0, 200.0, 0.0),
                rotation: Quat::from_rotation_z(0.3),
                scale: 32.0,
            },
            ..SpriteInstance::default()
        };
        assert_eq!(
            sprite_model(&sprite, (300, 400)),
            sprite.transform.model_matrix()
        );
    }

    #[test]
    fn world_depth_order_is_global_across_textures_and_screen_order_is_preserved() {
        let a = SpriteHandle::new();
        let b = SpriteHandle::new();
        let make_sprite = |handle, position, space| SpriteInstance {
            sprite: handle,
            transform: RenderTransform {
                translation: position,
                ..RenderTransform::default()
            },
            space,
            ..SpriteInstance::default()
        };
        let sprites = vec![
            make_sprite(a, Vec3::new(0.0, 0.0, -2.0), SpriteSpace::World),
            make_sprite(b, Vec3::ZERO, SpriteSpace::Screen),
            make_sprite(a, Vec3::new(0.0, 0.0, -8.0), SpriteSpace::World),
            make_sprite(b, Vec3::new(100.0, 0.0, -5.0), SpriteSpace::World),
            make_sprite(a, Vec3::ZERO, SpriteSpace::Screen),
            make_sprite(b, Vec3::new(0.0, 0.0, -2.0), SpriteSpace::World),
        ];
        let ordered = ordered_sprites(&sprites, Mat4::IDENTITY);
        let indices: Vec<_> = ordered
            .iter()
            .map(|s| {
                sprites
                    .iter()
                    .position(|original| std::ptr::eq(*s, original))
                    .unwrap()
            })
            .collect();
        // Off-axis distance must not override view depth; equal depths stay stable.
        assert_eq!(indices, [2, 3, 0, 5, 1, 4]);
        assert_eq!(
            ordered.iter().map(|s| s.sprite).collect::<Vec<_>>(),
            [a, b, a, b, b, a]
        );
    }

    #[test]
    fn world_depth_order_follows_camera_view_direction() {
        let sprites: Vec<_> = [2.0, 8.0, 5.0]
            .into_iter()
            .map(|x| SpriteInstance {
                transform: RenderTransform {
                    translation: Vec3::new(x, 0.0, 0.0),
                    ..RenderTransform::default()
                },
                ..SpriteInstance::default()
            })
            .collect();
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::X, Vec3::Y);
        let ordered = ordered_sprites(&sprites, view);
        assert_eq!(
            ordered
                .iter()
                .map(|s| s.transform.translation.x)
                .collect::<Vec<_>>(),
            [8.0, 5.0, 2.0]
        );
    }
}
