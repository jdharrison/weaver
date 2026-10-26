//! Mesh pipeline and instance buffer management.

use crate::error::WgpuRenderError;
use crate::pipeline::{
    MESH_SHADER, WATER_SHADER, camera_bind_group_layout, create_shader, instance_uniform_layout,
};

// Keep the complete uniform binding within WebGL2's guaranteed 16 KiB limit.
const MAX_INSTANCES: usize = 112;
use crate::resource::{GpuMesh, GpuTexture, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::HashMap;
use weaver_render::{
    MeshHandle, MeshInstance, MeshRenderMode, MeshTextureFit, SpriteHandle, WaterSettings,
};

// Retain transient peaks briefly, not for the lifetime of the renderer.
const RESOURCE_IDLE_FRAMES: u16 = 120;
const INSTANCE_BUFFER_SIZE: u64 = (MAX_INSTANCES * std::mem::size_of::<MeshInstanceRaw>()) as u64;
const WATER_UNIFORM_SIZE: u64 = 128;

struct InstanceSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct DrawSlots<T> {
    slots: Vec<(T, u16)>,
    used: usize,
}

impl<T> Default for DrawSlots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            used: 0,
        }
    }
}

impl<T> DrawSlots<T> {
    fn next(&mut self, create: impl FnOnce() -> T) -> &T {
        if self.used == self.slots.len() {
            self.slots.push((create(), 0));
        }
        let index = self.used;
        self.used += 1;
        self.slots[index].1 = 0;
        &self.slots[index].0
    }

    fn reset(&mut self) {
        for (_, idle) in &mut self.slots[self.used..] {
            *idle = idle.saturating_add(1);
        }
        while self
            .slots
            .last()
            .is_some_and(|(_, idle)| *idle >= RESOURCE_IDLE_FRAMES)
        {
            self.slots.pop();
        }
        self.used = 0;
    }
}

// Key by the actual resolved view, not the logical asset handle or its address.
// Replacing an override/view (including fallback white) cannot hit an old entry.
struct TextureBindings<K, V> {
    entries: HashMap<(K, K, K, MeshSamplingMode), (V, u16)>,
}

impl<K: Eq + std::hash::Hash, V> TextureBindings<K, V> {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn invalidate(&mut self, view: &K) {
        self.entries
            .retain(|(color, normal, mr, _), _| color != view && normal != view && mr != view);
    }

    fn age(&mut self) {
        self.entries.retain(|_, (_, idle)| {
            *idle = idle.saturating_add(1);
            *idle < RESOURCE_IDLE_FRAMES
        });
    }

    fn get_or_insert(
        &mut self,
        views: (K, K, K),
        mode: MeshSamplingMode,
        create: impl FnOnce() -> V,
    ) -> &V {
        let (binding, idle) = self
            .entries
            .entry((views.0, views.1, views.2, mode))
            .or_insert_with(|| (create(), 0));
        *idle = 0;
        binding
    }
}

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

type MeshGroups = HashMap<
    (
        MeshHandle,
        Option<SpriteHandle>,
        MeshSamplingMode,
        MeshRenderMode,
        Option<SpriteHandle>,
        Option<SpriteHandle>,
    ),
    Vec<MeshInstance>,
>;

fn mesh_blend_state(mode: MeshRenderMode) -> wgpu::BlendState {
    match mode {
        MeshRenderMode::Surface => wgpu::BlendState::ALPHA_BLENDING,
        MeshRenderMode::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        MeshRenderMode::Water => wgpu::BlendState::REPLACE,
    }
}

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
            left.render_mode == right.render_mode
                && left.normal_texture == right.normal_texture
                && left.metallic_roughness_texture == right.metallic_roughness_texture
                && MeshSamplingMode::for_fit(left.texture_fit)
                    == MeshSamplingMode::for_fit(right.texture_fit)
        })
        .map(|group| (MeshSamplingMode::for_fit(group[0].texture_fit), group))
}

