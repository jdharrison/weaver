//! Mesh pipeline and instance buffer management.

use crate::error::WgpuRenderError;
use crate::pipeline::{
    MESH_SHADER, camera_bind_group_layout, create_shader, instance_uniform_layout,
};

const MAX_INSTANCES: usize = 200;
use crate::resource::{GpuMesh, Vertex};
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshInstanceRaw {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    emissive: f32,
    _pad: [f32; 3],
}

/// Mesh rendering pipeline and resources.
pub struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    _camera_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
}

impl MeshPipeline {
    /// Create the mesh pipeline.
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
        let shader = create_shader(device, "mesh-shader", MESH_SHADER);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh-pipeline-layout"),
            bind_group_layouts: &[&camera_layout, &instance_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout()],
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
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-instances"),
            size: (MAX_INSTANCES * std::mem::size_of::<MeshInstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            _camera_layout: camera_layout,
            instance_layout,
            instance_buffer,
            instance_capacity: MAX_INSTANCES,
        })
    }

    /// Render mesh instances from a snapshot.
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
        meshes: &[(GpuMesh, Vec<weaver_render::MeshInstance>)],
    ) -> Result<(), WgpuRenderError> {
        let total_instances: usize = meshes.iter().map(|(_, inst)| inst.len()).sum();
        if total_instances == 0 {
            return Ok(());
        }
        let total_instances = total_instances.min(MAX_INSTANCES);
        self.ensure_capacity(device, total_instances);

        let mut raw = Vec::with_capacity(total_instances);
        'collect: for (_, instances) in meshes {
            for instance in instances {
                if raw.len() >= total_instances {
                    break 'collect;
                }
                let model = instance.transform.model_matrix();
                raw.push(MeshInstanceRaw {
                    model: model.to_cols_array_2d(),
                    color: instance.color,
                    emissive: instance.emissive,
                    _pad: [0.0; 3],
                });
            }
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&raw));

        let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh-instance-bind-group"),
            layout: &self.instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.instance_buffer.as_entire_binding(),
            }],
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, &instance_bind_group, &[]);

        let mut instance_offset = 0;
        for (mesh, instances) in meshes {
            let count = instances.len() as u32;
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(
                0..mesh.index_count,
                0,
                instance_offset..instance_offset + count,
            );
            instance_offset += count;
        }
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
            label: Some("mesh-instances"),
            size: (new_capacity * std::mem::size_of::<MeshInstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }
}
