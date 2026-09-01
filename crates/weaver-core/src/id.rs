//! Runtime identity.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// A unique identifier for a running Weaver world / application instance.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorldId(u64);

impl WorldId {
    /// Allocate a fresh world identifier.
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

impl Default for WorldId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WorldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "W{}", self.0)
    }
}
