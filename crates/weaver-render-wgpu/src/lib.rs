//! WGPU backend for Weaver rendering.
//!
//! This crate implements the renderer-neutral vocabulary from
//! `weaver-render` using WGPU. It owns the device, surface, pipelines,
//! resources, and frame submission.

#![warn(missing_docs)]

pub mod context;
pub mod error;
pub mod mesh;
pub mod particle;
pub mod pipeline;
pub mod render;
pub mod resource;
pub mod sprite;
pub mod text;
pub mod ui;

pub use context::{HeadlessContext, RenderSurface, WgpuContext, WgpuContextConfig};
pub use error::WgpuRenderError;
pub use render::{RenderFrame, WgpuRenderer};
pub use resource::{GpuMesh, GpuTexture, Vertex};
