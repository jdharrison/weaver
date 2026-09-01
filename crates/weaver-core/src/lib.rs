//! Weaver core primitives and interfaces.
//!
//! This crate intentionally stays small: it defines runtime identity, typed
//! commands and events, minimal entity storage, world revisions, and the
//! render-snapshot extraction seam. It does not depend on graphics APIs,
//! windowing, or the full simulation substrate.

#![warn(missing_docs)]

pub mod commands;
pub mod entity;
pub mod error;
pub mod events;
pub mod frame;
pub mod id;
pub mod revision;
pub mod runtime;
pub mod snapshot;
pub mod tick;

pub use commands::{Command, CommandSink};
pub use entity::{EntityId, EntityRegistry};
pub use error::WeaverError;
pub use events::{Event, EventStream};
pub use frame::{FrameId, FrameTree};
pub use id::WorldId;
pub use revision::{Revision, Versioned};
pub use runtime::{RuntimeConfig, RuntimePhase, WeaverRuntime};
pub use snapshot::{ExtractSnapshot, RenderSnapshot};
pub use tick::{Tick, TickRate};
