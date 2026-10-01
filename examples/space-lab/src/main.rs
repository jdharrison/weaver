//! Native desktop entry point for Space Lab.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), weaver_platform_desktop::DesktopError> {
    weaver_platform_desktop::run(
        Box::new(space_lab::SpaceLab::new()),
        weaver_platform_desktop::DesktopConfig::default(),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
