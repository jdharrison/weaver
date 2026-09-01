//! Text rendering using glyphon / cosmic-text.

use crate::error::WgpuRenderError;
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::collections::HashMap;

/// Text rendering subsystem.
pub struct TextPipeline {
    font_system: FontSystem,
    swash_cache: SwashCache,
    _cache: Cache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    buffers: HashMap<String, Buffer>,
}

impl TextPipeline {
    /// Create the text pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if initialization fails.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        screen_width: u32,
        screen_height: u32,
    ) -> Result<Self, WgpuRenderError> {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let mut viewport = Viewport::new(device, &cache);
        viewport.update(
            queue,
            Resolution {
                width: screen_width,
                height: screen_height,
            },
        );
        let mut atlas = TextAtlas::new(device, queue, &cache, surface_format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Ok(Self {
            font_system,
            swash_cache,
            _cache: cache,
            viewport,
            atlas,
            renderer,
            buffers: HashMap::new(),
        })
    }

    /// Update screen size for screen-space text layout.
    pub fn resize(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        self.viewport.update(queue, Resolution { width, height });
    }

    /// Prepare text for rendering.
    ///
    /// # Errors
    ///
    /// Returns an error if text preparation fails.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        runs: &[weaver_render::TextRun],
    ) -> Result<(), WgpuRenderError> {
        self.buffers.clear();
        for run in runs {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(run.size, run.size));
            buffer.set_size(
                &mut self.font_system,
                Some(self.viewport.resolution().width as f32),
                Some(self.viewport.resolution().height as f32),
            );
            let attrs = Attrs::new().family(Family::SansSerif);
            buffer.set_text(
                &mut self.font_system,
                &run.text,
                &attrs,
                glyphon::Shaping::Advanced,
            );
            self.buffers.insert(run.text.clone(), buffer);
        }

        let mut areas = Vec::with_capacity(runs.len());
        for run in runs {
            let buffer = self.buffers.get(&run.text).expect("buffer inserted above");
            let (width, height) = measure(buffer);
            let anchor_offset = run.anchor.offset();
            let x = run.position.x - width * anchor_offset.x;
            let y = run.position.y - height * anchor_offset.y;
            areas.push(TextArea {
                buffer,
                left: x,
                top: y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: self.viewport.resolution().width as i32,
                    bottom: self.viewport.resolution().height as i32,
                },
                default_color: Color::rgba(
                    (run.color[0] * 255.0) as u8,
                    (run.color[1] * 255.0) as u8,
                    (run.color[2] * 255.0) as u8,
                    (run.color[3] * 255.0) as u8,
                ),
                custom_glyphs: &[],
            });
        }
        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .map_err(|err| WgpuRenderError::Texture(format!("text prepare failed: {err}")))?;
        Ok(())
    }

    /// Render prepared text.
    ///
    /// # Errors
    ///
    /// Returns an error if rendering fails.
    pub fn render<'pass>(
        &'pass mut self,
        pass: &mut wgpu::RenderPass<'pass>,
    ) -> Result<(), WgpuRenderError> {
        self.renderer
            .render(&self.atlas, &self.viewport, pass)
            .map_err(|err| WgpuRenderError::Texture(format!("text render failed: {err}")))
    }

    /// Trim the glyph atlas after presentation.
    pub fn trim_atlas(&mut self) {
        self.atlas.trim();
    }
}

fn measure(buffer: &Buffer) -> (f32, f32) {
    let mut width = 0.0f32;
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height += run.line_height;
    }
    (width, height)
}

/// Measure the pixel bounds of a text string at the given font size.
///
/// This creates a temporary text layout and reports the exact width and
/// height of the laid-out glyphs.
#[must_use]
pub fn measure_text(text: &str, size: f32) -> (f32, f32) {
    let mut font_system = FontSystem::new();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(size, size));
    buffer.set_size(&mut font_system, Some(size * 4096.0), Some(size * 4096.0));
    let attrs = Attrs::new().family(Family::SansSerif);
    buffer.set_text(&mut font_system, text, &attrs, glyphon::Shaping::Advanced);
    measure(&buffer)
}
