//! Typed events emitted by the runtime.

use std::any::Any;

/// A type-erased event emitted by the runtime or a subsystem.
pub struct Event {
    payload: Box<dyn Any + Send + Sync>,
}

impl Event {
    /// Wrap a strongly-typed event payload.
    pub fn new<T>(event: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            payload: Box::new(event),
        }
    }

    /// Attempt to downcast to a concrete type.
    #[must_use]
    pub fn downcast<T>(&self) -> Option<&T>
    where
        T: Any,
    {
        self.payload.downcast_ref::<T>()
    }
}

/// A stream of runtime events.
pub trait EventStream {
    /// Poll for the next event, returning `None` when the stream is closed.
    fn next(&mut self) -> Option<Event>;
}
