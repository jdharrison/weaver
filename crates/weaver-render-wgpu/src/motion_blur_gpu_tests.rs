//! Opt-in local GPU tests: actual scene rasterization, blur, copy and readback.
use super::*;
use crate::context::{HeadlessContext, WgpuContext, WgpuContextConfig};
use weaver_render::CameraMotionBlur;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 8;

#[derive(Clone, Copy)]
enum DepthFixture {
    Flat,
    Step,
    Slope,
    SlopedStep,
}

impl DepthFixture {
    fn expression(self) -> &'static str {
        match self {
            Self::Flat => "0.5",
            Self::Step => "select(0.5, 0.8, white)",
            Self::Slope => "0.1 + 0.012 * p.x",
            Self::SlopedStep => "0.1 + 0.012 * p.x + select(0.0, 0.1, white)",
        }
    }
}

fn fixture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    depth: DepthFixture,
    partial: bool,
) -> wgpu::RenderPipeline {
    let depth_expression = depth.expression();
    let shader = format!(
        r"
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
            return vec4<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0, 0.5, 1.0);
        }}
        struct Output {{ @location(0) color: vec4<f32>, @builtin(frag_depth) depth: f32 }};
        @fragment fn fs(@builtin(position) p: vec4<f32>) -> Output {{
            let white = p.x >= 32.0;
            var output: Output;
            output.color = vec4<f32>(vec3<f32>(select(0.0, 1.0, white)), 1.0);
            output.depth = {depth_expression};
            return output;
        }}
    "
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blur-pixel-fixture"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blur-pixel-fixture"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        // With partial coverage only sample 3 contains depth; sample 0 is clear.
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: if partial { 8 } else { !0 },
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
        cache: None,
    })
}

struct ScreenOverlay {
    pipeline: crate::sprite::SpritePipeline,
    depth: wgpu::TextureView,
    camera: wgpu::BindGroup,
    texture: crate::resource::GpuTexture,
}

impl ScreenOverlay {
    fn new(context: &HeadlessContext, format: wgpu::TextureFormat) -> Self {
        use wgpu::util::DeviceExt;
        let mut camera_data = [0.0_f32; 20];
        camera_data[..16].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
        let buffer = context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("screen-test-camera"),
                contents: bytemuck::cast_slice(&camera_data),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layout = crate::pipeline::camera_bind_group_layout(&context.device);
        let camera = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("screen-test-camera"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
        Self {
            pipeline: crate::sprite::SpritePipeline::new(&context.device, format, 1).unwrap(),
            depth: target(
                &context.device,
                wgpu::TextureFormat::Depth32Float,
                (WIDTH, HEIGHT),
                1,
                false,
            ),
            camera,
            texture: crate::resource::GpuTexture::from_rgba(
                &context.device,
                &context.queue,
                1,
                1,
                &[255; 4],
            )
            .unwrap(),
        }
    }

    fn encode(
        &mut self,
        context: &HeadlessContext,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
    ) {
        let first = weaver_render::SpriteInstance {
            space: weaver_render::SpriteSpace::Screen,
            transform: weaver_render::RenderTransform {
                translation: glam::Vec3::new(0.0, 0.0, 0.5),
                scale: 0.5,
                ..weaver_render::RenderTransform::default()
            },
            tint: [1.0, 0.0, 0.0, 1.0],
            ..weaver_render::SpriteInstance::default()
        };
        let second = weaver_render::SpriteInstance {
            tint: [0.0, 1.0, 0.0, 0.5],
            ..first.clone()
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sharp-screen-test"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                // A nearer depth must not occlude a Screen sprite (Always).
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.pipeline
            .render(
                &mut pass,
                &context.device,
                &self.camera,
                &[(&self.texture, &first), (&self.texture, &second)],
            )
            .unwrap();
    }
}

struct Fixture {
    blur: MotionBlur,
    output: wgpu::Texture,
    readback: wgpu::Buffer,
}

impl Fixture {
    fn new(
        context: &HeadlessContext,
        format: wgpu::TextureFormat,
        samples: u32,
        depth: DepthFixture,
        partial: bool,
    ) -> Self {
        let device = &context.device;
        let blur = MotionBlur::new(device, format, (WIDTH, HEIGHT), samples);
        let msaa = (samples > 1).then(|| target(device, format, (WIDTH, HEIGHT), samples, false));
        let pipeline = fixture_pipeline(device, format, samples, depth, partial);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur-pixel-fixture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa.as_ref().unwrap_or(&blur.color),
                    resolve_target: msaa.as_ref().map(|_| &blur.color),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &blur.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        context.queue.submit([encoder.finish()]);
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blur-pixel-output"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur-pixel-readback"),
            size: u64::from(WIDTH * HEIGHT * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            blur,
            output,
            readback,
        }
    }

    fn pixels(
        &self,
        context: &HeadlessContext,
        previous: Option<Mat4>,
        settings: CameraMotionBlur,
    ) -> Vec<u8> {
        self.pixels_with_overlay(context, previous, settings, None)
    }

    fn pixels_with_overlay(
        &self,
        context: &HeadlessContext,
        previous: Option<Mat4>,
        settings: CameraMotionBlur,
        overlay: Option<&mut ScreenOverlay>,
    ) -> Vec<u8> {
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.blur.encode(
            &context.queue,
            &mut encoder,
            &self
                .output
                .create_view(&wgpu::TextureViewDescriptor::default()),
            Mat4::IDENTITY,
            previous,
            settings,
        );
        if let Some(overlay) = overlay {
            overlay.encode(
                context,
                &mut encoder,
                &self
                    .output
                    .create_view(&wgpu::TextureViewDescriptor::default()),
            );
        }
        encoder.copy_texture_to_buffer(
            self.output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH * 4),
                    rows_per_image: Some(HEIGHT),
                },
            },
            self.output.size(),
        );
        context.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender.send(result).unwrap();
            });
        context
            .device
            .poll(wgpu::PollType::Wait)
            .expect("GPU completion");
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("map callback")
            .expect("readback map");
        let pixels = self.readback.slice(..).get_mapped_range().to_vec();
        self.readback.unmap();
        pixels
    }
}

