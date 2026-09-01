//! High-level WGPU renderer that consumes a [`weaver_render::SceneSnapshot`].

use crate::context::{WgpuContext, WgpuContextConfig};
use crate::error::WgpuRenderError;
use crate::mesh::MeshPipeline;
use crate::particle::ParticlePipeline;
use crate::resource::{GpuMesh, GpuTexture, Vertex};
use crate::sprite::SpritePipeline;
use crate::text::TextPipeline;
use crate::ui::UiPipeline;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 3],
    _pad: f32,
}

/// A frame produced by the renderer.
pub struct RenderFrame {
    /// Surface texture, if rendering to a window.
    pub surface_texture: wgpu::SurfaceTexture,
    /// Command buffer to submit.
    pub command_buffer: wgpu::CommandBuffer,
}

/// WGPU renderer for Weaver.
pub struct WgpuRenderer {
    context: WgpuContext,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    _camera_layout: wgpu::BindGroupLayout,
    depth_texture: wgpu::TextureView,
    depth_format: wgpu::TextureFormat,
    mesh_pipeline: MeshPipeline,
    sprite_pipeline: SpritePipeline,
    particle_pipeline: ParticlePipeline,
    text_pipeline: TextPipeline,
    ui_pipeline: UiPipeline,
    meshes: HashMap<weaver_render::MeshHandle, GpuMesh>,
    textures: HashMap<weaver_render::SpriteHandle, GpuTexture>,
    default_texture: GpuTexture,
}

impl WgpuRenderer {
    /// Create a renderer bound to a window.
    ///
    /// # Errors
    ///
    /// Returns an error if WGPU initialization fails.
    pub async fn from_window(
        window: std::sync::Arc<Window>,
        config: WgpuContextConfig,
    ) -> Result<Self, WgpuRenderError> {
        let context = WgpuContext::from_window(window, config).await?;
        Self::from_context(context)
    }

