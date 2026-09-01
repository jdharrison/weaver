//! Errors specific to the WGPU renderer backend.

use thiserror::Error;

/// Errors returned by the WGPU renderer.
#[derive(Debug, Error)]
pub enum WgpuRenderError {
    /// No suitable GPU adapter was found.
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    /// The device could not be created.
    #[error("device creation failed: {0}")]
    DeviceCreationFailed(String),
    /// The surface could not be created or configured.
    #[error("surface error: {0}")]
    Surface(String),
    /// A shader could not be loaded or compiled.
    #[error("shader error: {0}")]
    ShaderError(String),
    /// A pipeline could not be created.
    #[error("pipeline creation failed: {0}")]
    PipelineCreation(String),
    /// A generic internal error.
    #[error("internal render error: {0}")]
    Internal(String),
    /// A GPU buffer operation failed.
    #[error("buffer error: {0}")]
    Buffer(String),
    /// A texture operation failed.
    #[error("texture error: {0}")]
    Texture(String),
    /// Failed to acquire the next swapchain texture.
    #[error("surface acquisition failed")]
    SurfaceAcquisitionFailed,
    /// Surface is lost and must be recreated.
    #[error("surface lost")]
    SurfaceLost,
    /// Surface is out of date (typically after resize).
    #[error("surface out of date")]
    SurfaceOutOfDate,
    /// Texture format is not supported.
    #[error("unsupported surface format")]
    UnsupportedSurfaceFormat,
}

impl From<wgpu::RequestDeviceError> for WgpuRenderError {
    fn from(err: wgpu::RequestDeviceError) -> Self {
        Self::DeviceCreationFailed(err.to_string())
    }
}

impl From<wgpu::SurfaceError> for WgpuRenderError {
    fn from(err: wgpu::SurfaceError) -> Self {
        match err {
            wgpu::SurfaceError::Lost => Self::SurfaceLost,
            wgpu::SurfaceError::Outdated => Self::SurfaceOutOfDate,
            wgpu::SurfaceError::Timeout => Self::SurfaceAcquisitionFailed,
            wgpu::SurfaceError::OutOfMemory => Self::Surface("out of memory".to_string()),
            _ => Self::Surface(err.to_string()),
        }
    }
}
