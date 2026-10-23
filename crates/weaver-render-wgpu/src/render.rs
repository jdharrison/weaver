//! High-level WGPU renderer that consumes a [`weaver_render::SceneSnapshot`].

use crate::context::{WgpuContext, WgpuContextConfig};
use crate::error::WgpuRenderError;
use crate::mesh::{MeshPipeline, group_mesh_instances};
use crate::motion_blur::{CameraHistory, History, MotionBlur};
use crate::particle::ParticlePipeline;
use crate::resource::{GpuMesh, GpuTexture, Vertex};
use crate::sprite::{SpritePipeline, sprites_for_passes};
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

/// A frame produced by the renderer. Submit with [`WgpuRenderer::present`] or
/// drop it before rendering the next frame; only one frame may be outstanding.
pub struct RenderFrame {
    /// Surface texture, if rendering to a window.
    pub surface_texture: wgpu::SurfaceTexture,
    // Keep submission and the lease together: extracting a pending command buffer
    // would allow its instance slots to be overwritten before submission.
    command_buffer: wgpu::CommandBuffer,
    _mesh_lease: std::sync::Arc<()>,
    motion_history: (u64, Option<CameraHistory>),
}

#[derive(Default)]
struct FrameTracker(std::sync::Arc<()>);

impl FrameTracker {
    fn begin(&self) -> Result<std::sync::Arc<()>, WgpuRenderError> {
        if std::sync::Arc::strong_count(&self.0) != 1 {
            return Err(WgpuRenderError::Internal(
                "present or drop the previous RenderFrame before rendering again".into(),
            ));
        }
        Ok(self.0.clone())
    }
}

/// WGPU renderer for Weaver.
///
/// Background and scene share optional 4x/8x color and matching depth attachments,
/// resolved before optional camera motion blur, sharp foreground meshes, and UI/text.
/// For 4-byte color and `Depth32Float`, 4x adds about 28 bytes per surface pixel
/// over single-sample depth; 8x adds about 60 bytes (excluding the unchanged
/// swapchain and GPU padding).
pub struct WgpuRenderer {
    context: WgpuContext,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    _camera_layout: wgpu::BindGroupLayout,
    depth_texture: SceneAttachment,
    msaa_color: Option<SceneAttachment>,
    depth_format: wgpu::TextureFormat,
    mesh_pipeline: MeshPipeline,
    motion_blur: Option<MotionBlur>,
    motion_history: History,
    foreground_pipeline: Option<MeshPipeline>,
    foreground_depth: Option<wgpu::TextureView>,
    frame_tracker: FrameTracker,
    sprite_pipeline: SpritePipeline,
    screen_sprite_pipeline: Option<SpritePipeline>,
    particle_pipeline: ParticlePipeline,
    text_pipeline: TextPipeline,
    ui_pipeline: UiPipeline,
    meshes: HashMap<weaver_render::MeshHandle, GpuMesh>,
    textures: HashMap<weaver_render::SpriteHandle, GpuTexture>,
    texture_overrides: HashMap<weaver_render::SpriteHandle, GpuTexture>,
    #[cfg(target_arch = "wasm32")]
    video_staging: HashMap<weaver_render::SpriteHandle, crate::external_image::VideoCanvas>,
    default_texture: GpuTexture,
    default_mesh_texture: GpuTexture,
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
        let depth_texture = create_scene_attachment(
            &context.device,
            depth_format,
            context.surface_size,
            context.sample_count,
        );
        let msaa_color = create_msaa_color(&context);

        let format = context.surface_config.format;
        let mesh_pipeline = MeshPipeline::new(
            &context.device,
            format,
            context.sample_count,
            context.anisotropy,
        )?;
        let sprite_pipeline = SpritePipeline::new(&context.device, format, context.sample_count)?;
        let particle_pipeline =
            ParticlePipeline::new(&context.device, format, context.sample_count)?;
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

        let default_texture =
            create_checker_texture(&context.device, &context.queue, 64, context.anisotropy);
        let default_mesh_texture = GpuTexture::from_rgba_with_anisotropy(
            &context.device,
            &context.queue,
            1,
            1,
            &[255; 4],
            context.anisotropy,
        )?;

