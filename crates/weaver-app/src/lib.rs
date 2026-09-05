//! Native application runner for Weaver.
//!
//! Composes world simulation, Woven connectivity, and rendering into a single
//! event-driven application loop.

#![warn(missing_docs)]

pub mod app;
pub mod debug;
pub mod error;
pub mod event;
pub mod headless;
pub mod world;

pub use app::{Application, ApplicationConfig, TitleStatusFn, TooltipFn};
pub use error::AppError;
pub use event::{AppEvent, InputEvent};
pub use headless::HeadlessApp;
pub use world::{Renderable, WeaverWorld, WorldConfig};
