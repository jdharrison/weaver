//! Mesh pipeline and instance buffer management.

use crate::error::WgpuRenderError;
use crate::pipeline::{
    MESH_SHADER, camera_bind_group_layout, create_shader, instance_uniform_layout,
};

// Keep the complete uniform binding within WebGL2's guaranteed 16 KiB limit.
const MAX_INSTANCES: usize = 128;
use crate::resource::{GpuMesh, GpuTexture, Vertex};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use weaver_render::{MeshHandle, MeshInstance, MeshTextureFit, SpriteHandle};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MeshSamplingMode {
    Repeat,
    ClampToEdge,
}

impl MeshSamplingMode {
    fn for_fit(fit: Option<MeshTextureFit>) -> Self {
        // Sampler selection must agree with the shader's validated opt-in flag.
        if mesh_texture_fit_data(fit)[2] > 0.5 {
            Self::ClampToEdge
        } else {
            Self::Repeat
        }
    }
}

type MeshGroups = HashMap<(MeshHandle, Option<SpriteHandle>, MeshSamplingMode), Vec<MeshInstance>>;

fn mesh_sampler_descriptor(
    sampling_mode: MeshSamplingMode,
    anisotropy: u16,
) -> wgpu::SamplerDescriptor<'static> {
    let (label, address_mode) = match sampling_mode {
        MeshSamplingMode::Repeat => ("mesh-repeat-sampler", wgpu::AddressMode::Repeat),
        MeshSamplingMode::ClampToEdge => ("mesh-contain-sampler", wgpu::AddressMode::ClampToEdge),
    };
    wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        address_mode_w: address_mode,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: anisotropy,
        ..Default::default()
    }
}

fn mesh_sampling_groups(
    instances: &[MeshInstance],
) -> impl Iterator<Item = (MeshSamplingMode, &[MeshInstance])> {
    // Also keep direct MeshPipeline callers safe when their input mixes modes.
    instances
        .chunk_by(|left, right| {
            MeshSamplingMode::for_fit(left.texture_fit)
                == MeshSamplingMode::for_fit(right.texture_fit)
        })
        .map(|group| (MeshSamplingMode::for_fit(group[0].texture_fit), group))
}

pub(crate) fn group_mesh_instances(instances: &[MeshInstance]) -> MeshGroups {
    let mut groups = MeshGroups::new();
    for instance in instances {
        groups
            .entry((
                instance.mesh,
                instance.texture,
                MeshSamplingMode::for_fit(instance.texture_fit),
            ))
            .or_default()
            .push(instance.clone());
    }
    groups
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshInstanceRaw {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    emissive: f32,
    ambient: f32,
    intensity: f32,
    lighting_enabled: f32,
    light_position: [f32; 4],
    // Unit-UV content scale (xy), contain enabled (z), padding (w).
    texture_fit: [f32; 4],
}

fn mesh_texture_fit_data(fit: Option<MeshTextureFit>) -> [f32; 4] {
    let unfitted = [1.0, 1.0, 0.0, 0.0];
    let Some(fit) = fit else {
        return unfitted;
    };
    if !fit.surface_aspect.is_finite()
        || !fit.content_aspect.is_finite()
        || fit.surface_aspect <= 0.0
        || fit.content_aspect <= 0.0
    {
        return unfitted;
    }
    // Divide smaller by larger to avoid overflow. Keep even extreme ratios
    // normal and positive so shader division survives GPU subnormal flushing.
    if fit.content_aspect < fit.surface_aspect {
        [
            (fit.content_aspect / fit.surface_aspect).max(f32::MIN_POSITIVE),
            1.0,
            1.0,
            0.0,
        ]
    } else {
        [
            1.0,
            (fit.surface_aspect / fit.content_aspect).max(f32::MIN_POSITIVE),
            1.0,
            0.0,
        ]
    }
}

impl From<&MeshInstance> for MeshInstanceRaw {
    fn from(instance: &MeshInstance) -> Self {
        let (light_position, ambient, intensity, lighting_enabled) =
            instance
                .lighting
                .map_or(([0.0; 4], 0.08, 1.0, 0.0), |lighting| {
                    (
                        lighting.position.extend(0.0).to_array(),
                        lighting.ambient,
                        lighting.intensity,
                        1.0,
                    )
                });
        Self {
            model: instance.transform.model_matrix().to_cols_array_2d(),
            color: instance.color,
            emissive: instance.emissive,
            ambient,
            intensity,
            lighting_enabled,
            light_position,
            texture_fit: mesh_texture_fit_data(instance.texture_fit),
        }
    }
}

const _: () = assert!(MAX_INSTANCES * std::mem::size_of::<MeshInstanceRaw>() <= 16 * 1024);

fn mesh_instance_batches(
    instances: &[MeshInstance],
) -> impl Iterator<Item = (&[MeshInstance], Vec<MeshInstanceRaw>)> {
    instances.chunks(MAX_INSTANCES).map(|chunk| {
        // The shader declares a fixed-size array, including in the final partial batch.
        let mut raw = vec![MeshInstanceRaw::zeroed(); MAX_INSTANCES];
        for (out, instance) in raw.iter_mut().zip(chunk) {
            *out = MeshInstanceRaw::from(instance);
        }
        (chunk, raw)
    })
}

/// Mesh rendering pipeline and resources.
pub struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    _camera_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    repeat_sampler: wgpu::Sampler,
    clamp_sampler: wgpu::Sampler,
}

