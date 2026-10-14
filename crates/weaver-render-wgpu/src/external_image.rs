//! Bounds and lifecycle policy for renderer-owned external image textures.

/// Maximum external-image dimension, also constrained by the device limit.
pub const MAX_EXTERNAL_IMAGE_DIMENSION: u32 = 4096;
/// Maximum resident external-image pixels (8 MiB of RGBA8 at 1080p).
pub const MAX_EXTERNAL_IMAGE_PIXELS: u64 = 1920 * 1080;

/// Validate decoded pixel dimensions, not CSS dimensions. No scaling is implied.
#[must_use]
pub fn valid_external_image_size(width: u32, height: u32, device_limit: u32) -> bool {
    let limit = device_limit.min(MAX_EXTERNAL_IMAGE_DIMENSION);
    width > 0
        && height > 0
        && width <= limit
        && height <= limit
        && u64::from(width) * u64::from(height) <= MAX_EXTERNAL_IMAGE_PIXELS
}

/// Firefox may temporarily report zero decoded dimensions during a seek even
/// while readyState still reports available data. This is not an oversized frame.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn video_frame_available(ready_state: u16, width: u32, height: u32) -> bool {
    ready_state >= 2 && width > 0 && height > 0
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn needs_allocation(current: Option<(u32, u32)>, requested: (u32, u32)) -> bool {
    current != Some(requested)
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn ensure_texture<'a>(
    overrides: &'a mut std::collections::HashMap<
        weaver_render::SpriteHandle,
        crate::resource::GpuTexture,
    >,
    device: &wgpu::Device,
    handle: weaver_render::SpriteHandle,
    width: u32,
    height: u32,
) -> Result<&'a crate::resource::GpuTexture, crate::error::WgpuRenderError> {
    if !valid_external_image_size(width, height, device.limits().max_texture_dimension_2d) {
        return Err(crate::error::WgpuRenderError::Texture(
            "external image exceeds decoded dimension bounds".into(),
        ));
    }
    if needs_allocation(
        overrides.get(&handle).map(|texture| texture.size),
        (width, height),
    ) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("external-image"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        overrides.insert(
            handle,
            crate::resource::GpuTexture {
                texture,
                view,
                sampler,
                size: (width, height),
            },
        );
    }
    Ok(&overrides[&handle])
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn video_uses_canvas_staging(backend: wgpu::Backend) -> bool {
    backend == wgpu::Backend::BrowserWebGpu
}

/// Renderer-owned staging for WebGPU video frames. Headless/software decoders
/// may produce HTMLVideoElement frames without GPU backing, so direct video
/// import is not universal. Drawing into a reusable canvas supplies a copyable
/// external image without getImageData or CPU readback. WebGL keeps direct video
/// uploads. The canvas is detached from the DOM and lives only with its override.
#[cfg(target_arch = "wasm32")]
pub(crate) struct VideoCanvas {
    canvas: web_sys::HtmlCanvasElement,
    context: web_sys::CanvasRenderingContext2d,
    size: (u32, u32),
}

#[cfg(target_arch = "wasm32")]
impl VideoCanvas {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, crate::error::WgpuRenderError> {
        use crate::error::WgpuRenderError;
        use wasm_bindgen::JsCast;

