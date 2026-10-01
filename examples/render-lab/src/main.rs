//! Native desktop entry point for Render Lab.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), weaver_platform_desktop::DesktopError> {
    weaver_platform_desktop::run(
        Box::new(render_lab::RenderLab::new()),
        weaver_platform_desktop::DesktopConfig::default(),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