fn red(pixels: &[u8], x: usize) -> u8 {
    pixels[(3 * WIDTH as usize + x) * 4]
}

#[test]
#[ignore = "requires a local headless GPU adapter"]
fn headless_single_and_four_sample_passes() {
    pollster::block_on(async {
        let context = WgpuContext::headless(WgpuContextConfig::default())
            .await
            .expect("headless adapter");
        eprintln!("motion blur adapter: {:?}", context.adapter.get_info());
        assert!(matches!(
            check_support(
                &context.adapter,
                &context.device,
                wgpu::TextureFormat::R32Uint,
                1
            ),
            Err(WgpuRenderError::UnsupportedMotionBlur)
        ));
        assert!(matches!(
            check_support(
                &context.adapter,
                &context.device,
                wgpu::TextureFormat::Rgba8Unorm,
                3
            ),
            Err(WgpuRenderError::UnsupportedMotionBlur)
        ));
        for samples in [1, 4] {
            for format in [
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ] {
                check_support(&context.adapter, &context.device, format, samples)
                    .expect("blur capability");
                context
                    .device
                    .push_error_scope(wgpu::ErrorFilter::Validation);
                let fixture = Fixture::new(&context, format, samples, DepthFixture::Flat, false);
                let settings = CameraMotionBlur {
                    exposure_seconds: 1.0 / 60.0,
                    max_blur_pixels: 8.0,
                    ..CameraMotionBlur::default()
                };
                let translated = Some(Mat4::from_translation(glam::Vec3::new(0.25, 0.0, 0.0)));
                let sharp = fixture.pixels(&context, None, settings);
                assert_eq!(red(&sharp, 28), 0);
                assert_eq!(red(&sharp, 40), 255);
                let blurred = fixture.pixels(&context, translated, settings);
                let mut overlay = ScreenOverlay::new(&context, format);
                let overlay_pixels =
                    fixture.pixels_with_overlay(&context, translated, settings, Some(&mut overlay));
                let expected_channel = if format.is_srgb() { 188_u8 } else { 128_u8 };
                for x in 24..40 {
                    let offset = (3 * WIDTH as usize + x) * 4;
                    assert!(
                        overlay_pixels[offset].abs_diff(expected_channel) <= 1,
                        "first sprite contributes half red"
                    );
                    assert!(
                        overlay_pixels[offset + 1].abs_diff(expected_channel) <= 1,
                        "second sprite contributes half green in input order"
                    );
                    assert_eq!(overlay_pixels[offset + 2], 0);
                }
                assert_eq!(
                    red(&overlay_pixels, 23),
                    red(&blurred, 23),
                    "screen sprite has no blur fringe"
                );
                assert_eq!(red(&overlay_pixels, 40), red(&blurred, 40));
                // Reuse the cached single-sample pipeline/depth on another frame.
                assert_eq!(
                    fixture.pixels_with_overlay(&context, translated, settings, Some(&mut overlay)),
                    overlay_pixels
                );
                let range = if format.is_srgb() {
                    175..=200
                } else {
                    115..=140
                };
                assert!(
                    range.contains(&red(&blurred, 28)),
                    "{samples}x {format:?}: pixel 28 = {}",
                    red(&blurred, 28)
                );
                assert_eq!(red(&blurred, 22), 0, "blur length is bounded");
                assert_eq!(red(&blurred, 40), 255);
                let shorter = fixture.pixels(
                    &context,
                    translated,
                    CameraMotionBlur {
                        exposure_seconds: 1.0 / 240.0,
                        ..settings
                    },
                );
                assert_eq!(red(&shorter, 28), 0, "exposure/dt scales reprojection");
                assert!(
                    red(&shorter, 31) > 0,
                    "short exposure still blurs at the edge"
                );
                assert_eq!(
                    fixture.pixels(&context, Some(Mat4::IDENTITY), settings),
                    sharp,
                    "stationary camera"
                );
                assert_eq!(
                    fixture.pixels(
                        &context,
                        translated,
                        CameraMotionBlur {
                            exposure_seconds: 0.0,
                            ..settings
                        }
                    ),
                    sharp,
                    "zero exposure"
                );
                assert_eq!(
                    fixture.pixels(
                        &context,
                        translated,
                        CameraMotionBlur {
                            max_blur_pixels: 0.0,
                            ..settings
                        }
                    ),
                    sharp,
                    "zero cap"
                );
                assert_eq!(
                    fixture.pixels(
                        &context,
                        translated,
                        CameraMotionBlur {
                            frame_delta_seconds: 0.101,
                            ..settings
                        }
                    ),
                    sharp,
                    "stall"
                );
                assert_eq!(
                    fixture.pixels(&context, None, settings),
                    sharp,
                    "no temporal color accumulation"
                );
                let capped = fixture.pixels(
                    &context,
                    translated,
                    CameraMotionBlur {
                        max_blur_pixels: 2.0,
                        ..settings
                    },
                );
                assert_eq!(red(&capped, 28), 0, "small cap cannot cross edge");
                let rotated = fixture.pixels(&context, Some(Mat4::from_rotation_y(0.25)), settings);
                assert!(red(&rotated, 31) > 0, "camera turning must blur");
                let edge = Fixture::new(&context, format, samples, DepthFixture::Step, false);
                assert_eq!(
                    red(&edge.pixels(&context, translated, settings), 28),
                    0,
                    "depth discontinuity must reject white taps"
                );
                // A slope changes depth enough over the kernel that the old
                // constant-depth test rejected valid taps. Planar extrapolation
                // should give exactly the flat surface's color result.
                let slope = Fixture::new(&context, format, samples, DepthFixture::Slope, false);
                let slope_pixels = slope.pixels(&context, translated, settings);
                assert!(
                    red(&slope_pixels, 28).abs_diff(red(&blurred, 28)) <= 1,
                    "sloped plane must preserve the full blur kernel"
                );
                for depth in [DepthFixture::Step, DepthFixture::SlopedStep] {
                    let step = Fixture::new(&context, format, samples, depth, false);
                    let pixels = step.pixels(&context, translated, settings);
                    for x in 28..32 {
                        assert_eq!(
                            red(&pixels, x),
                            0,
                            "depth step at pixel {x} must not become a slope"
                        );
                    }
                }
                if samples == 4 {
                    let partial = Fixture::new(&context, format, samples, DepthFixture::Flat, true);
                    assert!(
                        red(&partial.pixels(&context, translated, settings), 28) > 20,
                        "must use covered sample 3 rather than clear sample 0"
                    );
                }
                context
                    .device
                    .poll(wgpu::PollType::Wait)
                    .expect("GPU completion");
                let error = context.device.pop_error_scope().await;
                assert!(
                    error.is_none(),
                    "{samples}x {format:?} GPU validation: {error:?}"
                );
                eprintln!("{samples}x {format:?}: pixel readbacks passed");
            }
        }
    });
}
