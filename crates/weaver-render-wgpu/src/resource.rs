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

fn rgba_byte_len(width: u32, height: u32) -> Result<usize, WgpuRenderError> {
    let length = width
        .checked_mul(4)
        .and_then(|row| usize::try_from(row).ok())
        .and_then(|row| row.checked_mul(height as usize))
        .filter(|&length| isize::try_from(length).is_ok())
        .ok_or_else(|| WgpuRenderError::Texture("RGBA image byte length overflows".into()))?;
    Ok(length)
}

fn validate_rgba(
    width: u32,
    height: u32,
    data_len: usize,
    dimension_limit: u32,
) -> Result<(), WgpuRenderError> {
    if width == 0 || height == 0 || width > dimension_limit || height > dimension_limit {
        return Err(WgpuRenderError::Texture(
            "RGBA image dimensions are zero or exceed the device limit".into(),
        ));
    }
    let expected = rgba_byte_len(width, height)?;
    if data_len != expected {
        return Err(WgpuRenderError::Texture(format!(
            "RGBA image requires {expected} bytes, received {data_len}"
        )));
    }
    Ok(())
}

fn mip_level_count(width: u32, height: u32) -> u32 {
    u32::BITS - width.max(height).leading_zeros()
}

fn next_mip_size(width: u32, height: u32) -> (u32, u32) {
    ((width / 2).max(1), (height / 2).max(1))
}