        Ok(Self {
            context,
            camera_buffer,
            camera_bind_group,
            _camera_layout: camera_layout,
            depth_texture,
            msaa_color,
            depth_format,
            mesh_pipeline,
            motion_blur: None,
            motion_history: History::default(),
            foreground_pipeline: None,
            foreground_depth: None,
            frame_tracker: FrameTracker::default(),
            sprite_pipeline,
            screen_sprite_pipeline: None,
            particle_pipeline,
            text_pipeline,
            ui_pipeline,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            texture_overrides: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            video_staging: HashMap::new(),
            default_texture,
            default_mesh_texture,
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

    /// Register a static texture with mipmaps and effective anisotropic filtering.
    pub fn upload_texture_rgba(
        &mut self,
        handle: weaver_render::SpriteHandle,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), WgpuRenderError> {
        let texture = GpuTexture::from_rgba_with_anisotropy(
            &self.context.device,
            &self.context.queue,
            width,
            height,
            data,
            self.context.anisotropy,
        )?;
        if let Some(old) = self.textures.insert(handle, texture) {
            self.mesh_pipeline.invalidate_texture(&old);
            if let Some(pipeline) = &mut self.foreground_pipeline {
                pipeline.invalidate_texture(&old);
            }
        }
        Ok(())
    }

    /// Remove a dynamic override, releasing its GPU resource and restoring the asset.
    pub fn remove_texture_override(&mut self, handle: weaver_render::SpriteHandle) {
        if let Some(old) = self.texture_overrides.remove(&handle) {
            self.mesh_pipeline.invalidate_texture(&old);
            if let Some(pipeline) = &mut self.foreground_pipeline {
                pipeline.invalidate_texture(&old);
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.video_staging.remove(&handle);
    }

    /// Whether browser external-image copies have the required backend and format.
    #[cfg(target_arch = "wasm32")]
    pub fn supports_external_images(&self) -> bool {
        matches!(
            self.context.adapter.get_info().backend,
            wgpu::Backend::BrowserWebGpu | wgpu::Backend::Gl
        ) && self
            .context
            .adapter
            .get_texture_format_features(wgpu::TextureFormat::Rgba8UnormSrgb)
            .allowed_usages
            .contains(
                wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
    }

    /// Copy a decoded browser video into a reusable, renderer-owned texture.
    /// UV origin is top-left; sRGB, straight alpha, one mip, no CPU readback.
    /// WebGPU stages through a reusable 2D canvas because software-decoded video
    /// frames may lack direct-import GPU backing; WebGL copies video directly.
    /// The uploaded idle asset remains resident and is restored on removal.
    /// Returns false when decoding/seek temporarily has no copyable frame; any
    /// existing GPU override remains unchanged and the caller can retry its slot.
    #[cfg(target_arch = "wasm32")]
    pub fn update_external_video(
        &mut self,
        handle: weaver_render::SpriteHandle,
        video: &web_sys::HtmlVideoElement,
    ) -> Result<bool, WgpuRenderError> {
        let (width, height) = (video.video_width(), video.video_height());
        if !self.supports_external_images() || !self.textures.contains_key(&handle) {
            return Err(WgpuRenderError::Texture(
                "external video backend is unsupported or its target texture is unavailable".into(),
            ));
        }
        if !crate::external_image::video_frame_available(video.ready_state(), width, height) {
            return Ok(false);
        }
        if !crate::external_image::valid_external_image_size(
            width,
            height,
            self.context.device.limits().max_texture_dimension_2d,
        ) {
            return Err(WgpuRenderError::Texture(format!(
                "external video decoded dimensions {width}x{height} exceed bounds"
            )));
        }
        let source = if crate::external_image::video_uses_canvas_staging(
            self.context.adapter.get_info().backend,
        ) {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.video_staging.entry(handle)
            {
                entry.insert(crate::external_image::VideoCanvas::new(width, height)?);
            }
            let canvas = self
                .video_staging
                .get_mut(&handle)
                .unwrap()
                .draw(video, width, height)?;
            wgpu::ExternalImageSource::HTMLCanvasElement(canvas)
        } else {
            wgpu::ExternalImageSource::HTMLVideoElement(video.clone())
        };
        if let Some(old) = self.texture_overrides.get(&handle)
            && old.size != (width, height)
        {
            self.mesh_pipeline.invalidate_texture(old);
            if let Some(pipeline) = &mut self.foreground_pipeline {
                pipeline.invalidate_texture(old);
            }
        }
        let texture = &crate::external_image::ensure_texture(
            &mut self.texture_overrides,
            &self.context.device,
            handle,
            width,
            height,
        )?
        .texture;
        self.context.queue.copy_external_image_to_texture(
            &wgpu::CopyExternalImageSourceInfo {
                source,
                origin: wgpu::Origin2d::ZERO,
                flip_y: false,
            },
            wgpu::CopyExternalImageDestInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
                color_space: wgpu::PredefinedColorSpace::Srgb,
                premultiplied_alpha: false,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(true)
    }

    /// Resize the renderer surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.reset_motion_blur_history();
        self.motion_blur = None;
        self.foreground_depth = None;
        self.context.resize(width, height);
        self.depth_texture = create_scene_attachment(
            &self.context.device,
            self.depth_format,
            self.context.surface_size,
            self.context.sample_count,
        );
        self.msaa_color = create_msaa_color(&self.context);
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
        let mesh_lease = self.frame_tracker.begin()?;
        snapshot
            .validate()
            .map_err(|err| WgpuRenderError::Internal(err.to_string()))?;

        let aspect = self.context.aspect_ratio();
        let view_matrix = snapshot.camera.view_matrix();
        let proj_matrix = snapshot.camera.projection.matrix(aspect);
        let view_proj = proj_matrix * view_matrix;
        validate_blur_camera(
            snapshot.motion_blur.is_some(),
            [view_matrix, proj_matrix, view_proj],
        )?;
        let (scene_sprites, screen_sprites) = sprites_for_passes(
            &snapshot.sprites,
            view_matrix,
            snapshot.motion_blur.is_some(),
        );
        let camera = CameraHistory {
            view_projection: view_proj,
            projection: proj_matrix,
        };
        let pending_history = self
            .motion_history
            .pending(snapshot.motion_blur.map(|_| camera));
        if snapshot.motion_blur.is_some() && self.motion_blur.is_none() {
            crate::motion_blur::check_support(
                &self.context.adapter,
                &self.context.device,
                self.context.surface_config.format,
                self.context.sample_count,
            )?;
            self.motion_blur = Some(MotionBlur::new(
                &self.context.device,
                self.context.surface_config.format,
                self.context.surface_size,
                self.context.sample_count,
            ));
        }
        if !snapshot.foreground_meshes.is_empty() && self.foreground_pipeline.is_none() {
            self.foreground_pipeline = Some(MeshPipeline::new(
                &self.context.device,
                self.context.surface_config.format,
                1,
                self.context.anisotropy,
            )?);
        }
        if !screen_sprites.is_empty() && self.screen_sprite_pipeline.is_none() {
            self.screen_sprite_pipeline = Some(SpritePipeline::new(
                &self.context.device,
                self.context.surface_config.format,
                1,
            )?);
        }
        if (!snapshot.foreground_meshes.is_empty() || !screen_sprites.is_empty())
            && self.foreground_depth.is_none()
        {
            self.foreground_depth = Some(crate::motion_blur::target(
                &self.context.device,
                self.depth_format,
                self.context.surface_size,
                1,
                false,
            ));
        }
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
        // The previous frame was submitted (same queue) or discarded. Writes
        // queued below cannot precede a prior submitted draw on these slots.
        self.mesh_pipeline.reset_instance_slots();
        if let Some(pipeline) = &mut self.foreground_pipeline {
            pipeline.reset_instance_slots();
        }
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let blur = snapshot.motion_blur.and(self.motion_blur.as_ref());
        let resolved_view = blur.map_or(&surface_view, |blur| &blur.color);
        let depth_view = blur.map_or(&self.depth_texture.view, |blur| &blur.depth);
        let scene_view = self
            .msaa_color
            .as_ref()
            .map_or(resolved_view, |color| &color.view);
        let resolve_target = self.msaa_color.as_ref().map(|_| resolved_view);

        // Background preserves multisample color/depth; only the main pass resolves.
        // Background pass: clear color/depth and render distant background particles.
        if !snapshot.background_particles.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(background_color(&snapshot.background_color)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
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
                    view: scene_view,
                    resolve_target,
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
                    view: depth_view,
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

            let mesh_groups = group_mesh_instances(&snapshot.meshes);
            let mut mesh_draws = Vec::new();
            for ((handle, texture_handle, _sampling_mode), instances) in mesh_groups {
                if let Some(mesh) = self.meshes.get(&handle) {
                    let texture = texture_handle
                        .and_then(|handle| {
                            self.texture_overrides
                                .get(&handle)
                                .or_else(|| self.textures.get(&handle))
                        })
                        .unwrap_or(&self.default_mesh_texture);
                    mesh_draws.push((mesh.clone(), texture, instances));
                }
            }
            self.mesh_pipeline.render(
                &mut pass,
                &self.context.device,
                &self.context.queue,
                &self.camera_bind_group,
                &mesh_draws,
            )?;

            // Sort globally before selecting textures; texture grouping must not
            // reorder alpha-blended world sprites. Screen sprites keep input order.
            let sprite_draws: Vec<_> = scene_sprites
                .into_iter()
                .map(|sprite| {
                    let texture = self
                        .texture_overrides
                        .get(&sprite.sprite)
                        .or_else(|| self.textures.get(&sprite.sprite))
                        .unwrap_or(&self.default_texture);
                    (texture, sprite)
                })
                .collect();
            self.sprite_pipeline.render(
                &mut pass,
                &self.context.device,
                &self.camera_bind_group,
                &sprite_draws,
            )?;

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

        if let (Some(blur), Some(settings)) = (blur, snapshot.motion_blur) {
            blur.encode(
                &self.context.queue,
                &mut encoder,
                &surface_view,
                view_proj,
                self.motion_history.previous(camera, settings.reset_history),
                settings,
            );
        }

        // Foreground is an overlay, not world-occluded geometry. Its fresh depth
        // only orders foreground meshes against one another.
        if !snapshot.foreground_meshes.is_empty() {
            let pipeline = self
                .foreground_pipeline
                .as_mut()
                .expect("foreground pipeline");
            let mut draws = Vec::new();
            for ((handle, texture_handle, _), instances) in
                group_mesh_instances(&snapshot.foreground_meshes)
            {
                if let Some(mesh) = self.meshes.get(&handle) {
                    let texture = texture_handle
                        .and_then(|handle| {
                            self.texture_overrides
                                .get(&handle)
                                .or_else(|| self.textures.get(&handle))
                        })
                        .unwrap_or(&self.default_mesh_texture);
                    draws.push((mesh.clone(), texture, instances));
                }
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sharp-foreground"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.foreground_depth.as_ref().expect("foreground depth"),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pipeline.render(
                &mut pass,
                &self.context.device,
                &self.context.queue,
                &self.camera_bind_group,
                &draws,
            )?;
        }

        if !screen_sprites.is_empty() {
            let draws: Vec<_> = screen_sprites
                .into_iter()
                .map(|sprite| {
                    let texture = self
                        .texture_overrides
                        .get(&sprite.sprite)
                        .or_else(|| self.textures.get(&sprite.sprite))
                        .unwrap_or(&self.default_texture);
                    (texture, sprite)
                })
                .collect();
            // Screen sprites compare Always and never write depth. The existing
            // pipeline still requires a compatible attachment; reuse the lazy
            // overlay target, independently cleared after foreground meshes.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sharp-screen-sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.foreground_depth.as_ref().expect("overlay depth"),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.screen_sprite_pipeline
                .as_mut()
                .expect("screen sprite pipeline")
                .render(
                    &mut pass,
                    &self.context.device,
                    &self.camera_bind_group,
                    &draws,
                )?;
        }

        // UI and text stay single-sample on the resolved surface, without depth.
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
            _mesh_lease: mesh_lease,
            motion_history: pending_history,
        })
    }

    /// Submit a rendered frame.
    pub fn present(&mut self, frame: RenderFrame) {
        self.context.queue.submit([frame.command_buffer]);
        frame.surface_texture.present();
        self.motion_history.commit(frame.motion_history);
        // Dynamic debug and tooltip text can otherwise retain glyph atlas work
        // indefinitely across frames.
        self.text_pipeline.trim_atlas();
    }

    /// Invalidate camera motion history after a user/platform camera cut.
    /// Also prevents an already encoded frame from restoring pre-cut history.
    pub fn reset_motion_blur_history(&mut self) {
        self.motion_history.reset();
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

fn validate_blur_camera(enabled: bool, matrices: [glam::Mat4; 3]) -> Result<(), WgpuRenderError> {
    // Legacy screen/UI-only snapshots need not supply an invertible 3D camera.
    if enabled
        && matrices
            .into_iter()
            .any(|matrix| !crate::motion_blur::valid_matrix(matrix))
    {
        return Err(WgpuRenderError::Internal(
            "nonfinite or singular camera matrix".into(),
        ));
    }
    Ok(())
}

fn background_color(color: &[f32; 4]) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color[0]),
        g: f64::from(color[1]),
        b: f64::from(color[2]),
        a: f64::from(color[3]),
    }
}

// Retain raw textures as well as views for the lifetime of the sized attachments.
struct SceneAttachment {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn create_msaa_color(context: &WgpuContext) -> Option<SceneAttachment> {
    (context.sample_count > 1).then(|| {
        create_scene_attachment(
            &context.device,
            context.surface_config.format,
            context.surface_size,
            context.sample_count,
        )
    })
}

fn create_scene_attachment(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    sample_count: u32,
) -> SceneAttachment {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene-attachment"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    SceneAttachment {
        _texture: texture,
        view,
    }
}

fn create_checker_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: u32,
    anisotropy: u16,
) -> GpuTexture {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let c = if (x / 8 + y / 8) % 2 == 0 { 200 } else { 50 };
            data.extend_from_slice(&[c, c, c, 255]);
        }
    }
    GpuTexture::from_rgba_with_anisotropy(device, queue, size, size, &data, anisotropy)
        .expect("default texture upload")
}

