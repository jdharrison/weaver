//! WGPU device, queue, and surface management.

use crate::error::WgpuRenderError;
use wgpu::{InstanceDescriptor, SurfaceConfiguration};
use winit::window::Window;

/// Configuration for creating a [`WgpuContext`].
#[derive(Clone, Debug)]
pub struct WgpuContextConfig {
    /// Power preference for adapter selection.
    pub power_preference: wgpu::PowerPreference,
    /// Required features.
    pub required_features: wgpu::Features,
    /// Required limits.
    pub required_limits: wgpu::Limits,
    /// Desired surface format, if known.
    pub desired_format: Option<wgpu::TextureFormat>,
    /// Present mode.
    pub present_mode: wgpu::PresentMode,
}

impl Default for WgpuContextConfig {
    fn default() -> Self {
        Self {
            power_preference: wgpu::PowerPreference::HighPerformance,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
            desired_format: None,
            present_mode: wgpu::PresentMode::AutoVsync,
        }
    }
}

/// WGPU context including instance, adapter, device, queue, and surface.
pub struct WgpuContext {
    /// WGPU instance.
    pub instance: wgpu::Instance,
    /// Selected adapter.
    pub adapter: wgpu::Adapter,
    /// Logical device.
    pub device: wgpu::Device,
    /// Command queue.
    pub queue: wgpu::Queue,
    /// Surface configuration.
    pub surface_config: SurfaceConfiguration,
    /// Surface.
    pub surface: wgpu::Surface<'static>,
    /// Logical window size in pixels.
    pub size: (u32, u32),
    /// Actual surface texture size in pixels, clamped to the device's maximum
    /// supported 2D texture dimension.
    pub surface_size: (u32, u32),
}

impl WgpuContext {
    /// Create a WGPU context bound to a window.
    ///
    /// # Errors
    ///
    /// Returns an error if no adapter is found, the device cannot be created,
    /// or the surface cannot be configured.
    pub async fn from_window(
        window: std::sync::Arc<Window>,
        config: WgpuContextConfig,
    ) -> Result<Self, WgpuRenderError> {
        let instance = wgpu::Instance::new(&InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| WgpuRenderError::Surface(err.to_string()))?;

        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: config.power_preference,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(_) => return Err(WgpuRenderError::NoAdapter),
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: config.required_features,
                required_limits: adapter.limits(),
                label: Some("weaver-device"),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        let max_dim = device.limits().max_texture_dimension_2d;
        let size = (window.inner_size().width, window.inner_size().height);
        let surface_size = (size.0.max(1).min(max_dim), size.1.max(1).min(max_dim));
        let surface_caps = surface.get_capabilities(&adapter);
        let format = match config
            .desired_format
            .or(surface_caps.formats.first().copied())
        {
            Some(format) => format,
            None => return Err(WgpuRenderError::UnsupportedSurfaceFormat),
        };
        let surface_config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: surface_size.0,
            height: surface_size.1,
            present_mode: config.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface_config,
            surface,
            size,
            surface_size,
        })
    }

    /// Create a headless WGPU context without a surface.
    ///
    /// # Errors
    ///
    /// Returns an error if no adapter or device is available.
    pub async fn headless(config: WgpuContextConfig) -> Result<HeadlessContext, WgpuRenderError> {
        let instance = wgpu::Instance::new(&InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: config.power_preference,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(_) => return Err(WgpuRenderError::NoAdapter),
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: config.required_features,
                required_limits: config.required_limits,
                label: Some("weaver-headless-device"),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        Ok(HeadlessContext {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Reconfigure the surface after a resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let max_dim = self.device.limits().max_texture_dimension_2d;
        self.size = (width, height);
        self.surface_size = (width.min(max_dim), height.min(max_dim));
        self.surface_config.width = self.surface_size.0;
        self.surface_config.height = self.surface_size.1;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Acquire the next surface texture.
    ///
    /// # Errors
    ///
    /// Returns a [`WgpuRenderError`] if acquisition fails.
    pub fn acquire(&self) -> Result<wgpu::SurfaceTexture, WgpuRenderError> {
        match self.surface.get_current_texture() {
            Ok(frame) => Ok(frame),
            Err(wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.surface_config);
                self.surface
                    .get_current_texture()
                    .map_err(WgpuRenderError::from)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Current surface aspect ratio.
    #[must_use]
    pub fn aspect_ratio(&self) -> f32 {
        let (w, h) = self.size;
        if h == 0 { 1.0 } else { w as f32 / h as f32 }
    }
}

/// A headless WGPU context without a surface.
pub struct HeadlessContext {
    /// WGPU instance.
    pub instance: wgpu::Instance,
    /// Selected adapter.
    pub adapter: wgpu::Adapter,
    /// Logical device.
    pub device: wgpu::Device,
    /// Command queue.
    pub queue: wgpu::Queue,
}

/// Surface abstraction used by the renderer.
pub struct RenderSurface<'a> {
    /// Reference to the WGPU context.
    pub context: &'a WgpuContext,
}

impl<'a> RenderSurface<'a> {
    /// Wrap a context as a render surface.
    #[must_use]
    pub const fn new(context: &'a WgpuContext) -> Self {
        Self { context }
    }
}