pub(crate) fn group_mesh_instances<'a>(
    instances: impl IntoIterator<Item = &'a MeshInstance>,
) -> MeshGroups {
    let mut groups = MeshGroups::new();
    for instance in instances {
        groups
            .entry((
                instance.mesh,
                instance.texture,
                MeshSamplingMode::for_fit(instance.texture_fit),
                instance.render_mode,
                instance.normal_texture,
                instance.metallic_roughness_texture,
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
    // Metallic, roughness, normal enabled, MR enabled.
    material: [f32; 4],
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
            material: [
                if instance.metallic.is_finite() {
                    instance.metallic.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                if instance.roughness.is_finite() {
                    instance.roughness.clamp(0.0, 1.0)
                } else {
                    1.0
                },
                f32::from(instance.normal_texture.is_some()),
                f32::from(instance.metallic_roughness_texture.is_some()),
            ],
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterUniformRaw {
    deep_color: [f32; 4],
    shallow_color: [f32; 4],
    horizon: [f32; 4],
    zenith: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    params: [f32; 4],
    params2: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<WaterUniformRaw>() as u64 <= WATER_UNIFORM_SIZE);

const _: () = assert!(MAX_INSTANCES * std::mem::size_of::<MeshInstanceRaw>() <= 16 * 1024);

#[cfg(test)]
fn mesh_instance_batches(
    instances: &[MeshInstance],
) -> impl Iterator<Item = (&[MeshInstance], Vec<MeshInstanceRaw>)> {
    instances.chunks(MAX_INSTANCES).map(|chunk| {
        // Only active records are uploaded; the GPU binding remains the full array.
        let raw = chunk.iter().map(MeshInstanceRaw::from).collect();
        (chunk, raw)
    })
}

/// Mesh rendering pipeline and resources.
pub struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    skinned_pipeline: wgpu::RenderPipeline,
    skin_layout: wgpu::BindGroupLayout,
    skin_slots: DrawSlots<InstanceSlot>,
    additive_pipeline: wgpu::RenderPipeline,
    skinned_additive_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    water_bind_group: wgpu::BindGroup,
    water_buffer: wgpu::Buffer,
    _camera_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    repeat_sampler: wgpu::Sampler,
    clamp_sampler: wgpu::Sampler,
    instance_slots: DrawSlots<InstanceSlot>,
    texture_bindings: TextureBindings<wgpu::TextureView, wgpu::BindGroup>,
    pub(crate) draw_stats: crate::stats::MeshDrawStats,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
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
        let skin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin-palette-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(8192),
                },
                count: None,
            }],
        });
        let skinned_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skinned-mesh-layout"),
            bind_group_layouts: &[
                &camera_layout,
                &instance_layout,
                &texture_layout,
                &skin_layout,
            ],
            push_constant_ranges: &[],
        });
        let water_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(WATER_UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("water-mesh-layout"),
                bind_group_layouts: &[
                    &camera_layout,
                    &instance_layout,
                    &texture_layout,
                    &water_layout,
                ],
                push_constant_ranges: &[],
            });
        let water_shader = create_shader(device, "water-shader", WATER_SHADER);
        let water_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water-uniform"),
            size: WATER_UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let water_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water-uniform-bind-group"),
            layout: &water_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: water_buffer.as_entire_binding(),
            }],
        });
        let skinned_shader = create_shader(
            device,
            "skinned-mesh-shader",
            &crate::pipeline::skinned_mesh_shader(),
        );
        let make_pipeline = |layout: &wgpu::PipelineLayout,
                             shader: &wgpu::ShaderModule,
                             buffers: &[wgpu::VertexBufferLayout<'_>],
                             mode: MeshRenderMode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh-pipeline"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(mesh_blend_state(mode)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // Skinning may reflect geometry during animation. Keep rigid
                    // culling unchanged; skinned geometry renders both windings.
                    cull_mode: if buffers.len() > 1
                        || mode == MeshRenderMode::Additive
                        || mode == MeshRenderMode::Water
                    {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    front_face: wgpu::FrontFace::Ccw,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: mode == MeshRenderMode::Surface
                        || mode == MeshRenderMode::Water,
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
            })
        };

        let pipeline = make_pipeline(
            &pipeline_layout,
            &shader,
            &[Vertex::layout()],
            MeshRenderMode::Surface,
        );
        let additive_pipeline = make_pipeline(
            &pipeline_layout,
            &shader,
            &[Vertex::layout()],
            MeshRenderMode::Additive,
        );
        let skinned_additive_pipeline = make_pipeline(
            &skinned_layout,
            &skinned_shader,
            &[Vertex::layout(), crate::SkinVertex::layout()],
            MeshRenderMode::Additive,
        );
        let skinned_pipeline = make_pipeline(
            &skinned_layout,
            &skinned_shader,
            &[Vertex::layout(), crate::SkinVertex::layout()],
            MeshRenderMode::Surface,
        );
        let water_pipeline = make_pipeline(
            &water_pipeline_layout,
            &water_shader,
            &[Vertex::layout()],
            MeshRenderMode::Water,
        );
        Ok(Self {
            pipeline,
            skinned_pipeline,
            additive_pipeline,
            skinned_additive_pipeline,
            water_pipeline,
            water_bind_group,
            water_buffer,
            skin_layout,
            skin_slots: DrawSlots::default(),
            _camera_layout: camera_layout,
            instance_layout,
            texture_layout,
            repeat_sampler,
            clamp_sampler,
            instance_slots: DrawSlots::default(),
            texture_bindings: TextureBindings::new(),
            draw_stats: crate::stats::MeshDrawStats::default(),
        })
    }

    /// Update the stylized water uniform. Invalid inputs fall back to defaults.
    pub fn set_water(
        &mut self,
        queue: &wgpu::Queue,
        settings: Option<WaterSettings>,
        presentation_time: f32,
    ) {
        let settings = settings.filter(WaterSettings::is_valid).unwrap_or_default();
        let finite = |value: Vec3, fallback: Vec3| {
            if value.is_finite() { value } else { fallback }
        };
        let sun = finite(settings.sun_direction, Vec3::Y);
        let sun = if sun.length_squared() > 1.0e-6 {
            sun.normalize()
        } else {
            Vec3::Y
        };
        let finite_scalar = |value: f32, fallback: f32| {
            if value.is_finite() { value } else { fallback }
        };
        let time = finite_scalar(presentation_time, 0.0);
        let raw = WaterUniformRaw {
            deep_color: finite(settings.deep_color, Vec3::ZERO)
                .extend(0.0)
                .to_array(),
            shallow_color: finite(settings.shallow_color, Vec3::ZERO)
                .extend(0.0)
                .to_array(),
            horizon: finite(settings.sky_horizon, Vec3::ZERO)
                .extend(0.0)
                .to_array(),
            zenith: finite(settings.sky_zenith, Vec3::ZERO)
                .extend(0.0)
                .to_array(),
            sun_direction: sun.extend(0.0).to_array(),
            sun_color: finite(settings.sun_color, Vec3::ONE).extend(0.0).to_array(),
            params: [
                time,
                finite_scalar(settings.specular_strength, 1.0).clamp(0.0, 8.0),
                finite_scalar(settings.wave_scale, 1.0).clamp(0.01, 16.0),
                finite_scalar(settings.wave_height, 0.5).clamp(0.0, 4.0),
            ],
            params2: [
                finite_scalar(settings.foam_strength, 0.3).clamp(0.0, 2.0),
                finite_scalar(settings.detail_strength, 0.3).clamp(0.0, 2.0),
                finite_scalar(settings.normal_strength, 2.5).clamp(0.0, 12.0),
                0.0,
            ],
        };
        queue.write_buffer(&self.water_buffer, 0, bytemuck::bytes_of(&raw));
    }

    /// Immediately release cached bindings for a removed/replaced texture view.
    /// Already encoded draws retain their own WGPU references. In-place pixel
    /// updates require no invalidation; new views cannot hit old cache entries.
    pub fn invalidate_texture(&mut self, texture: &GpuTexture) {
        self.texture_bindings.invalidate(&texture.view);
        if let Some(view) = &texture.linear_view {
            self.texture_bindings.invalidate(view);
        }
    }

    /// Reclaim instance slots after ALL previously encoded mesh draws have been
    /// submitted to the same queue, or their command buffers have been discarded.
    /// GPU completion is not required: subsequent queue writes execute after the
    /// earlier submission. Never reset between passes/draws of a pending submission.
    /// Surplus slots are released after 120 idle resets; active slots are retained.
    pub fn reset_instance_slots(&mut self) {
        self.instance_slots.reset();
        self.skin_slots.reset();
    }

    /// Render mesh instances from a snapshot.
    ///
    /// Calls append distinct instance slots until `reset_instance_slots` is called.
    /// Use the same device/queue for this pipeline's lifetime. Texture bindings
    /// follow resolved view identity and sampling mode; unused entries expire
    /// after 120 render calls (in-place pixel updates do not invalidate bindings).
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
        meshes: &[(
            GpuMesh,
            &GpuTexture,
            &GpuTexture,
            &GpuTexture,
            Vec<MeshInstance>,
        )],
    ) -> Result<(), WgpuRenderError> {
        self.draw_stats = crate::stats::MeshDrawStats::default();
        self.texture_bindings.age();
        pass.set_bind_group(0, camera_bind_group, &[]);

        // Finish opaque/water surfaces before additive draws, even for mixed-mode input.
        for mode in [
            MeshRenderMode::Surface,
            MeshRenderMode::Water,
            MeshRenderMode::Additive,
        ] {
            for (mesh, texture, normal, mr, sampling_mode, instances) in
                meshes
                    .iter()
                    .flat_map(|(mesh, texture, normal, mr, instances)| {
                        mesh_sampling_groups(instances)
                            .filter(move |(_, group)| group[0].render_mode == mode)
                            .map(move |(sampling_mode, instances)| {
                                (mesh, texture, normal, mr, sampling_mode, instances)
                            })
                    })
            {
                let sampler = match sampling_mode {
                    MeshSamplingMode::Repeat => &self.repeat_sampler,
                    MeshSamplingMode::ClampToEdge => &self.clamp_sampler,
                };
                let texture_bind_group = self.texture_bindings.get_or_insert(
                    (
                        texture.view.clone(),
                        normal.linear_view.as_ref().unwrap_or(&normal.view).clone(),
                        mr.linear_view.as_ref().unwrap_or(&mr.view).clone(),
                    ),
                    sampling_mode,
                    || {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::TextureView(
                                        normal.linear_view.as_ref().unwrap_or(&normal.view),
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(
                                        mr.linear_view.as_ref().unwrap_or(&mr.view),
                                    ),
                                },
                            ],
                        })
                    },
                );
                pass.set_bind_group(2, texture_bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                let skinned = mesh.skin_buffer.is_some();
                pass.set_pipeline(match (skinned, mode) {
                    (false, MeshRenderMode::Surface) => &self.pipeline,
                    (false, MeshRenderMode::Additive) => &self.additive_pipeline,
                    (false, MeshRenderMode::Water) => &self.water_pipeline,
                    (true, MeshRenderMode::Surface | MeshRenderMode::Water) => {
                        &self.skinned_pipeline
                    }
                    (true, MeshRenderMode::Additive) => &self.skinned_additive_pipeline,
                });
                if !skinned && mode == MeshRenderMode::Water {
                    pass.set_bind_group(3, &self.water_bind_group, &[]);
                }
                if let Some(buffer) = &mesh.skin_buffer {
                    pass.set_vertex_buffer(1, buffer.slice(..));
                }
                let batch_size = if skinned { 1 } else { MAX_INSTANCES };
                for chunk in instances.chunks(batch_size) {
                    let raw: Vec<_> = chunk.iter().map(MeshInstanceRaw::from).collect();
                    if skinned {
                        let palette = chunk[0].skin.as_ref().ok_or_else(|| {
                            WgpuRenderError::Buffer("skinned mesh has no palette".into())
                        })?;
                        if palette.len() < mesh.skin_joint_count
                            || palette.len() > weaver_render::mesh::MAX_SKIN_JOINTS
                            || palette.iter().any(|m| {
                                !m.is_finite()
                                    || !m.inverse().is_finite()
                                    || m.x_axis.w != 0.0
                                    || m.y_axis.w != 0.0
                                    || m.z_axis.w != 0.0
                                    || m.w_axis.w != 1.0
                            })
                        {
                            return Err(WgpuRenderError::Buffer("invalid skin palette".into()));
                        }
                        let slot = self.skin_slots.next(|| {
                            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some("skin-palette"),
                                size: 8192,
                                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                                mapped_at_creation: false,
                            });
                            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("skin-palette"),
                                layout: &self.skin_layout,
                                entries: &[wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: buffer.as_entire_binding(),
                                }],
                            });
                            InstanceSlot { buffer, bind_group }
                        });
                        let matrices: Vec<_> =
                            palette.iter().map(glam::Mat4::to_cols_array_2d).collect();
                        queue.write_buffer(&slot.buffer, 0, bytemuck::cast_slice(&matrices));
                        pass.set_bind_group(3, &slot.bind_group, &[]);
                    } else if chunk.iter().any(|instance| instance.skin.is_some()) {
                        return Err(WgpuRenderError::Buffer(
                            "palette supplied for rigid geometry".into(),
                        ));
                    }
                    let slot = self.instance_slots.next(|| {
                        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("mesh-instances"),
                            size: INSTANCE_BUFFER_SIZE,
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("mesh-instance-bind-group"),
                            layout: &self.instance_layout,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: buffer.as_entire_binding(),
                            }],
                        });
                        InstanceSlot { buffer, bind_group }
                    });
                    queue.write_buffer(&slot.buffer, 0, bytemuck::cast_slice(&raw));
                    pass.set_bind_group(1, &slot.bind_group, &[]);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..chunk.len() as u32);
                    self.draw_stats.record(mesh.index_count, chunk.len());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "skin_tests.rs"]
