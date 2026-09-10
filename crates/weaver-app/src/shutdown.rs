//! Cooperative application cancellation, independent of simulation time.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Cloneable stop signal with an optional monotonic wall-clock deadline.
///
/// Runners check this between operations; it cannot interrupt a blocking call.
#[derive(Clone, Debug, Default)]
pub struct ShutdownSignal {
    requested: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl ShutdownSignal {
    /// Create a signal that also requests shutdown at the specified deadline.
    #[must_use]
    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
            ..Self::default()
        }
    }

    /// Request shutdown for every clone of this signal.
    pub fn request(&self) {
        self.requested.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation was requested or the wall-clock deadline has elapsed.
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cancellation_is_shared_and_deadline_is_wall_clock() {
        let signal = ShutdownSignal::default();
        let clone = signal.clone();
        assert!(!clone.is_requested());
        signal.request();
        assert!(clone.is_requested());
        assert!(ShutdownSignal::with_deadline(Instant::now()).is_requested());
        assert!(
            !ShutdownSignal::with_deadline(Instant::now() + Duration::from_secs(60)).is_requested()
        );
    }
}