#[cfg(test)]
mod frame_tests {
    use super::{FrameTracker, validate_blur_camera};

    #[test]
    fn screen_only_snapshot_requires_invertible_camera_only_for_blur() {
        let snapshot = weaver_render::SceneSnapshot::new(weaver_core::Revision::ZERO);
        assert!(snapshot.validate().is_ok());
        assert!(snapshot.motion_blur.is_none());
        let view = snapshot.camera.view_matrix();
        let projection = snapshot.camera.projection.matrix(1.0);
        let matrices = [view, projection, projection * view];
        assert!(validate_blur_camera(snapshot.motion_blur.is_some(), matrices).is_ok());
        assert!(validate_blur_camera(true, matrices).is_err());
        assert!(validate_blur_camera(false, [glam::Mat4::ZERO; 3]).is_ok());
        assert!(validate_blur_camera(true, [glam::Mat4::ZERO; 3]).is_err());
    }

    #[test]
    fn pending_frame_blocks_reuse_until_submitted_or_discarded() {
        let tracker = FrameTracker::default();
        let pending = tracker.begin().unwrap();
        assert!(tracker.begin().is_err());
        // present holds this lease through queue.submit; an abandoned frame
        // releases it only when its privately owned command buffer is dropped.
        drop(pending);
        let next = tracker.begin().unwrap();
        assert!(tracker.begin().is_err());
        drop(next);
        assert!(tracker.begin().is_ok());
    }
}
