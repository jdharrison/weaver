//! Minimal UI rectangle pipeline.

use crate::error::WgpuRenderError;
use crate::pipeline::{UI_SHADER, create_shader};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const MAX_RECTS: usize = 250;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiUniform {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiRectRaw {
    position: [f32; 2],
    size: [f32; 2],
    background: [f32; 4],
    border_color: [f32; 4],
    border_width: f32,
    corner_radius: f32,
    layer: f32,
    _pad: f32,
}

/// UI rectangle rendering pipeline.
pub struct UiPipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    rect_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    rect_buffer: wgpu::Buffer,
    rect_capacity: usize,
    screen_size: (u32, u32),
}

impl UiPipeline {
    /// Create the UI pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if pipeline creation fails.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        screen_width: u32,
        screen_height: u32,
    ) -> Result<Self, WgpuRenderError> {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let rect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-rect-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = create_shader(device, "ui-shader", UI_SHADER);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-pipeline-layout"),
            bind_group_layouts: &[&uniform_layout, &rect_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui-pipeline"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui-uniform"),
            contents: bytemuck::cast_slice(&[UiUniform {
                screen_size: [screen_width as f32, screen_height as f32],
                _pad: [0.0; 2],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let rect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-rects"),
            size: (MAX_RECTS * std::mem::size_of::<UiRectRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            uniform_layout,
            rect_layout,
            uniform_buffer,
            rect_buffer,
            rect_capacity: MAX_RECTS,
            screen_size: (screen_width, screen_height),
        })
    }

    /// Update screen size.
    pub fn resize(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        self.screen_size = (width, height);
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[UiUniform {
                screen_size: [width as f32, height as f32],
                _pad: [0.0; 2],
            }]),
        );
    }

    /// Render UI rectangles.
    ///
    /// # Errors
    ///
    /// Returns an error if buffer allocation fails.
    pub fn render<'pass>(
        &'pass mut self,
        pass: &mut wgpu::RenderPass<'pass>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rects: &[weaver_render::UiRect],
    ) -> Result<(), WgpuRenderError> {
        if rects.is_empty() {
            return Ok(());
        }
        let count = rects.len().min(MAX_RECTS);
        self.ensure_capacity(device, count);

        let raw: Vec<UiRectRaw> = rects
            .iter()
            .take(count)
            .map(|r| UiRectRaw {
                position: [r.position.x, r.position.y],
                size: [r.size.x, r.size.y],
                background: r.background,
                border_color: r.border_color,
                border_width: r.border_width,
                corner_radius: r.corner_radius,
                layer: r.layer as f32,
                _pad: 0.0,
            })
            .collect();
        queue.write_buffer(&self.rect_buffer, 0, bytemuck::cast_slice(&raw));

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-uniform-bind-group"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform_buffer.as_entire_binding(),
            }],
        });
        let rect_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-rect-bind-group"),
            layout: &self.rect_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.rect_buffer.as_entire_binding(),
            }],
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &uniform_bind_group, &[]);
        pass.set_bind_group(1, &rect_bind_group, &[]);
        pass.draw(0..4, 0..count as u32);
        Ok(())
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, required: usize) {
        if required <= self.rect_capacity {
            return;
        }
        let new_capacity = (self.rect_capacity * 2).max(required).min(MAX_RECTS);
        if new_capacity == self.rect_capacity {
            return;
        }
        self.rect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-rects"),
            size: (new_capacity * std::mem::size_of::<UiRectRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.rect_capacity = new_capacity;
    }
}
