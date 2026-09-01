//! Sprite pipeline and instance buffer management.

use crate::error::WgpuRenderError;
use crate::pipeline::{
    SPRITE_SHADER, camera_bind_group_layout, create_shader, instance_uniform_layout,
};

const MAX_INSTANCES: usize = 160;
use crate::resource::GpuTexture;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteInstanceRaw {
    model: [[f32; 4]; 4],
    uv_rect: [f32; 4],
    tint: [f32; 4],
}

/// Sprite rendering pipeline and resources.
pub struct SpritePipeline {
    pipeline: wgpu::RenderPipeline,
    _camera_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
}

impl SpritePipeline {
    /// Create the sprite pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if pipeline creation fails.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite-pipeline"),
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
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite-instances"),
            size: (MAX_INSTANCES * std::mem::size_of::<SpriteInstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            _camera_layout: camera_layout,
            instance_layout,
            texture_layout,
            instance_buffer,
            instance_capacity: MAX_INSTANCES,
        })
    }

    /// Render sprite instances.
    ///
    /// # Errors
    ///
    /// Returns an error if instance buffer allocation fails.
    pub fn render<'pass>(
        &'pass mut self,
        pass: &mut wgpu::RenderPass<'pass>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_bind_group: &'pass wgpu::BindGroup,
        texture: &GpuTexture,
        sprites: &[weaver_render::SpriteInstance],
    ) -> Result<(), WgpuRenderError> {
        if sprites.is_empty() {
            return Ok(());
        }
        let count = sprites.len().min(MAX_INSTANCES);
        self.ensure_capacity(device, count);

        let mut raw = Vec::with_capacity(count);
        for sprite in sprites.iter().take(count) {
            let model = sprite.transform.model_matrix();
            raw.push(SpriteInstanceRaw {
                model: model.to_cols_array_2d(),
                uv_rect: sprite.uv_rect,
                tint: sprite.tint,
            });
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&raw));

        let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite-instance-bind-group"),
            layout: &self.instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.instance_buffer.as_entire_binding(),
            }],
        });
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

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &instance_bind_group, &[]);
        pass.set_bind_group(2, &texture_bind_group, &[]);
        pass.draw(0..4, 0..sprites.len() as u32);
        Ok(())
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, required: usize) {
        if required <= self.instance_capacity {
            return;
        }
        let new_capacity = (self.instance_capacity * 2)
            .max(required)
            .min(MAX_INSTANCES);
        if new_capacity == self.instance_capacity {
            return;
        }
        self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite-instances"),
            size: (new_capacity * std::mem::size_of::<SpriteInstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }
}
