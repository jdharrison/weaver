//! GPU-owned mesh and texture resources.

use crate::error::WgpuRenderError;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// A vertex with position, normal, color, and UV.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// Position.
    pub position: [f32; 3],
    /// Normal.
    pub normal: [f32; 3],
    /// Color.
    pub color: [f32; 4],
    /// UV coordinates.
    pub uv: [f32; 2],
}

const VERTEX_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x4,
    3 => Float32x2,
];

impl Vertex {
    /// Vertex buffer layout expected by the mesh shader.
    #[must_use]
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBS,
        }
    }
}

/// A mesh stored on the GPU.
#[derive(Clone)]
pub struct GpuMesh {
    /// Vertex buffer.
    pub vertex_buffer: wgpu::Buffer,
    /// Index buffer.
    pub index_buffer: wgpu::Buffer,
    /// Number of indices.
    pub index_count: u32,
}

impl GpuMesh {
    /// Upload a mesh to the GPU.
    ///
    /// # Errors
    ///
    /// Returns an error if buffer creation fails.
    pub fn new(
        device: &wgpu::Device,
        vertices: &[Vertex],
        indices: &[u16],
    ) -> Result<Self, WgpuRenderError> {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
        })
    }
}

/// A texture stored on the GPU.
pub struct GpuTexture {
    /// Texture handle.
    pub texture: wgpu::Texture,
    /// Texture view.
    pub view: wgpu::TextureView,
    /// Sampler.
    pub sampler: wgpu::Sampler,
    /// Dimensions.
    pub size: (u32, u32),
}

impl GpuTexture {
    /// Upload an RGBA8 image to the GPU.
    ///
    /// # Errors
    ///
    /// Returns an error if texture creation fails.
    pub fn from_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<Self, WgpuRenderError> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu-texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            texture,
            view,
            sampler,
            size: (width, height),
        })
    }
}