    fn from_context(context: WgpuContext) -> Result<Self, WgpuRenderError> {
        let camera_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("camera-bind-group-layout"),
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
        let camera_buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera-uniform"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("camera-bind-group"),
                layout: &camera_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                }],
            });

        let depth_format = wgpu::TextureFormat::Depth32Float;
        let depth_texture = create_depth_view(&context.device, depth_format, context.surface_size);

        let format = context.surface_config.format;
        let mesh_pipeline = MeshPipeline::new(&context.device, format)?;
        let sprite_pipeline = SpritePipeline::new(&context.device, format)?;
        let particle_pipeline = ParticlePipeline::new(&context.device, format)?;
        let text_pipeline = TextPipeline::new(
            &context.device,
            &context.queue,
            format,
            context.surface_size.0,
            context.surface_size.1,
        )?;
        let ui_pipeline = UiPipeline::new(
            &context.device,
            format,
            context.surface_size.0,
            context.surface_size.1,
        )?;

        let default_texture = create_checker_texture(&context.device, &context.queue, 64);

        Ok(Self {
            context,
            camera_buffer,
            camera_bind_group,
            _camera_layout: camera_layout,
            depth_texture,
            depth_format,
            mesh_pipeline,
            sprite_pipeline,
            particle_pipeline,
            text_pipeline,
            ui_pipeline,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            default_texture,
        })
    }

    /// Register a mesh with the renderer.
    pub fn upload_mesh(
        &mut self,
        handle: weaver_render::MeshHandle,
        vertices: &[Vertex],
        indices: &[u16],
    ) -> Result<(), WgpuRenderError> {
        let mesh = GpuMesh::new(&self.context.device, vertices, indices)?;
        self.meshes.insert(handle, mesh);
        Ok(())
    }

    /// Register a texture with the renderer.
    pub fn upload_texture_rgba(
        &mut self,
        handle: weaver_render::SpriteHandle,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), WgpuRenderError> {
        let texture = GpuTexture::from_rgba(
            &self.context.device,
            &self.context.queue,
            width,
            height,
            data,
        )?;
        self.textures.insert(handle, texture);
        Ok(())
    }

    /// Resize the renderer surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.context.resize(width, height);
        self.depth_texture = create_depth_view(
            &self.context.device,
            self.depth_format,
            self.context.surface_size,
        );
        self.text_pipeline.resize(
            &self.context.queue,
            self.context.surface_size.0,
            self.context.surface_size.1,
        );
        self.ui_pipeline.resize(
            &self.context.queue,
            self.context.surface_size.0,
            self.context.surface_size.1,
        );
    }

    /// Render a snapshot to the surface.
    ///
    /// # Errors
    ///
    /// Returns an error if surface acquisition or rendering fails.
    pub fn render(
        &mut self,
        snapshot: &weaver_render::SceneSnapshot,
    ) -> Result<RenderFrame, WgpuRenderError> {
        snapshot
            .validate()
            .map_err(|err| WgpuRenderError::Internal(err.to_string()))?;

        let aspect = self.context.aspect_ratio();
        let view_matrix = snapshot.camera.view_matrix();
        let proj_matrix = snapshot.camera.projection.matrix(aspect);
        let view_proj = proj_matrix * view_matrix;
        let eye = snapshot.camera.eye;
        self.context.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform {
                view_proj: view_proj.to_cols_array_2d(),
                eye: [eye.x, eye.y, eye.z],
                _pad: 0.0,
            }]),
        );

        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("render-encoder"),
                });

        let surface_texture = self.context.acquire()?;
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Background pass: clear color/depth and render distant background particles.
        if !snapshot.background_particles.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(background_color(&snapshot.background_color)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.particle_pipeline.render(
                &mut pass,
                &self.context.device,
                &self.context.queue,
                &self.camera_bind_group,
                &snapshot.background_particles,
            )?;
        }

        // Scene pass: load color/depth and render geometry, sprites, and foreground particles.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if snapshot.background_particles.is_empty() {
                            wgpu::LoadOp::Clear(background_color(&snapshot.background_color))
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture,
                    depth_ops: Some(wgpu::Operations {
                        load: if snapshot.background_particles.is_empty() {
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

            // Group mesh instances by mesh handle.
            let mut mesh_groups: HashMap<
                weaver_render::MeshHandle,
                Vec<weaver_render::MeshInstance>,
            > = HashMap::new();
            for instance in &snapshot.meshes {
                mesh_groups
                    .entry(instance.mesh)
                    .or_default()
                    .push(instance.clone());
            }

            let mut mesh_draws: Vec<(GpuMesh, Vec<weaver_render::MeshInstance>)> = Vec::new();
            for (handle, instances) in mesh_groups {
                if let Some(mesh) = self.meshes.get(&handle) {
                    mesh_draws.push((mesh.clone(), instances));
                }
            }
            self.mesh_pipeline.render(
                &mut pass,
                &self.context.device,
                &self.context.queue,
                &self.camera_bind_group,
                &mesh_draws,
            )?;

            // Sprites: for simplicity, batch all sprites with the first registered texture.
            if !snapshot.sprites.is_empty() {
                let texture = self
                    .textures
                    .values()
                    .next()
                    .unwrap_or(&self.default_texture);
                self.sprite_pipeline.render(
                    &mut pass,
                    &self.context.device,
                    &self.context.queue,
                    &self.camera_bind_group,
                    texture,
                    &snapshot.sprites,
                )?;
            }

            // Foreground particles from emitters.
            let mut all_particles: Vec<weaver_render::Particle> = Vec::new();
            for (_, _, particles) in &snapshot.particles {
                all_particles.extend_from_slice(particles);
            }
            if !all_particles.is_empty() {
                self.particle_pipeline.render(
                    &mut pass,
                    &self.context.device,
                    &self.context.queue,
                    &self.camera_bind_group,
                    &all_particles,
                )?;
            }
        }

        // UI and text in a separate pass without depth.
        self.text_pipeline
            .prepare(&self.context.device, &self.context.queue, &snapshot.text)?;
        let ui_rects: Vec<weaver_render::UiRect> = snapshot
            .ui
            .iter()
            .filter_map(|el| match el {
                weaver_render::UiElement::Rect(r) => Some(r.clone()),
                _ => None,
            })
            .collect();

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.ui_pipeline.render(
                &mut pass,
                &self.context.device,
                &self.context.queue,
                &ui_rects,
            )?;
            self.text_pipeline.render(&mut pass)?;
        }

        let command_buffer = encoder.finish();
        Ok(RenderFrame {
            surface_texture,
            command_buffer,
        })
    }

    /// Submit a rendered frame.
    pub fn present(&self, frame: RenderFrame) {
        self.context.queue.submit([frame.command_buffer]);
        frame.surface_texture.present();
    }

    /// Mutable access to the WGPU context.
    #[must_use]
    pub fn context_mut(&mut self) -> &mut WgpuContext {
        &mut self.context
    }

    /// Immutable access to the WGPU context.
    #[must_use]
    pub fn context(&self) -> &WgpuContext {
        &self.context
    }
}

fn background_color(color: &[f32; 4]) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color[0]),
        g: f64::from(color[1]),
        b: f64::from(color[2]),
        a: f64::from(color[3]),
    }
}

fn create_depth_view(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: (u32, u32),
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-texture"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_checker_texture(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> GpuTexture {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let c = if (x / 8 + y / 8) % 2 == 0 { 200 } else { 50 };
            data.extend_from_slice(&[c, c, c, 255]);
        }
    }
    GpuTexture::from_rgba(device, queue, size, size, &data).expect("default texture upload")
}
