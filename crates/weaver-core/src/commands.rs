//! Typed commands that external state changes must pass through.

use crate::EntityId;
use std::any::Any;

/// A type-erased command that can be submitted to the runtime.
///
/// Commands are the only way external input (user input, network events,
/// replicated state) may mutate the world. This preserves a clean audit trail
/// and deterministic replay hook.
pub struct Command {
    payload: Box<dyn Any + Send + Sync>,
}

impl Command {
    /// Wrap a strongly-typed command payload.
    pub fn new<T>(command: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            payload: Box::new(command),
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

    /// Consume the command and downcast to a concrete type.
    #[must_use]
    pub fn into_downcast<T>(self) -> Option<T>
    where
        T: Any,
    {
        let boxed: Box<dyn Any + Send + Sync> = self.payload;
        boxed.downcast::<T>().ok().map(|b| *b)
    }
}

/// A sink that accepts commands for a specific entity.
pub trait CommandSink {
    /// Submit a command to the runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is closed or the command is rejected.
    fn submit(&self, command: Command) -> Result<(), crate::WeaverError>;

    /// Submit a command targeted at a specific entity.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink is closed or the command is rejected.
    fn submit_to(&self, entity: EntityId, command: Command) -> Result<(), crate::WeaverError>;
}