mod skin_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use weaver_render::{MeshLighting, RenderTransform};

    #[test]
    fn render_modes_separate_groups_without_changing_uniforms() {
        let surface = MeshInstance::new(MeshHandle::new(), RenderTransform::default());
        let mut additive = surface.clone();
        additive.render_mode = MeshRenderMode::Additive;
        assert_eq!(
            bytemuck::bytes_of(&MeshInstanceRaw::from(&surface)),
            bytemuck::bytes_of(&MeshInstanceRaw::from(&additive)),
        );
        let instances = [additive.clone(), surface.clone(), additive.clone()];
        let groups = group_mesh_instances(&instances);
        assert_eq!(groups.len(), 2);
        for mode in [MeshRenderMode::Surface, MeshRenderMode::Additive] {
            let group = &groups[&(
                surface.mesh,
                None,
                MeshSamplingMode::Repeat,
                mode,
                None,
                None,
            )];
            assert!(group.iter().all(|instance| instance.render_mode == mode));
            assert_eq!(
                group.len(),
                if mode == MeshRenderMode::Surface {
                    1
                } else {
                    2
                }
            );
        }
        let runs: Vec<_> = mesh_sampling_groups(&instances).collect();
        assert_eq!(runs.len(), 3);
        let ordered: Vec<_> = [MeshRenderMode::Surface, MeshRenderMode::Additive]
            .into_iter()
            .flat_map(|mode| {
                runs.iter()
                    .filter(move |(_, group)| group[0].render_mode == mode)
            })
            .flat_map(|(_, group)| group.iter().cloned())
            .collect();
        assert_eq!(ordered, [surface, additive.clone(), additive]);
    }

    #[test]
    fn additive_blend_weights_rgb_and_preserves_destination_alpha() {
        assert_eq!(
            mesh_blend_state(MeshRenderMode::Surface),
            wgpu::BlendState::ALPHA_BLENDING
        );
        let blend = mesh_blend_state(MeshRenderMode::Additive);
        assert_eq!(blend.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(blend.color.operation, wgpu::BlendOperation::Add);
        assert_eq!(blend.alpha.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(blend.alpha.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(blend.alpha.operation, wgpu::BlendOperation::Add);
    }

    #[test]
    fn slots_are_distinct_until_reset_and_reused_without_creation() {
        let mut slots = DrawSlots::default();
        // Multiple materials, chunks, and render calls in one submission append.
        for id in 0..6 {
            assert_eq!(*slots.next(|| id), id);
        }
        assert_eq!(slots.used, 6);
        slots.reset();
        for id in 0..6 {
            assert_eq!(*slots.next(|| panic!("warm slot allocated")), id);
        }
        slots.reset();
        for _ in 0..RESOURCE_IDLE_FRAMES {
            assert_eq!(*slots.next(|| panic!("active slot allocated")), 0);
            slots.reset();
        }
        assert_eq!(slots.slots.len(), 1);
        for _ in 0..RESOURCE_IDLE_FRAMES {
            slots.reset();
        }
        assert!(slots.slots.is_empty());
        assert_eq!(*slots.next(|| 99), 99);
    }

    #[test]
    fn texture_cache_tracks_resolved_identity_modes_and_eviction() {
        let mut cache = TextureBindings::new();
        let repeat = MeshSamplingMode::Repeat;
        let clamp = MeshSamplingMode::ClampToEdge;
        // Synthetic view identities exercise the same cache as WGPU views.
        let (white, asset, replacement, video) = (0, 1, 2, 3);
        assert_eq!(
            *cache.get_or_insert((white, white, white), repeat, || 10),
            10
        );
        assert_eq!(
            *cache.get_or_insert((white, white, white), clamp, || 11),
            11
        );
        assert_eq!(
            *cache.get_or_insert((asset, asset, asset), repeat, || 20),
            20
        );
        assert_eq!(
            *cache.get_or_insert((asset, asset, asset), clamp, || 21),
            21
        );
        // In-place updates and cloned handles preserve binding identity.
        assert_eq!(
            *cache.get_or_insert((asset, asset, asset), repeat, || panic!("cache miss")),
            20
        );
        // Even without explicit invalidation (direct callers), replacement views
        // must not reuse a binding for the old view at the same logical handle.
        assert_eq!(
            *cache.get_or_insert((replacement, replacement, replacement), repeat, || 30),
            30
        );
        assert_eq!(
            *cache.get_or_insert((asset, asset, asset), repeat, || panic!("old view changed")),
            20
        );
        cache.invalidate(&asset);
        assert!(!cache.entries.keys().any(|(view, _, _, _)| *view == asset));
        assert_eq!(
            *cache.get_or_insert((replacement, replacement, replacement), repeat, || 30),
            30
        );
        assert_eq!(
            *cache.get_or_insert((video, video, video), repeat, || 40),
            40
        );
        cache.invalidate(&video);
        assert_eq!(
            *cache.get_or_insert((replacement, replacement, replacement), repeat, || panic!(
                "restored asset miss"
            )),
            30
        );
        cache.invalidate(&replacement);
        assert_eq!(
            *cache.get_or_insert((white, white, white), repeat, || panic!("fallback miss")),
            10
        );
        cache.invalidate(&white);
        assert!(cache.entries.is_empty());
        cache.get_or_insert((white, white, white), repeat, || 50);
        cache.get_or_insert((asset, asset, asset), clamp, || 60);
        for _ in 0..RESOURCE_IDLE_FRAMES {
            cache.age();
            cache.get_or_insert((white, white, white), repeat, || {
                panic!("live binding expired")
            });
        }
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(
            *cache.get_or_insert((asset, asset, asset), clamp, || 61),
            61
        );
    }

    #[test]
    fn tiny_batch_uploads_only_active_records_but_binding_is_16_kib() {
        let instances = [MeshInstance::new(
            MeshHandle::new(),
            RenderTransform::default(),
        )];
        let (_, raw) = mesh_instance_batches(&instances).next().unwrap();
        assert_eq!(bytemuck::cast_slice::<_, u8>(&raw).len(), 144);
        assert_eq!(INSTANCE_BUFFER_SIZE, 16_128);
    }

    #[test]
    fn uniform_batches_preserve_128_129_and_160_instances() {
        for (count, expected_sizes) in [
            (128, vec![112, 16]),
            (129, vec![112, 17]),
            (160, vec![112, 48]),
        ] {
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
                assert_eq!(raw.len(), chunk.len());
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
                assert_eq!(raw.len(), chunk.len());
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
                        (
                            instance.mesh,
                            instance.texture,
                            MeshSamplingMode::Repeat,
                            MeshRenderMode::Surface,
                            None,
                            None
                        ),
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
                let end = f32::midpoint(1.0, scale);
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
        let repeating = &groups[&(
            mesh,
            Some(texture),
            MeshSamplingMode::Repeat,
            MeshRenderMode::Surface,
            None,
            None,
        )];
        let fitted = &groups[&(
            mesh,
            Some(texture),
            MeshSamplingMode::ClampToEdge,
            MeshRenderMode::Surface,
            None,
            None,
        )];
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
        for anisotropy in [1, 4, 8, 16] {
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
        assert!(MESH_SHADER.contains(
            "let mapped_mr = textureSample(metallic_roughness_map, mesh_sampler, in.uv);"
        ));
        assert!(MESH_SHADER.contains("mapped_mr.bg"));
        assert!(MESH_SHADER.contains("let handedness = duv1.x * duv2.y - duv1.y * duv2.x;"));
        assert!(MESH_SHADER.contains(
            "surface_normal = normalize(mat3x3<f32>(t, bitangent, surface_normal) * mapped_normal);"
        ));
        assert!(MESH_SHADER.contains("let metallic = in.material.x * mr.x;"));
        assert!(MESH_SHADER.contains("let roughness = max(0.04, in.material.y * mr.y);"));
        assert!(MESH_SHADER.contains("let glow = color.rgb * in.emissive;"));
    }

    #[test]
    fn material_maps_split_bindings_and_encode_factors() {
        let mesh = MeshHandle::new();
        let map = SpriteHandle::new();
        let plain = MeshInstance::new(mesh, RenderTransform::default());
        let mut mapped = plain.clone();
        mapped.normal_texture = Some(map);
        mapped.metallic_roughness_texture = Some(map);
        mapped.metallic = 0.7;
        mapped.roughness = 0.3;
        let groups = group_mesh_instances(&[plain.clone(), mapped.clone()]);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[&(
                mesh,
                None,
                MeshSamplingMode::Repeat,
                MeshRenderMode::Surface,
                Some(map),
                Some(map)
            )],
            [mapped.clone()]
        );
        assert_eq!(MeshInstanceRaw::from(&plain).material, [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(
            MeshInstanceRaw::from(&mapped).material,
            [0.7, 0.3, 1.0, 1.0]
        );
        let mut cache = TextureBindings::<u32, u32>::new();
        assert_eq!(
            *cache.get_or_insert((1, 2, 3), MeshSamplingMode::Repeat, || 5),
            5
        );
        assert_eq!(
            *cache.get_or_insert((1, 4, 3), MeshSamplingMode::Repeat, || 6),
            6
        );
        cache.invalidate(&2);
        assert_eq!(cache.entries.len(), 1);
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
        assert_eq!(
            groups[&(
                mesh,
                None,
                MeshSamplingMode::Repeat,
                MeshRenderMode::Surface,
                None,
                None
            )],
            vec![plain]
        );
        assert_eq!(
            groups[&(
                mesh,
                Some(texture),
                MeshSamplingMode::Repeat,
                MeshRenderMode::Surface,
                None,
                None
            )],
            vec![textured, tinted]
        );
        assert_eq!(
            groups[&(
                mesh,
                Some(other_texture),
                MeshSamplingMode::Repeat,
                MeshRenderMode::Surface,
                None,
                None
            )]
                .len(),
            1
        );
        assert_eq!(
            groups[&(
                other_mesh,
                Some(texture),
                MeshSamplingMode::Repeat,
                MeshRenderMode::Surface,
                None,
                None
            )]
                .len(),
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
        assert_eq!(MESH_SHADER.matches("textureSample(").count(), 3);
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
        assert_eq!(std::mem::size_of::<MeshInstanceRaw>(), 144);
        assert_eq!(std::mem::align_of::<MeshInstanceRaw>(), 16);
        assert_eq!(MAX_INSTANCES, 112);
        assert_eq!(
            MAX_INSTANCES * std::mem::size_of::<MeshInstanceRaw>(),
            16_128
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
            (
                11,
                "material",
                std::mem::offset_of!(MeshInstanceRaw, material),
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

    #[test]
    fn water_shader_parses_and_binds_a_128_byte_uniform() {
        let shader = wgpu::naga::front::wgsl::parse_str(WATER_SHADER).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&shader)
        .unwrap();
        let water = shader
            .global_variables
            .iter()
            .map(|(_, variable)| variable)
            .find(|variable| variable.name.as_deref() == Some("water"))
            .expect("water uniform");
        assert_eq!(
            water.binding,
            Some(wgpu::naga::ResourceBinding {
                group: 3,
                binding: 0
            })
        );
        let wgpu::naga::TypeInner::Struct { members, span } = &shader.types[water.ty].inner else {
            panic!("water uniform must be a struct");
        };
        assert_eq!(*span, WATER_UNIFORM_SIZE as u32);
        assert_eq!(members.len(), 8);
        assert_eq!(std::mem::size_of::<WaterUniformRaw>(), 128);
        let instances = shader
            .global_variables
            .iter()
            .map(|(_, variable)| variable)
            .find(|variable| variable.name.as_deref() == Some("instances"))
            .expect("mesh instance uniform");
        let wgpu::naga::TypeInner::Array { base, size, stride } = shader.types[instances.ty].inner
        else {
            panic!("instances must be an array");
        };
        assert_eq!(stride as usize, std::mem::size_of::<MeshInstanceRaw>());
        assert_eq!(
            size,
            wgpu::naga::ArraySize::Constant(std::num::NonZeroU32::new(112).unwrap())
        );
        let wgpu::naga::TypeInner::Struct { span, .. } = shader.types[base].inner else {
            panic!("instance must be a struct");
        };
        assert_eq!(span as usize, std::mem::size_of::<MeshInstanceRaw>());
    }

    #[test]
    #[ignore = "opt-in local headless GPU; no window or network"]
    fn headless_water_pipeline_and_uniform_create() {
        let context = pollster::block_on(crate::WgpuContext::headless(
            crate::WgpuContextConfig::default(),
        ))
        .expect("headless GPU required");
        let mut pipeline =
            MeshPipeline::new(&context.device, wgpu::TextureFormat::Rgba8UnormSrgb, 1, 8).unwrap();
        pipeline.set_water(&context.queue, Some(WaterSettings::default()), 1.0);
    }
}
