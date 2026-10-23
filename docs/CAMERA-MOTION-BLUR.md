# Opt-in camera motion blur

Implemented in `weaver-render` and `weaver-render-wgpu`; not enabled by default.
This is reusable engine source, not a VeilRun integration or a submodule pin update.

## Contract and integration

Set `SceneSnapshot.motion_blur` to `Some(CameraMotionBlur { ... })` to enable the
pass. `None` preserves the direct scene-to-surface rendering path. No game rules,
input, networking, or simulation timing are changed.

```rust
use weaver_render::CameraMotionBlur;

// Supply actual presentation timing and a one-frame camera-cut flag.
scene.motion_blur = Some(CameraMotionBlur {
    frame_delta_seconds: elapsed_presentation_seconds,
    reset_history: camera_cut,
    ..CameraMotionBlur::default()
});
```

The defaults are a **1/240-second shutter** and **12 physical-pixel maximum blur
length**. This corresponds to a subtle 90-degree shutter at 60 FPS. The exposure
is fixed in seconds, not in frames: reprojection is scaled by exposure/frame time.
Always update `frame_delta_seconds`; its default is only a 60 Hz convenience value.
Use elapsed time since the previous presented camera, not a fixed simulation tick
or a clamped simulation delta. If presentation fails or a frame is discarded,
accumulate elapsed presentation time or explicitly reset history before resuming.

Settings are validated before resource creation: shutter 0..=1/30 seconds, cap
0..=64 pixels, and finite strictly positive frame time. Zero shutter or cap keeps
the image sharp but still runs the enabled composition path; use `None` to bypass
it. The sampler uses at most 16 taps. Frame times above 100 ms suppress blur.

Set `reset_history` for respawn, teleport, reconnect, focus/capture changes, or any
other discontinuous camera change. Direct renderer consumers may instead call
`WgpuRenderer::reset_motion_blur_history()`. Resize and projection changes reset
the baseline automatically. A first frame or first presented frame after disabling
blur is sharp. Camera history commits only when `present()` submits the frame;
failed or dropped frames do not advance it. An explicit renderer reset also
invalidates history carried by an already encoded frame. Blur-enabled cameras must
have finite, invertible view/projection matrices.

### Sharp overlays and first-person models

Order when enabled:

1. World geometry, world sprites, and particles, with the selected scene MSAA.
2. Resolve color and run depth-reprojected camera blur.
3. `SceneSnapshot.foreground_meshes`.
4. Screen-space sprites, in their existing order.
5. UI and text.

Move only camera-mounted model instances that should remain sharp into
`foreground_meshes`. They share mesh/texture handles and the scene camera, but use
a **fresh single-sample depth buffer**: they occlude one another, not world geometry.
This intentionally makes them an overlay. It is not suitable for remote players,
world weapons, or other objects that must be occluded by the environment.
Foreground works independently of whether blur is enabled. Its silhouettes do
**not** receive scene MSAA in this version; native visual evaluation is required
before adopting it for a game's weapon. There is no separate foreground FOV.

## Implementation and costs

The backend lazily allocates a single-sample scene-color target and sampleable
`Depth32Float` scene depth when first enabled. The existing scene color MSAA is
resolved into that target. The shader reconstructs current world positions and
projects them through the previous camera, covering both rotation and translation.
It uses current-frame color only: there is no previous-color accumulation.
Nearest covered MSAA depth is selected rather than assuming sample zero is covered.
Slope-aware depth rejection reduces color bleeding across discontinuities.
sRGB source reads decode to linear values; the output target encodes once.

Offscreen targets, pipelines and bindings are reused. Resize releases sized blur
and overlay resources for lazy reconstruction. Disabling bypasses the pass but
retains its resources for reuse. Additional GPU memory includes the resolved scene
color and sampled scene depth (the original direct-path depth remains allocated),
plus an optional single-sample foreground/screen-sprite depth target. The extra
fullscreen sampling costs bandwidth; neither FPS improvement nor a particular
performance budget is claimed.

Format and adapter capabilities are checked before enabling. Unsupported cases
return `WgpuRenderError::UnsupportedMotionBlur`; there is no silent fallback or
implicit MSAA downgrade. A consumer may explicitly disable the effect after
reporting that result. Supported format checks cover RGBA/BGRA 8-bit linear/sRGB
and RGBA16Float; validated formats and backends are narrower, as listed below.

## Limits

- Camera motion only, not independently moving object motion vectors.
- Clear-depth background stays sharp. Transparent world effects do not supply
  independent velocity/depth; they may inherit the underlying surface's blur.
- Depth rejection is a screen-space heuristic, not perfect disocclusion recovery.
- Does not replace MSAA/TAA, repair frame pacing, or interpolate stepped camera aim.
- Foreground models are currently single-sample overlays, as described above.
- No game, desktop-shell settings/toggle, browser, or mobile integration is included.

## Validation for this change

Run from the engine root:

```sh
cargo test --locked -p weaver-render -p weaver-render-wgpu
cargo test --locked -p weaver-render-wgpu headless_single_and_four_sample_passes -- --ignored --nocapture
cargo clippy --locked -p weaver-render -p weaver-render-wgpu --lib -- -D warnings
cargo fmt -p weaver-render -p weaver-render-wgpu -- --check
```

CPU suite: **5 neutral + 51 backend tests passed**; the headless test is ignored in
ordinary runs and was also run explicitly. Headless GPU readback passed on
**AMD Radeon RX 7900 GRE, Vulkan/RADV, Mesa 25.2.8**, for **1× and 4× samples** with
**RGBA8 linear and sRGB** targets. Assertions cover translation/rotation,
stationary/no-history/zero-strength/stall bypass, exposure scaling, pixel caps,
flat/sloped surfaces and depth steps, nonzero covered MSAA sample selection,
linear/sRGB color averaging, and sharp ordered screen sprites. CPU tests cover
settings validation, history commits/resets, camera math, and disabled-path camera
compatibility.

Library Clippy with warnings denied and scoped formatting passed. All-target
Clippy still reports the pre-existing `manual_midpoint` warning in `mesh.rs`.
No full windowed scene/foreground/UI smoke test or performance measurement has
been performed. Other adapters/backends, 8× MSAA, BGRA, and RGBA16Float are not
claimed validated. VeilRun's desktop package compiles against these additions but
has not enabled them. No hosted connection, commit, or gitlink update was made.
