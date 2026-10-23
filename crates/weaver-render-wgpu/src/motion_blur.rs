//! Current-frame-only camera reprojection. No color history is retained.
use crate::error::WgpuRenderError;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CameraHistory {
    pub view_projection: Mat4,
    pub projection: Mat4,
}

#[derive(Default)]
pub(crate) struct History {
    committed: Option<CameraHistory>,
    generation: u64,
}

impl History {
    pub fn reset(&mut self) {
        self.committed = None;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn pending(&self, camera: Option<CameraHistory>) -> (u64, Option<CameraHistory>) {
        (self.generation, camera)
    }

    pub fn commit(&mut self, pending: (u64, Option<CameraHistory>)) {
        if pending.0 == self.generation {
            self.committed = pending.1;
        }
    }

    pub fn previous(&self, current: CameraHistory, reset: bool) -> Option<Mat4> {
        self.committed
            .filter(|old| !reset && old.projection == current.projection)
            .map(|old| old.view_projection)
    }
}

pub(crate) fn valid_matrix(matrix: Mat4) -> bool {
    matrix.is_finite()
        && matrix.determinant().is_finite()
        && matrix.determinant() != 0.0
        && matrix.inverse().is_finite()
}

fn shutter_scale(exposure: f32, dt: f32, has_history: bool) -> f32 {
    if !has_history || !dt.is_finite() || dt <= 0.0 || dt > 0.1 {
        return 0.0;
    }
    let scale = exposure / dt;
    if scale.is_finite() { scale } else { 0.0 }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    inverse_current: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
    settings: [f32; 4],
}

pub(crate) fn check_support(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
) -> Result<(), WgpuRenderError> {
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let depth = adapter.get_texture_format_features(wgpu::TextureFormat::Depth32Float);
    let color = adapter.get_texture_format_features(format);
    let downlevel = adapter.get_downlevel_capabilities();
    if !depth.allowed_usages.contains(usage)
        || !color.allowed_usages.contains(usage)
        || !depth.flags.sample_count_supported(samples)
        || !color.flags.sample_count_supported(samples)
        || (samples > 1
            && !color
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
        || !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
                | wgpu::TextureFormat::Rgba16Float
        )
        || (samples > 1
            && !downlevel
                .flags
                .contains(wgpu::DownlevelFlags::MULTISAMPLED_SHADING))
        || device.limits().max_sampled_textures_per_shader_stage < 2
    {
        return Err(WgpuRenderError::UnsupportedMotionBlur);
    }
    Ok(())
}

fn shader_source(samples: u32) -> String {
    let (kind, load) = if samples == 1 {
        (
            "texture_depth_2d",
            "return textureLoad(depth, p, 0);".to_owned(),
        )
    } else {
        (
            "texture_depth_multisampled_2d",
            format!(
                "var nearest = 1.0; for (var s = 0; s < {samples}; s += 1) {{ nearest = min(nearest, textureLoad(depth, p, s)); }} return nearest;"
            ),
        )
    };
    include_str!("motion_blur.wgsl")
        .replace("DEPTH_TYPE", kind)
        .replace("DEPTH_LOAD", &load)
}

pub(crate) struct MotionBlur {
    pub color: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bindings: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    size: (u32, u32),
}

pub(crate) fn target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    samples: u32,
    texture_binding: bool,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("motion-blur-target"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if texture_binding {
                    wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl MotionBlur {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        samples: u32,
    ) -> Self {
        let color = target(device, format, size, 1, true);
        let depth = target(
            device,
            wgpu::TextureFormat::Depth32Float,
            size,
            samples,
            true,
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("motion-blur-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: samples > 1,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion-blur-layout"),
            entries: &entries,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion-blur-bindings"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("camera-motion-blur"),
            source: wgpu::ShaderSource::Wgsl(shader_source(samples).into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion-blur-pipeline-layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("camera-motion-blur"),
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
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self {
            color,
            depth,
            pipeline,
            bindings,
            uniform,
            size,
        }
    }

    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        current: Mat4,
        previous: Option<Mat4>,
        settings: weaver_render::CameraMotionBlur,
    ) {
        // Keep the ratio finite even for a valid but subnormal positive dt.
        let scale = shutter_scale(
            settings.exposure_seconds,
            settings.frame_delta_seconds,
            previous.is_some(),
        );
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Params {
                inverse_current: current.inverse().to_cols_array_2d(),
                previous: previous.unwrap_or(current).to_cols_array_2d(),
                settings: [
                    self.size.0 as f32,
                    self.size.1 as f32,
                    scale,
                    settings.max_blur_pixels,
                ],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("camera-motion-blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4};

    fn camera(x: f32) -> CameraHistory {
        let projection = Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0);
        CameraHistory {
            view_projection: projection * Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
            projection,
        }
    }

    #[test]
    fn history_is_transactional_and_cuts_invalidate_pending_frames() {
        let mut history = History::default();
        let a = camera(0.0);
        let b = camera(1.0);
        let discarded = history.pending(Some(a));
        assert_eq!(history.previous(b, false), None);
        history.commit(discarded);
        assert_eq!(history.previous(b, false), Some(a.view_projection));
        let _failed_or_dropped = history.pending(Some(b));
        assert_eq!(history.previous(b, false), Some(a.view_projection));
        assert_eq!(history.previous(b, true), None);
        let mut changed = b;
        changed.projection = Mat4::IDENTITY;
        assert_eq!(history.previous(changed, false), None);
        let pending = history.pending(Some(b));
        history.reset();
        history.commit(pending);
        assert_eq!(history.previous(a, false), None);
        history.commit(history.pending(Some(a)));
        history.commit(history.pending(None));
        assert_eq!(history.previous(b, false), None);
    }

    #[test]
    fn reprojection_covers_translation_rotation_and_stationary_camera() {
        let projection = camera(0.0).projection;
        let point = Vec4::new(0.2, 0.1, -5.0, 1.0);
        let old_clip = projection * point;
        for view in [
            Mat4::IDENTITY,
            Mat4::from_rotation_y(0.1),
            Mat4::from_translation(Vec3::X),
        ] {
            let current = projection * view;
            let clip = current * point;
            let ndc = clip / clip.w;
            let reconstructed = current.inverse() * ndc;
            let previous = projection * reconstructed;
            assert!(
                (previous / previous.w - old_clip / old_clip.w)
                    .abs()
                    .max_element()
                    < 0.00001
            );
            if view != Mat4::IDENTITY {
                assert!((ndc.x - previous.x / previous.w).abs() > 0.01);
            }
        }
        assert!(valid_matrix(projection));
        assert!(!valid_matrix(Mat4::ZERO));
        assert!(!valid_matrix(Mat4::from_cols_array(&[f32::NAN; 16])));
    }

    #[test]
    fn shutter_scales_exposure_and_disables_stalls_or_overflow() {
        assert_eq!(shutter_scale(1.0 / 240.0, 1.0 / 60.0, true), 0.25);
        assert_eq!(shutter_scale(0.0, 0.01, true), 0.0);
        assert_eq!(shutter_scale(0.01, 0.01, false), 0.0);
        assert_eq!(shutter_scale(0.01, 0.1001, true), 0.0);
        assert_eq!(shutter_scale(0.01, f32::from_bits(1), true), 0.0);
        assert_eq!(shutter_scale(0.01, f32::NAN, true), 0.0);
        assert!(shutter_scale(0.01, 0.1, true) > 0.0);
    }

    #[test]
    fn shader_variants_use_current_depth_and_bounded_work() {
        let single = shader_source(1);
        let multi = shader_source(4);
        assert!(single.contains("texture_depth_2d"));
        assert!(multi.contains("texture_depth_multisampled_2d"));
        assert!(multi.contains("s < 4"));
        assert!(multi.contains("min(nearest, textureLoad"));
        assert!(!single.contains("DEPTH_"));
        assert!(!multi.contains("DEPTH_"));
        assert!(single.contains("2.0, 16.0"));
        assert_eq!(std::mem::size_of::<Params>(), 144);
    }
}

#[cfg(test)]
#[path = "motion_blur_gpu_tests.rs"]
mod gpu;
