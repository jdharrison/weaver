//! Resource identifiers used by the renderer.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// An opaque identifier for a renderer-owned resource (mesh, texture, etc.).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderResourceId(u64);

impl RenderResourceId {
    /// Allocate a fresh resource identifier.
    #[must_use]
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw numeric value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for RenderResourceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RenderResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R{}", self.0)
    }
}
