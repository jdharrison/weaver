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
    /// Requested scene samples: values below 4 select 1, otherwise 4.
    /// Falls back to 1 unless surface color resolve and `Depth32Float` support 4x.
    pub requested_sample_count: u32,
    /// Requested static-texture anisotropy, clamped to 1..=8 and capability-gated.
    /// Defaults to 4 in browsers and 8 on native targets.
    pub requested_anisotropy: u16,
}

impl Default for WgpuContextConfig {
    fn default() -> Self {
        Self {
            power_preference: wgpu::PowerPreference::HighPerformance,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
            desired_format: None,
            present_mode: wgpu::PresentMode::AutoVsync,
            requested_sample_count: 4,
            requested_anisotropy: if cfg!(target_arch = "wasm32") { 4 } else { 8 },
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
    /// Full window size in physical pixels, used for projection and overlay layout.
    pub size: (u32, u32),
    /// Actual surface texture size in pixels. Browsers fit within 1920×1080
    /// (or 1080×1920 in portrait), preserving aspect; native uses the device limit.
    pub surface_size: (u32, u32),
    /// Effective scene samples (1 or 4); UI and text remain single-sample.
    pub sample_count: u32,
    /// Effective static-texture anisotropy (1..=8), or 1 without adapter support.
    pub anisotropy: u16,
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
        let instance = create_instance(&InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        })
        .await;

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
        let surface_size = select_surface_size(size, max_dim, cfg!(target_arch = "wasm32"));
        let surface_caps = surface.get_capabilities(&adapter);
        let format = match config
            .desired_format
            .or(surface_caps.formats.first().copied())
        {
            Some(format) => format,
            None => return Err(WgpuRenderError::UnsupportedSurfaceFormat),
        };
        let sample_count = select_sample_count(
            config.requested_sample_count,
            adapter.get_texture_format_features(format).flags,
            adapter
                .get_texture_format_features(wgpu::TextureFormat::Depth32Float)
                .flags,
        );
        let anisotropy = select_anisotropy(
            config.requested_anisotropy,
            adapter.get_downlevel_capabilities().flags,
        );
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
            sample_count,
            anisotropy,
        })
    }

    /// Create a headless WGPU context without a surface.
    ///
    /// # Errors
    ///
    /// Returns an error if no adapter or device is available.
    pub async fn headless(config: WgpuContextConfig) -> Result<HeadlessContext, WgpuRenderError> {
        let instance = create_instance(&InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        })
        .await;

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
        self.surface_size = select_surface_size(self.size, max_dim, cfg!(target_arch = "wasm32"));
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

// Keep quality selection independent of GPU initialization for fallback tests.
fn select_surface_size(size: (u32, u32), device_limit: u32, browser: bool) -> (u32, u32) {
    let (width, height) = (size.0.max(1), size.1.max(1));
    let device_limit = device_limit.max(1);
    if !browser {
        return (width.min(device_limit), height.min(device_limit));
    }
    // Bound fill rate and MSAA storage independently of HiDPI/Retina scale.
    let long_edge = f64::from(width.max(height));
    let short_edge = f64::from(width.min(height));
    let scale = (1920.0 / long_edge)
        .min(1080.0 / short_edge)
        .min(f64::from(device_limit) / long_edge)
        .min(1.0);
    (
        (f64::from(width) * scale).round().max(1.0) as u32,
        (f64::from(height) * scale).round().max(1.0) as u32,
    )
}

fn select_sample_count(
    requested: u32,
    color: wgpu::TextureFormatFeatureFlags,
    depth: wgpu::TextureFormatFeatureFlags,
) -> u32 {
    use wgpu::TextureFormatFeatureFlags as Flags;
    if requested >= 4
        && color.contains(Flags::MULTISAMPLE_X4 | Flags::MULTISAMPLE_RESOLVE)
        && depth.contains(Flags::MULTISAMPLE_X4)
    {
        4
    } else {
        1
    }
}

fn select_anisotropy(requested: u16, flags: wgpu::DownlevelFlags) -> u16 {
    if flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING) {
        requested.clamp(1, 8)
    } else {
        1
    }
}

fn create_instance(
    descriptor: &InstanceDescriptor,
) -> impl std::future::Future<Output = wgpu::Instance> + '_ {
    #[cfg(target_arch = "wasm32")]
    {
        create_browser_instance(descriptor)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::future::ready(wgpu::Instance::new(descriptor))
    }
}

#[cfg(target_arch = "wasm32")]
async fn create_browser_instance(descriptor: &InstanceDescriptor) -> wgpu::Instance {
    let mut descriptor = descriptor.clone();
    if descriptor.backends.contains(wgpu::Backends::BROWSER_WEBGPU)
        && !browser_webgpu_supported_with_timeout().await
    {
        descriptor.backends.remove(wgpu::Backends::BROWSER_WEBGPU);
    }
    wgpu::Instance::new(&descriptor)
}

