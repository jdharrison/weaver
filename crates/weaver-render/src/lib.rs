//! Renderer-neutral vocabulary for the Weaver engine.
//!
//! This crate defines everything the simulation layer needs to describe
//! visible state, without depending on WGPU, winit, or any specific GPU API.

#![warn(missing_docs)]

pub mod camera;
pub mod error;
pub mod id;
pub mod mesh;
pub mod particle;
pub mod snapshot;
pub mod sprite;
pub mod text;
pub mod transform;
pub mod ui;

pub use camera::{Camera, Projection, Viewport};
pub use error::RenderError;
pub use id::RenderResourceId;
pub use mesh::{MeshHandle, MeshInstance};
pub use particle::{Particle, ParticleEmitter};
pub use snapshot::SceneSnapshot;
pub use sprite::{SpriteHandle, SpriteInstance, SpriteSpace};
pub use text::{TextAnchor, TextRun};
pub use transform::RenderTransform;
pub use ui::{UiElement, UiImage, UiRect, UiText};
