//! Particle pipeline and buffer management.

use crate::error::WgpuRenderError;
use crate::pipeline::{
    PARTICLE_SHADER, camera_bind_group_layout, create_shader, instance_uniform_layout,
};

const MAX_PARTICLES: usize = 500;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleRaw {
    position: [f32; 3],
    size: f32,
    color: [f32; 4],
}

/// Particle rendering pipeline.
pub struct ParticlePipeline {
    pipeline: wgpu::RenderPipeline,
    _camera_layout: wgpu::BindGroupLayout,
    particle_layout: wgpu::BindGroupLayout,
    particle_buffer: wgpu::Buffer,
    particle_capacity: usize,
}

impl ParticlePipeline {
    /// Create the particle pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if pipeline creation fails.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
    ) -> Result<Self, WgpuRenderError> {
        let camera_layout = camera_bind_group_layout(device);
        let particle_layout = instance_uniform_layout(device);
        let shader = create_shader(device, "particle-shader", PARTICLE_SHADER);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle-pipeline-layout"),
            bind_group_layouts: &[&camera_layout, &particle_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle-pipeline"),
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
        let particle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: (MAX_PARTICLES * std::mem::size_of::<ParticleRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            _camera_layout: camera_layout,
            particle_layout,
            particle_buffer,
            particle_capacity: MAX_PARTICLES,
        })
    }

    /// Render particles.
    ///
    /// # Errors
    ///
    /// Returns an error if buffer allocation fails.
    pub fn render<'pass>(
        &'pass mut self,
        pass: &mut wgpu::RenderPass<'pass>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_bind_group: &'pass wgpu::BindGroup,
        particles: &[weaver_render::Particle],
    ) -> Result<(), WgpuRenderError> {
        if particles.is_empty() {
            return Ok(());
        }
        let count = particles.len().min(MAX_PARTICLES);
        self.ensure_capacity(device, count);

        let raw: Vec<ParticleRaw> = particles
            .iter()
            .take(count)
            .map(|p| ParticleRaw {
                position: [p.position.x, p.position.y, p.position.z],
                size: p.size,
                color: p.color,
            })
            .collect();
        queue.write_buffer(&self.particle_buffer, 0, bytemuck::cast_slice(&raw));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bind-group"),
            layout: &self.particle_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.particle_buffer.as_entire_binding(),
            }],
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..count as u32);
        Ok(())
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, required: usize) {
        if required <= self.particle_capacity {
            return;
        }
        let new_capacity = (self.particle_capacity * 2)
            .max(required)
            .min(MAX_PARTICLES);
        if new_capacity == self.particle_capacity {
            return;
        }
        self.particle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: (new_capacity * std::mem::size_of::<ParticleRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.particle_capacity = new_capacity;
    }
}
