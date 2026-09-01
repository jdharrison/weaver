//! Renderer-neutral errors and capabilities.

use thiserror::Error;

/// Errors that can occur during rendering or resource management.
#[derive(Debug, Error)]
pub enum RenderError {
    /// The renderer failed to initialize.
    #[error("renderer initialization failed: {0}")]
    InitializationFailed(String),
    /// A requested resource is missing.
    #[error("resource not found: {0}")]
    ResourceNotFound(String),
    /// The surface is unavailable or lost.
    #[error("surface lost")]
    SurfaceLost,
    /// The surface needs to be reconfigured after a resize.
    #[error("surface needs resize")]
    SurfaceNeedsResize,
    /// A shader could not be loaded or compiled.
    #[error("shader error: {0}")]
    ShaderError(String),
    /// A generic internal error.
    #[error("internal render error: {0}")]
    Internal(String),
}

/// GPU capabilities reported by a renderer backend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderCapabilities {
    /// Maximum supported texture dimension.
    pub max_texture_dimension_2d: u32,
    /// Maximum number of vertices per draw call.
    pub max_vertices_per_draw: u32,
    /// Whether instancing is supported.
    pub supports_instancing: bool,
}