#[cfg(target_arch = "wasm32")]
async fn browser_webgpu_supported_with_timeout() -> bool {
    const PROBE_TIMEOUT_MILLIS: i32 = 1_500;

    let probe = wasm_bindgen_futures::future_to_promise(async {
        Ok(wasm_bindgen::JsValue::from_bool(
            wgpu::util::is_browser_webgpu_supported().await,
        ))
    });
    let timeout = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                &resolve,
                PROBE_TIMEOUT_MILLIS,
            );
        } else {
            let _ = resolve.call1(&wasm_bindgen::JsValue::NULL, &wasm_bindgen::JsValue::FALSE);
        }
    });
    let candidates = js_sys::Array::new();
    candidates.push(&probe);
    candidates.push(&timeout);
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::race(&candidates))
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_defaults_keep_msaa_with_moderate_browser_filtering() {
        let config = WgpuContextConfig::default();
        assert_eq!(config.requested_sample_count, 4);
        assert_eq!(
            config.requested_anisotropy,
            if cfg!(target_arch = "wasm32") { 4 } else { 8 }
        );
    }

    #[test]
    fn browser_surface_is_bounded_in_both_orientations_without_upscaling() {
        for (size, expected) in [
            ((1920, 1080), (1920, 1080)),
            ((2560, 1440), (1920, 1080)),
            ((3840, 2160), (1920, 1080)),
            ((7680, 4320), (1920, 1080)),
            ((1080, 1920), (1080, 1920)),
            ((2160, 3840), (1080, 1920)),
            ((2560, 1080), (1920, 810)),
            ((1080, 2560), (810, 1920)),
            ((3440, 1440), (1920, 804)),
            ((1440, 3440), (804, 1920)),
            ((2532, 1170), (1920, 887)),
            ((1170, 2532), (887, 1920)),
            ((800, 600), (800, 600)),
            ((600, 800), (600, 800)),
            ((640, 360), (640, 360)),
            ((360, 640), (360, 640)),
        ] {
            assert_eq!(select_surface_size(size, 8192, true), expected);
        }
    }

    #[test]
    fn browser_surface_respects_device_limits_and_preserves_aspect() {
        for size in [(2532, 1170), (1170, 2532), (1919, 1079)] {
            for device_limit in [128, 512, 8192] {
                let (width, height) = select_surface_size(size, device_limit, true);
                assert!(width >= 1 && height >= 1);
                assert!(width.max(height) <= 1920.min(device_limit));
                assert!(width.min(height) <= 1080.min(device_limit));
                assert!(width <= size.0 && height <= size.1);
                // Each edge rounds by at most half a pixel from the shared scale.
                let aspect_error = (f64::from(width) * f64::from(size.1)
                    - f64::from(height) * f64::from(size.0))
                .abs();
                assert!(aspect_error <= f64::from(size.0 + size.1) * 0.5);
            }
        }
        assert_eq!(select_surface_size((0, 0), 8192, true), (1, 1));
        assert_eq!(select_surface_size((u32::MAX, 1), 8192, true), (1920, 1));
    }

    #[test]
    fn native_surface_retains_full_resolution_up_to_the_device_limit() {
        assert_eq!(select_surface_size((3840, 2160), 8192, false), (3840, 2160));
        assert_eq!(select_surface_size((9000, 2160), 8192, false), (8192, 2160));
        assert_eq!(select_surface_size((0, 0), 8192, false), (1, 1));
    }

    #[test]
    fn msaa_requires_color_samples_resolve_and_depth_samples() {
        use wgpu::TextureFormatFeatureFlags as Flags;
        let x4 = Flags::MULTISAMPLE_X4;
        let resolve = Flags::MULTISAMPLE_RESOLVE;
        for color in [Flags::empty(), x4, resolve, x4 | resolve] {
            for depth in [Flags::empty(), x4] {
                let supported = color == x4 | resolve && depth == x4;
                for requested in [0, 1, 2, 3, 4, 8, u32::MAX] {
                    assert_eq!(
                        select_sample_count(requested, color, depth),
                        if supported && requested >= 4 { 4 } else { 1 }
                    );
                }
            }
        }
    }

    #[test]
    fn anisotropy_is_bounded_and_capability_gated() {
        for requested in [0, 1, 2, 4, 8, 16, u16::MAX] {
            assert_eq!(
                select_anisotropy(requested, wgpu::DownlevelFlags::empty()),
                1
            );
            assert_eq!(
                select_anisotropy(requested, wgpu::DownlevelFlags::ANISOTROPIC_FILTERING),
                requested.clamp(1, 8)
            );
        }
    }
}