fn srgb_decode_table() -> [f64; 256] {
    std::array::from_fn(|value| {
        let encoded = value as f64 / 255.0;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn encode_srgb(linear: f64) -> u8 {
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

// Area filtering includes odd trailing rows/columns without over-weighting them.
// Bounds use integer coordinates scaled by the destination dimension, avoiding
// floating-point boundary drift. RGB is accumulated premultiplied in linear
// light, then unpremultiplied for the renderer's straight-alpha blend contract.
fn downsample_rgba(
    width: u32,
    height: u32,
    data: &[u8],
    linear: &[f64; 256],
) -> Result<Vec<u8>, WgpuRenderError> {
    validate_rgba(width, height, data.len(), u32::MAX)?;
    let (next_width, next_height) = next_mip_size(width, height);
    let length = rgba_byte_len(next_width, next_height)?;
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|error| {
        WgpuRenderError::Texture(format!("RGBA mip allocation failed: {error}"))
    })?;
    let (width, height) = (u64::from(width), u64::from(height));
    let (next_width, next_height) = (u64::from(next_width), u64::from(next_height));
    for y in 0..next_height {
        let (top, bottom) = (y * height, (y + 1) * height);
        for x in 0..next_width {
            let (left, right) = (x * width, (x + 1) * width);
            let mut rgb = [0.0; 3];
            let mut alpha = 0.0;
            for source_y in top / next_height..bottom.div_ceil(next_height) {
                let overlap_y =
                    bottom.min((source_y + 1) * next_height) - top.max(source_y * next_height);
                for source_x in left / next_width..right.div_ceil(next_width) {
                    let overlap_x =
                        right.min((source_x + 1) * next_width) - left.max(source_x * next_width);
                    let weight = (overlap_x * overlap_y) as f64;
                    let index = ((source_y * width + source_x) * 4) as usize;
                    let weighted_alpha = weight * f64::from(data[index + 3]);
                    alpha += weighted_alpha;
                    for channel in 0..3 {
                        rgb[channel] += linear[usize::from(data[index + channel])] * weighted_alpha;
                    }
                }
            }
            let encoded_alpha = (alpha / (width * height) as f64).round() as u8;
            for channel in rgb {
                output.push(if encoded_alpha == 0 {
                    0
                } else {
                    encode_srgb(channel / alpha)
                });
            }
            output.push(encoded_alpha);
        }
    }
    Ok(output)
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
    /// Upload a straight-alpha sRGB RGBA8 image with a complete static mip chain.
    ///
    /// # Errors
    ///
    /// Returns a texture error for invalid dimensions, byte length, or mip allocation.
    pub fn from_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<Self, WgpuRenderError> {
        Self::from_rgba_with_anisotropy(device, queue, width, height, data, 1)
    }

    /// Upload a static straight-alpha sRGB image with trilinear mip filtering.
    ///
    /// `anisotropy` is clamped to 1..=16. The caller must pass 1 when the
    /// adapter does not support anisotropic filtering.
    ///
    /// # Errors
    ///
    /// Returns a texture error for invalid dimensions, byte length, or mip allocation.
    pub fn from_rgba_with_anisotropy(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        data: &[u8],
        anisotropy: u16,
    ) -> Result<Self, WgpuRenderError> {
        validate_rgba(
            width,
            height,
            data.len(),
            device.limits().max_texture_dimension_2d,
        )?;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu-texture"),
            size,
            mip_level_count: mip_level_count(width, height),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let linear = srgb_decode_table();
        // Borrow the original pixels; only adjacent generated levels coexist.
        let mut pixels = std::borrow::Cow::Borrowed(data);
        let (mut mip_width, mut mip_height) = (width, height);
        for mip_level in 0..mip_level_count(width, height) {
            let mut destination = texture.as_image_copy();
            destination.mip_level = mip_level;
            queue.write_texture(
                destination,
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * mip_width),
                    rows_per_image: Some(mip_height),
                },
                wgpu::Extent3d {
                    width: mip_width,
                    height: mip_height,
                    depth_or_array_layers: 1,
                },
            );
            if mip_width == 1 && mip_height == 1 {
                break;
            }
            let next = downsample_rgba(mip_width, mip_height, &pixels, &linear)?;
            pixels = std::borrow::Cow::Owned(next);
            (mip_width, mip_height) = next_mip_size(mip_width, mip_height);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&static_sampler_descriptor(anisotropy));
        Ok(Self {
            texture,
            view,
            sampler,
            size: (width, height),
        })
    }
}

fn static_sampler_descriptor(anisotropy: u16) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: anisotropy.clamp(1, 16),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_sampler_preserves_high_quality_and_bounds_requests() {
        for (requested, expected) in [(0, 1), (1, 1), (8, 8), (16, 16), (17, 16), (u16::MAX, 16)] {
            let sampler = static_sampler_descriptor(requested);
            assert_eq!(sampler.anisotropy_clamp, expected);
            assert_eq!(sampler.mag_filter, wgpu::FilterMode::Linear);
            assert_eq!(sampler.min_filter, wgpu::FilterMode::Linear);
            assert_eq!(sampler.mipmap_filter, wgpu::FilterMode::Linear);
        }
    }

    fn mip(width: u32, height: u32, data: &[u8]) -> Vec<u8> {
        downsample_rgba(width, height, data, &srgb_decode_table()).unwrap()
    }

    #[test]
    fn opaque_black_white_average_in_linear_light() {
        assert_eq!(
            mip(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255]),
            [188, 188, 188, 255]
        );
        for value in 0..=255 {
            assert_eq!(encode_srgb(srgb_decode_table()[value]), value as u8);
        }
    }

    #[test]
    fn transparent_pixels_neither_darken_nor_bleed() {
        for hidden_rgb in [[0, 0, 0], [0, 255, 0], [255, 255, 255]] {
            let pixels = [
                255,
                0,
                0,
                255,
                hidden_rgb[0],
                hidden_rgb[1],
                hidden_rgb[2],
                0,
            ];
            assert_eq!(mip(2, 1, &pixels), [255, 0, 0, 128]);
        }
        assert_eq!(mip(2, 1, &[255, 0, 0, 0, 0, 255, 0, 0]), [0; 4]);
        // A mip rounded to zero alpha must not carry hidden color onward.
        assert_eq!(
            mip(3, 1, &[255, 0, 0, 1, 0, 255, 0, 0, 0, 0, 255, 0]),
            [0; 4]
        );
    }

    #[test]
    fn partial_alpha_weights_rgb_but_output_is_straight_alpha() {
        assert_eq!(
            mip(2, 1, &[255, 255, 255, 192, 0, 0, 0, 64]),
            [225, 225, 225, 128]
        );
        assert_eq!(
            mip(2, 1, &[80, 120, 200, 64, 80, 120, 200, 64]),
            [80, 120, 200, 64]
        );
    }

    #[test]
    fn npot_area_filter_covers_edges_and_split_texels() {
        let mut pixels = vec![0; 5 * 3 * 4];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        // The middle column straddles both destination pixels (1/5 each).
        for row in 0..3 {
            pixels[(row * 5 + 2) * 4..(row * 5 + 2) * 4 + 3].fill(255);
        }
        assert_eq!(mip(5, 3, &pixels), [124, 124, 124, 255, 124, 124, 124, 255]);
        // Each source texel, including the last row/column, contributes area.
        for source in 0..15 {
            let mut impulse = vec![0; 5 * 3 * 4];
            impulse[source * 4..source * 4 + 4].fill(255);
            let first = mip(5, 3, &impulse);
            assert!(first.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
            let terminal = mip(2, 1, &first);
            assert_eq!(&terminal[..3], &[255; 3]);
            assert_eq!(terminal[3], 17);
        }
    }

    #[test]
    fn complete_mip_sizes_end_at_one_by_one() {
        for (width, height, expected) in [
            (1, 1, vec![(1, 1)]),
            (1, 7, vec![(1, 7), (1, 3), (1, 1)]),
            (7, 1, vec![(7, 1), (3, 1), (1, 1)]),
            (7, 5, vec![(7, 5), (3, 2), (1, 1)]),
            (8, 4, vec![(8, 4), (4, 2), (2, 1), (1, 1)]),
        ] {
            let mut sizes = vec![(width, height)];
            let mut pixels = vec![255; rgba_byte_len(width, height).unwrap()];
            let (mut w, mut h) = (width, height);
            while (w, h) != (1, 1) {
                pixels = mip(w, h, &pixels);
                (w, h) = next_mip_size(w, h);
                assert_eq!(pixels.len(), rgba_byte_len(w, h).unwrap());
                assert!(pixels.iter().all(|&byte| byte == 255));
                sizes.push((w, h));
            }
            assert_eq!(sizes, expected);
            assert_eq!(sizes.len(), mip_level_count(width, height) as usize);
        }
    }

    #[test]
    fn invalid_dimensions_and_lengths_are_texture_errors() {
        assert!(validate_rgba(1, 1, 4, 1).is_ok());
        assert!(validate_rgba(5, 3, 60, 5).is_ok());
        for (width, height, length, limit) in [
            (0, 1, 0, 8),
            (1, 0, 0, 8),
            (9, 1, 36, 8),
            (1, 9, 36, 8),
            (1, 1, 3, 8),
            (1, 1, 5, 8),
            (u32::MAX, 1, 0, u32::MAX),
            (u32::MAX / 4, u32::MAX, 0, u32::MAX),
        ] {
            assert!(matches!(
                validate_rgba(width, height, length, limit),
                Err(WgpuRenderError::Texture(_))
            ));
        }
        assert!(matches!(
            downsample_rgba(0, 0, &[], &srgb_decode_table()),
            Err(WgpuRenderError::Texture(_))
        ));
        assert!(matches!(
            downsample_rgba(2, 2, &[0; 4], &srgb_decode_table()),
            Err(WgpuRenderError::Texture(_))
        ));
    }
}