impl MeshPipeline {
    /// Create the mesh pipeline with effective scene samples and static anisotropy.
    ///
    /// # Errors
    ///
    /// Returns an error if pipeline creation fails.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
        anisotropy: u16,
    ) -> Result<Self, WgpuRenderError> {
        let camera_layout = camera_bind_group_layout(device);
        let instance_layout = instance_uniform_layout(device);
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh-texture-layout"),
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
        let repeat_sampler = device.create_sampler(&mesh_sampler_descriptor(
            MeshSamplingMode::Repeat,
            anisotropy,
        ));
        let clamp_sampler = device.create_sampler(&mesh_sampler_descriptor(
            MeshSamplingMode::ClampToEdge,
            anisotropy,
        ));
        let shader = create_shader(device, "mesh-shader", MESH_SHADER);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh-pipeline-layout"),
            bind_group_layouts: &[&camera_layout, &instance_layout, &texture_layout],
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
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        Ok(Self {
            pipeline,
            _camera_layout: camera_layout,
            instance_layout,
            texture_layout,
            repeat_sampler,
            clamp_sampler,
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
        _queue: &wgpu::Queue,
        camera_bind_group: &'pass wgpu::BindGroup,
        meshes: &[(GpuMesh, &GpuTexture, Vec<MeshInstance>)],
    ) -> Result<(), WgpuRenderError> {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);

        for (mesh, texture, sampling_mode, instances) in
            meshes.iter().flat_map(|(mesh, texture, instances)| {
                mesh_sampling_groups(instances).map(move |(sampling_mode, instances)| {
                    (mesh, texture, sampling_mode, instances)
                })
            })
        {
            let sampler = match sampling_mode {
                MeshSamplingMode::Repeat => &self.repeat_sampler,
                MeshSamplingMode::ClampToEdge => &self.clamp_sampler,
            };
            let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh-texture-bind-group"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            pass.set_bind_group(2, &texture_bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            for (chunk, raw) in mesh_instance_batches(instances) {
                // Distinct storage prevents later batches overwriting earlier draws
                // before the command buffer is submitted, as in the sprite pipeline.
                let instance_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh-instances"),
                        contents: bytemuck::cast_slice(&raw),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mesh-instance-bind-group"),
                    layout: &self.instance_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: instance_buffer.as_entire_binding(),
                    }],
                });
                pass.set_bind_group(1, &instance_bind_group, &[]);
                pass.draw_indexed(0..mesh.index_count, 0, 0..chunk.len() as u32);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use weaver_render::{MeshLighting, RenderTransform};

    #[test]
    fn uniform_batches_preserve_128_129_and_160_instances() {
        for (count, expected_sizes) in [(128, vec![128]), (129, vec![128, 1]), (160, vec![128, 32])]
        {
            let mesh = MeshHandle::new();
            let instances: Vec<_> = (0..count)
                .map(|index| {
                    let mut instance = MeshInstance::new(mesh, RenderTransform::default());
                    instance.emissive = index as f32;
                    instance
                })
                .collect();
            let batches: Vec<_> = mesh_instance_batches(&instances).collect();
            assert_eq!(
                batches
                    .iter()
                    .map(|(chunk, _)| chunk.len())
                    .collect::<Vec<_>>(),
                expected_sizes
            );
            let mut next = 0;
            for (chunk, raw) in &batches {
                assert_eq!(raw.len(), MAX_INSTANCES);
                assert!(raw.len() * std::mem::size_of::<MeshInstanceRaw>() <= 16 * 1024);
                for (instance, encoded) in chunk.iter().zip(raw) {
                    assert!(std::ptr::eq(instance, &raw const instances[next]));
                    assert_eq!(encoded.emissive, next as f32);
                    assert_eq!(
                        bytemuck::bytes_of(encoded),
                        bytemuck::bytes_of(&MeshInstanceRaw::from(instance))
                    );
                    next += 1;
                }
                assert!(
                    bytemuck::cast_slice::<MeshInstanceRaw, u8>(&raw[chunk.len()..])
                        .iter()
                        .all(|byte| *byte == 0)
                );
            }
            assert_eq!(next, count);
            assert!(
                !std::ptr::eq(batches[0].1.as_ptr(), batches.last().unwrap().1.as_ptr())
                    || batches.len() == 1
            );
        }
        assert_eq!(mesh_instance_batches(&[]).count(), 0);
    }

    #[test]
    fn uniform_batches_preserve_multiple_mesh_and_texture_groups() {
        let mesh = MeshHandle::new();
        let other_mesh = MeshHandle::new();
        let texture = SpriteHandle::new();
        let instances: Vec<_> = [
            (mesh, None, 129),
            (mesh, Some(texture), 160),
            (other_mesh, None, 128),
        ]
        .into_iter()
        .flat_map(|(mesh, texture, count)| {
            (0..count).map(move |index| {
                let mut instance = MeshInstance::new(mesh, RenderTransform::default());
                instance.texture = texture;
                instance.emissive = index as f32;
                instance
            })
        })
        .collect();
        let groups = group_mesh_instances(&instances);
        assert_eq!(groups.len(), 3);
        let mut rendered = 0;
        for (key, group) in &groups {
            let mut next = 0;
            for (chunk, raw) in mesh_instance_batches(group) {
                assert!(chunk.len() <= MAX_INSTANCES);
                for (instance, encoded) in chunk.iter().zip(&raw) {
                    assert_eq!(
                        (instance.mesh, instance.texture, MeshSamplingMode::Repeat),
                        *key
                    );
                    assert_eq!(encoded.emissive, next as f32);
                    next += 1;
                }
            }
            assert_eq!(next, group.len());
            rendered += next;
        }
        assert_eq!(rendered, 417);
    }

    #[test]
    fn default_lighting_encoding_preserves_origin_light_and_emission() {
        let mut instance = MeshInstance::new(MeshHandle::new(), RenderTransform::default());
        instance.color = [0.2, 0.4, 0.6, 0.5];
        instance.emissive = 0.7;
        let raw = MeshInstanceRaw::from(&instance);
        assert_eq!(raw.light_position, [0.0; 4]);
        assert_eq!(raw.ambient, 0.08);
        assert_eq!(raw.intensity, 1.0);
        assert_eq!(raw.lighting_enabled, 0.0);
        assert_eq!(raw.texture_fit, [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(raw.color, instance.color);
        assert_eq!(raw.emissive, instance.emissive);
        assert_eq!(
            raw.model,
            instance.transform.model_matrix().to_cols_array_2d()
        );
    }

    #[test]
    fn independent_point_light_parameters_are_encoded_per_instance() {
        let mut instance = MeshInstance::new(MeshHandle::new(), RenderTransform::default());
        instance.emissive = 0.3;
        for (position, ambient, intensity) in [
            (glam::Vec3::new(0.0, 2.7, 2.0), 0.3, 0.9),
            (glam::Vec3::new(0.0, 2.7, -2.0), 0.02, 0.1),
        ] {
            instance.lighting = Some(MeshLighting {
                position,
                ambient,
                intensity,
            });
            let raw = MeshInstanceRaw::from(&instance);
            assert_eq!(raw.light_position, position.extend(0.0).to_array());
            assert_eq!(raw.ambient, ambient);
            assert_eq!(raw.intensity, intensity);
            assert_eq!(raw.lighting_enabled, 1.0);
            assert_eq!(raw.emissive, instance.emissive);
            assert_eq!(raw.color, instance.color);
        }
        instance.lighting = None;
        assert_eq!(MeshInstanceRaw::from(&instance).lighting_enabled, 0.0);
    }

    #[test]
    fn contain_fit_matches_canonical_aspects_and_centered_uv_bounds() {
        let surface_aspect = 2.39;
        for (content_aspect, expected_scale) in [
            (16.0 / 9.0, [1600.0 / 2151.0, 1.0]),
            (4.0 / 3.0, [400.0 / 717.0, 1.0]),
            (9.0 / 16.0, [900.0 / 3824.0, 1.0]),
            (2.39, [1.0, 1.0]),
            (2.76, [1.0, 239.0 / 276.0]),
            (32.0 / 9.0, [1.0, 2151.0 / 3200.0]),
        ] {
            let fit = mesh_texture_fit_data(Some(MeshTextureFit {
                surface_aspect,
                content_aspect,
            }));
            assert_eq!(&fit[2..], &[1.0, 0.0]);
            assert!((surface_aspect * fit[0] / fit[1] - content_aspect).abs() < 1e-6);
            for axis in 0..2 {
                let scale = fit[axis];
                assert!((scale - expected_scale[axis]).abs() < 1e-6);
                assert!(scale > 0.0 && scale <= 1.0);
                let image_uv = |uv: f32| (uv - 0.5) / scale + 0.5;
                let start = (1.0 - scale) * 0.5;
                let end = (1.0 + scale) * 0.5;
                assert!(image_uv(start).abs() < 1e-6);
                assert!((image_uv(end) - 1.0).abs() < 1e-6);
                assert_eq!(image_uv(0.5), 0.5);
                if scale < 1.0 {
                    assert!(image_uv(0.0) < 0.0);
                    assert!(image_uv(1.0) > 1.0);
                    assert!(image_uv(start - 0.001) < 0.0);
                    assert!(image_uv(end + 0.001) > 1.0);
                    assert!(image_uv(start + 0.001) > 0.0);
                    assert!(image_uv(end - 0.001) < 1.0);
                } else {
                    assert_eq!(image_uv(0.0), 0.0);
                    assert_eq!(image_uv(1.0), 1.0);
                }
            }
        }
    }

    #[test]
    fn absent_or_invalid_texture_fit_falls_back_without_changing_instance_data() {
        let mut instance = MeshInstance::new(MeshHandle::new(), RenderTransform::default());
        instance.color = [0.2, 0.4, 0.6, 0.5];
        instance.emissive = 0.7;
        instance.lighting = Some(MeshLighting {
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            ambient: 0.3,
            intensity: 0.9,
        });
        let unfitted = MeshInstanceRaw::from(&instance);
        assert_eq!(mesh_texture_fit_data(None), [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(MeshSamplingMode::for_fit(None), MeshSamplingMode::Repeat);
        for invalid in [0.0, -0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for (surface_aspect, content_aspect) in [(invalid, 2.39), (2.39, invalid)] {
                instance.texture_fit = Some(MeshTextureFit {
                    surface_aspect,
                    content_aspect,
                });
                let raw = MeshInstanceRaw::from(&instance);
                assert_eq!(bytemuck::bytes_of(&raw), bytemuck::bytes_of(&unfitted));
                assert_eq!(
                    MeshSamplingMode::for_fit(instance.texture_fit),
                    MeshSamplingMode::Repeat
                );
            }
        }
    }

    #[test]
    fn extreme_valid_texture_fit_scales_and_unit_uv_division_remain_finite() {
        for (surface_aspect, content_aspect) in [
            (f32::MAX, f32::MIN_POSITIVE),
            (f32::MIN_POSITIVE, f32::MAX),
            (f32::MAX, f32::from_bits(1)),
            (f32::from_bits(1), f32::MAX),
            (f32::MAX, f32::MAX),
            (f32::from_bits(1), f32::from_bits(1)),
        ] {
            let fit = mesh_texture_fit_data(Some(MeshTextureFit {
                surface_aspect,
                content_aspect,
            }));
            assert_eq!(&fit[2..], &[1.0, 0.0]);
            assert!(fit.iter().all(|value| value.is_finite()));
            for scale in &fit[..2] {
                assert!(*scale >= f32::MIN_POSITIVE && *scale <= 1.0);
                for uv in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    assert!(((uv - 0.5) / scale + 0.5).is_finite());
                }
            }
        }
    }

    #[test]
    fn same_texture_fitted_and_unfitted_instances_have_separate_sampling_groups() {
        let mesh = MeshHandle::new();
        let texture = SpriteHandle::new();
        let mut plain = MeshInstance::new(mesh, RenderTransform::default());
        plain.texture = Some(texture);
        let mut narrower = plain.clone();
        narrower.texture_fit = Some(MeshTextureFit {
            surface_aspect: 2.39,
            content_aspect: 4.0 / 3.0,
        });
        let mut wider = plain.clone();
        wider.texture_fit = Some(MeshTextureFit {
            surface_aspect: 2.39,
            content_aspect: 2.76,
        });
        let mut exact = plain.clone();
        exact.texture_fit = Some(MeshTextureFit {
            surface_aspect: 2.39,
            content_aspect: 2.39,
        });
        let mut invalid = plain.clone();
        invalid.texture_fit = Some(MeshTextureFit {
            surface_aspect: 2.39,
            content_aspect: 0.0,
        });
        let groups = group_mesh_instances(&[
            plain.clone(),
            narrower.clone(),
            invalid.clone(),
            wider.clone(),
            exact.clone(),
        ]);
        assert_eq!(groups.len(), 2);
        let repeating = &groups[&(mesh, Some(texture), MeshSamplingMode::Repeat)];
        let fitted = &groups[&(mesh, Some(texture), MeshSamplingMode::ClampToEdge)];
        assert_eq!(repeating, &[plain, invalid]);
        assert_eq!(fitted, &[narrower, wider, exact]);
        for (instances, sampling_mode, enabled) in [
            (repeating, MeshSamplingMode::Repeat, 0.0),
            (fitted, MeshSamplingMode::ClampToEdge, 1.0),
        ] {
            let sampling_groups: Vec<_> = mesh_sampling_groups(instances).collect();
            assert_eq!(sampling_groups.len(), 1);
            assert_eq!(sampling_groups[0].0, sampling_mode);
            let batches: Vec<_> = mesh_instance_batches(instances).collect();
            assert_eq!(batches.len(), 1);
            for (instance, raw) in instances.iter().zip(&batches[0].1) {
                assert_eq!(raw.texture_fit, mesh_texture_fit_data(instance.texture_fit));
                assert_eq!(raw.texture_fit[2], enabled);
                assert_eq!(raw.color, instance.color);
                assert_eq!(raw.emissive, instance.emissive);
            }
        }
    }

    #[test]
    fn direct_pipeline_sampling_groups_split_mixed_runs_without_reordering() {
        let plain = MeshInstance::new(MeshHandle::new(), RenderTransform::default());
        let mut fitted = plain.clone();
        fitted.texture_fit = Some(MeshTextureFit {
            surface_aspect: 2.39,
            content_aspect: 16.0 / 9.0,
        });
        let instances = [plain.clone(), fitted.clone(), fitted.clone(), plain, fitted];
        let groups: Vec<_> = mesh_sampling_groups(&instances).collect();
        assert_eq!(
            groups
                .iter()
                .map(|(mode, group)| (*mode, group.len()))
                .collect::<Vec<_>>(),
            [
                (MeshSamplingMode::Repeat, 1),
                (MeshSamplingMode::ClampToEdge, 2),
                (MeshSamplingMode::Repeat, 1),
                (MeshSamplingMode::ClampToEdge, 1),
            ]
        );
        assert_eq!(
            groups
                .iter()
                .flat_map(|(_, group)| group.iter())
                .collect::<Vec<_>>(),
            instances.iter().collect::<Vec<_>>()
        );
        assert_eq!(mesh_sampling_groups(&[]).count(), 0);
    }

    #[test]
    fn samplers_preserve_repeat_defaults_and_clamp_only_the_fitted_address_mode() {
        for anisotropy in [1, 4, 8] {
            let expected_repeat = wgpu::SamplerDescriptor {
                label: Some("mesh-repeat-sampler"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            };
            assert_eq!(
                mesh_sampler_descriptor(MeshSamplingMode::Repeat, anisotropy),
                expected_repeat
            );
            assert_eq!(
                mesh_sampler_descriptor(MeshSamplingMode::ClampToEdge, anisotropy),
                wgpu::SamplerDescriptor {
                    label: Some("mesh-contain-sampler"),
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    address_mode_w: wgpu::AddressMode::ClampToEdge,
                    ..expected_repeat
                }
            );
        }
    }

    #[test]
    fn shader_preserves_default_lighting_and_additive_emission() {
        assert!(MESH_SHADER.contains("var light_direction = -in.world_pos;"));
        assert!(MESH_SHADER.contains("var ambient = 0.08;"));
        assert!(MESH_SHADER.contains("var intensity = 1.0;"));
        assert!(MESH_SHADER.contains("if (in.lighting.z > 0.5)"));
        assert!(MESH_SHADER.contains("light_direction = in.light_position - in.world_pos;"));
        assert!(MESH_SHADER.contains("if (dot(light_direction, light_direction) > 0.0)"));
        assert!(MESH_SHADER.contains("let lit = color.rgb * (ambient + diff * intensity);"));
        assert!(MESH_SHADER.contains("let glow = color.rgb * in.emissive;"));
    }

    #[test]
    fn batches_distinguish_mesh_and_optional_texture() {
        let mesh = MeshHandle::new();
        let other_mesh = MeshHandle::new();
        let texture = SpriteHandle::new();
        let other_texture = SpriteHandle::new();
        let plain = MeshInstance::new(mesh, RenderTransform::default());
        let mut textured = plain.clone();
        textured.texture = Some(texture);
        let mut other_textured = plain.clone();
        other_textured.texture = Some(other_texture);
        let mut other = MeshInstance::new(other_mesh, RenderTransform::default());
        other.texture = Some(texture);
        let mut tinted = textured.clone();
        tinted.color = [0.5; 4];
        tinted.lighting = Some(MeshLighting {
            position: glam::Vec3::new(0.0, 2.7, 2.0),
            ambient: 0.2,
            intensity: 0.8,
        });
        let groups = group_mesh_instances(&[
            plain.clone(),
            textured.clone(),
            other_textured,
            other,
            tinted.clone(),
        ]);
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[&(mesh, None, MeshSamplingMode::Repeat)], vec![plain]);
        assert_eq!(
            groups[&(mesh, Some(texture), MeshSamplingMode::Repeat)],
            vec![textured, tinted]
        );
        assert_eq!(
            groups[&(mesh, Some(other_texture), MeshSamplingMode::Repeat)].len(),
            1
        );
        assert_eq!(
            groups[&(other_mesh, Some(texture), MeshSamplingMode::Repeat)].len(),
            1
        );
        assert!(group_mesh_instances(&[]).is_empty());
    }

    #[test]
    fn shader_preserves_vertex_instance_and_texture_alpha() {
        assert!(MESH_SHADER.contains("out.color = input.color * inst.color;"));
        assert!(MESH_SHADER.contains(
            "let color = textureSample(mesh_texture, mesh_sampler, image_uv) * in.color;"
        ));
        assert!(MESH_SHADER.contains("return vec4<f32>(lit + glow, color.a);"));
    }

    #[test]
    fn shader_contains_opt_in_centered_fit_and_opaque_black_bars() {
        assert!(MESH_SHADER.contains("@location(7) @interpolate(flat) texture_fit: vec3<f32>"));
        assert!(MESH_SHADER.contains("out.texture_fit = inst.texture_fit.xyz;"));
        assert!(MESH_SHADER.contains("var image_uv = in.uv;"));
        assert!(!MESH_SHADER.contains("half_texel"));
        assert!(!MESH_SHADER.contains("clamp(image_uv"));
        assert!(MESH_SHADER.contains("if (in.texture_fit.z > 0.5)"));
        assert!(
            MESH_SHADER.contains(
                "image_uv = (in.uv - vec2<f32>(0.5)) / in.texture_fit.xy + vec2<f32>(0.5);"
            )
        );
        assert!(MESH_SHADER.contains("if (in.texture_fit.z > 0.5 &&"));
        assert!(MESH_SHADER.contains("any(image_uv < vec2<f32>(0.0))"));
        assert!(MESH_SHADER.contains("any(image_uv > vec2<f32>(1.0))"));
        let sample = MESH_SHADER.find("let color = textureSample(").unwrap();
        let bars = MESH_SHADER
            .find("return vec4<f32>(0.0, 0.0, 0.0, 1.0);")
            .unwrap();
        assert!(
            sample < bars,
            "sample before divergent return for implicit derivatives"
        );
        assert!(bars < MESH_SHADER.find("let lit =").unwrap());
        assert_eq!(MESH_SHADER.matches("textureSample(").count(), 1);
        assert!(!MESH_SHADER.contains("discard"));
    }

    #[test]
    fn shader_layout_matches_rust_and_webgl2_uniform_limit() {
        let shader =
            wgpu::naga::front::wgsl::parse_str(MESH_SHADER).expect("valid textured mesh shader");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&shader)
        .expect("mesh shader uses no optional capabilities");
        let instances = shader
            .global_variables
            .iter()
            .map(|(_, variable)| variable)
            .find(|variable| variable.name.as_deref() == Some("instances"))
            .expect("instance uniform");
        let wgpu::naga::TypeInner::Array { base, size, stride } = shader.types[instances.ty].inner
        else {
            panic!("instances must be an array");
        };
        assert_eq!(std::mem::size_of::<MeshInstanceRaw>(), 128);
        assert_eq!(std::mem::align_of::<MeshInstanceRaw>(), 16);
        assert_eq!(MAX_INSTANCES, 128);
        assert_eq!(
            MAX_INSTANCES * std::mem::size_of::<MeshInstanceRaw>(),
            16_384
        );
        assert_eq!(stride as usize, std::mem::size_of::<MeshInstanceRaw>());
        assert_eq!(
            size,
            wgpu::naga::ArraySize::Constant(
                std::num::NonZeroU32::new(MAX_INSTANCES as u32).unwrap()
            )
        );
        let wgpu::naga::TypeInner::Struct { ref members, span } = shader.types[base].inner else {
            panic!("instance must be a struct");
        };
        assert_eq!(span as usize, std::mem::size_of::<MeshInstanceRaw>());
        assert_eq!(
            members[4].offset as usize,
            std::mem::offset_of!(MeshInstanceRaw, color)
        );
        assert_eq!(
            members[5].offset as usize,
            std::mem::offset_of!(MeshInstanceRaw, emissive)
        );
        for (index, name, offset) in [
            (6, "ambient", std::mem::offset_of!(MeshInstanceRaw, ambient)),
            (
                7,
                "intensity",
                std::mem::offset_of!(MeshInstanceRaw, intensity),
            ),
            (
                8,
                "lighting_enabled",
                std::mem::offset_of!(MeshInstanceRaw, lighting_enabled),
            ),
            (
                9,
                "light_position",
                std::mem::offset_of!(MeshInstanceRaw, light_position),
            ),
            (
                10,
                "texture_fit",
                std::mem::offset_of!(MeshInstanceRaw, texture_fit),
            ),
        ] {
            assert_eq!(members[index].name.as_deref(), Some(name));
            assert_eq!(members[index].offset as usize, offset);
        }
        assert!(MAX_INSTANCES * stride as usize <= 16 * 1024);
        for (name, binding) in [("mesh_texture", 0), ("mesh_sampler", 1)] {
            let variable = shader
                .global_variables
                .iter()
                .map(|(_, variable)| variable)
                .find(|variable| variable.name.as_deref() == Some(name))
                .expect("texture binding");
            assert_eq!(
                variable.binding,
                Some(wgpu::naga::ResourceBinding { group: 2, binding })
            );
        }
    }
}
