use super::*;
use crate::{SkinVertex, WgpuContext, WgpuContextConfig};
use glam::{Mat4, Vec3};
use weaver_render::{MeshHandle, MeshLighting, RenderTransform};
use wgpu::util::DeviceExt;

#[test]
fn skinned_shader_layout_and_portable_uniform_budget() {
    let source = crate::pipeline::skinned_mesh_shader();
    let shader = wgpu::naga::front::wgsl::parse_str(&source).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&shader)
    .unwrap();
    let palette = shader
        .global_variables
        .iter()
        .map(|(_, v)| v)
        .find(|v| v.name.as_deref() == Some("joints"))
        .unwrap();
    assert_eq!(
        palette.binding,
        Some(wgpu::naga::ResourceBinding {
            group: 3,
            binding: 0
        })
    );
    let wgpu::naga::TypeInner::Array { size, stride, .. } = shader.types[palette.ty].inner else {
        panic!()
    };
    assert_eq!(stride, 64);
    assert_eq!(
        size,
        wgpu::naga::ArraySize::Constant(
            std::num::NonZeroU32::new(weaver_render::mesh::MAX_SKIN_JOINTS as u32).unwrap()
        )
    );
    assert!(stride as usize * weaver_render::mesh::MAX_SKIN_JOINTS <= 16384);
    assert_eq!(size_of::<SkinVertex>(), 48);
    let layout = SkinVertex::layout();
    assert_eq!(layout.array_stride, 48);
    assert_eq!(
        layout
            .attributes
            .iter()
            .map(|a| a.offset)
            .collect::<Vec<_>>(),
        [0, 8, 16, 32]
    );
    assert_eq!(
        layout
            .attributes
            .iter()
            .map(|a| a.shader_location)
            .collect::<Vec<_>>(),
        [4, 5, 6, 7]
    );
    assert!(source.contains("cofactor * input.normal"));
    assert!(source.contains("normalize(in.normal)"));
}

#[test]
#[ignore = "opt-in local headless GPU; no window or network"]
fn headless_skin_palettes_survive_multiple_draws_and_reuse() {
    let context = pollster::block_on(WgpuContext::headless(WgpuContextConfig::default()))
        .expect("headless GPU required");
    let device = &context.device;
    let queue = &context.queue;
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pipeline = MeshPipeline::new(device, wgpu::TextureFormat::Rgba8Unorm, 1, 1).unwrap();
    let vertices = [[-0.3, -0.6, 0.5], [0.3, -0.6, 0.5], [0.0, 0.6, 0.5]].map(|position| Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        color: [1.0; 4],
        uv: [0.0; 2],
    });
    // Only the eighth influence moves geometry; a four-weight implementation fails.
    let stream = [SkinVertex {
        joints: [0, 0, 0, 0, 0, 0, 0, 73],
        weights: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    }; 3];
    let mesh = GpuMesh::with_skin(device, &vertices, &[0, 1, 2], Some(&stream)).unwrap();
    let texture = GpuTexture::from_rgba(device, queue, 1, 1, &[255; 4]).unwrap();
    let camera_data: Vec<f32> = Mat4::IDENTITY
        .to_cols_array()
        .into_iter()
        .chain([0.0; 4])
        .collect();
    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&camera_data),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &camera_bind_group_layout(device),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.as_entire_binding(),
        }],
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: target.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 8192,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    // Exercise both blend pipelines while reusing the same palette/instance slots.
    for frame in 0..4 {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for draw in 0..2 {
            let mut palette = vec![Mat4::IDENTITY; 74];
            palette[73] =
                Mat4::from_translation(Vec3::new(if draw == 0 { -0.5 } else { 0.5 }, 0.0, 0.0));
            let mut instance = MeshInstance::new(MeshHandle::new(), RenderTransform::IDENTITY);
            instance.skin = Some(palette.into());
            instance.render_mode = if frame < 2 {
                MeshRenderMode::Surface
            } else {
                MeshRenderMode::Additive
            };
            let red = (draw + frame) % 2 == 0;
            instance.color = if red {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [0.0, 1.0, 0.0, 1.0]
            };
            instance.emissive = 1.0;
            instance.lighting = Some(MeshLighting {
                position: Vec3::ZERO,
                ambient: 0.0,
                intensity: 0.0,
            });
            let meshes = [(mesh.clone(), &texture, vec![instance])];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if draw == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: if draw == 0 {
                            wgpu::LoadOp::Clear(1.0)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pipeline
                .render(&mut pass, device, queue, &camera, &meshes)
                .unwrap();
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(32),
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        assert_eq!(pipeline.skin_slots.used, 2);
        pipeline.reset_instance_slots();
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(wgpu::PollType::Wait).unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let data = readback.slice(..).get_mapped_range();
        for (draw, x) in [16, 48].into_iter().enumerate() {
            let pixel = &data[16 * 256 + x * 4..16 * 256 + x * 4 + 4];
            assert_eq!(
                pixel,
                if (draw + frame) % 2 == 0 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 255, 0, 255]
                }
            );
        }
        drop(data);
        readback.unmap();
    }
    assert!(pollster::block_on(device.pop_error_scope()).is_none());
}