        if !valid_external_image_size(width, height, MAX_EXTERNAL_IMAGE_DIMENSION) {
            return Err(WgpuRenderError::Texture(
                "video canvas exceeds decoded dimension bounds".into(),
            ));
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| {
                WgpuRenderError::Texture("video staging requires a browser document".into())
            })?;
        let canvas = document
            .create_element("canvas")
            .map_err(|error| {
                WgpuRenderError::Texture(format!("video canvas creation failed: {error:?}"))
            })?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|_| {
                WgpuRenderError::Texture("video staging element is not a canvas".into())
            })?;
        canvas.set_width(width);
        canvas.set_height(height);
        let context = canvas
            .get_context("2d")
            .map_err(|error| {
                WgpuRenderError::Texture(format!("video canvas context failed: {error:?}"))
            })?
            .ok_or_else(|| WgpuRenderError::Texture("video canvas 2D context unavailable".into()))?
            .dyn_into::<web_sys::CanvasRenderingContext2d>()
            .map_err(|_| WgpuRenderError::Texture("video canvas context is not 2D".into()))?;
        Ok(Self {
            canvas,
            context,
            size: (width, height),
        })
    }

    pub(crate) fn draw(
        &mut self,
        video: &web_sys::HtmlVideoElement,
        width: u32,
        height: u32,
    ) -> Result<web_sys::HtmlCanvasElement, crate::error::WgpuRenderError> {
        use crate::error::WgpuRenderError;

        if !valid_external_image_size(width, height, MAX_EXTERNAL_IMAGE_DIMENSION) {
            return Err(WgpuRenderError::Texture(
                "video canvas exceeds decoded dimension bounds".into(),
            ));
        }
        if needs_allocation(Some(self.size), (width, height)) {
            // Assigning canvas dimensions clears/resets its backing store; never
            // do this on ordinary same-size frame updates.
            self.canvas.set_width(width);
            self.canvas.set_height(height);
            self.size = (width, height);
        }
        self.context
            .draw_image_with_html_video_element(video, 0.0, 0.0)
            .map_err(|error| {
                WgpuRenderError::Texture(format!("video canvas drawImage failed: {error:?}"))
            })?;
        Ok(self.canvas.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webgpu_stages_video_while_webgl_keeps_direct_uploads() {
        assert!(video_uses_canvas_staging(wgpu::Backend::BrowserWebGpu));
        assert!(!video_uses_canvas_staging(wgpu::Backend::Gl));
    }

    #[test]
    fn seeks_with_ready_data_but_zero_dimensions_defer_without_relaxing_bounds() {
        for (ready_state, width, height) in [
            (0, 0, 0),
            (1, 638, 360),
            (4, 0, 0),
            (4, 0, 360),
            (4, 638, 0),
        ] {
            assert!(!video_frame_available(ready_state, width, height));
        }
        assert!(video_frame_available(2, 638, 360));
        assert!(video_frame_available(4, 638, 360));
        assert!(video_frame_available(4, 3840, 2160));
        assert!(!valid_external_image_size(3840, 2160, 4096));
    }

    #[test]
    fn decoded_dimensions_are_bounded_without_overflow() {
        for size in [(1920, 1080), (1080, 1920), (4096, 506), (1, 1)] {
            assert!(valid_external_image_size(size.0, size.1, 4096));
        }
        for size in [
            (0, 1080),
            (1920, 0),
            (4097, 1),
            (4096, 507),
            (1921, 1080),
            (u32::MAX, u32::MAX),
        ] {
            assert!(!valid_external_image_size(size.0, size.1, 4096));
        }
        assert!(!valid_external_image_size(4096, 1, 2048));
        assert!(!valid_external_image_size(1, 1, 0));
    }

    #[test]
    fn gpu_override_reuses_texture_replaces_on_resize_and_preserves_idle_asset() {
        let context = match pollster::block_on(crate::WgpuContext::headless(
            crate::WgpuContextConfig::default(),
        )) {
            Ok(context) => context,
            Err(crate::WgpuRenderError::NoAdapter) => {
                eprintln!("GPU resource test skipped: no headless adapter available");
                return;
            }
            Err(error) => panic!("headless device initialization failed: {error}"),
        };
        let handle = weaver_render::SpriteHandle::new();
        let idle = crate::resource::GpuTexture::from_rgba(
            &context.device,
            &context.queue,
            1,
            1,
            &[0, 0, 0, 255],
        )
        .unwrap();
        let idle_texture = idle.texture.clone();
        let assets = std::collections::HashMap::from([(handle, idle)]);
        let mut overrides = std::collections::HashMap::new();
        let first = ensure_texture(&mut overrides, &context.device, handle, 64, 36)
            .unwrap()
            .texture
            .clone();
        assert_eq!(first.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(
            first.usage(),
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT
        );
        let same = ensure_texture(&mut overrides, &context.device, handle, 64, 36)
            .unwrap()
            .texture
            .clone();
        assert_eq!(first, same);
        let resized = ensure_texture(&mut overrides, &context.device, handle, 128, 72)
            .unwrap()
            .texture
            .clone();
        assert_ne!(first, resized);
        assert_eq!(overrides.len(), 1);
        assert!(ensure_texture(&mut overrides, &context.device, handle, 3840, 2160).is_err());
        assert_eq!(overrides[&handle].texture, resized);
        overrides.remove(&handle);
        assert!(overrides.is_empty());
        assert_eq!(assets[&handle].texture, idle_texture);
        let reattached = ensure_texture(&mut overrides, &context.device, handle, 128, 72)
            .unwrap()
            .texture
            .clone();
        assert_ne!(resized, reattached);
    }

    #[test]
    fn resource_lifecycle_allocates_only_on_resize_or_after_removal() {
        let mut current = None;
        let mut allocations = 0;
        for requested in [(1920, 1080), (1920, 1080), (1280, 720), (1280, 720)] {
            if needs_allocation(current, requested) {
                allocations += 1;
                current = Some(requested);
            }
        }
        assert_eq!(allocations, 2);
        current = None; // Detach releases the override, never the idle asset.
        assert!(needs_allocation(current, (1280, 720)));
    }
}
