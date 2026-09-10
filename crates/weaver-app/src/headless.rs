//! Headless application runner for tests and servers.

use crate::error::AppError;
use crate::event::InputEvent;
use crate::world::{WeaverWorld, WorldConfig};
use std::time::Duration;

/// A headless Weaver application that runs simulation without a renderer.
pub struct HeadlessApp {
    world: WeaverWorld,
    max_steps: Option<usize>,
    shutdown_signal: crate::ShutdownSignal,
}

impl HeadlessApp {
    /// Create a new headless application.
    ///
    /// # Errors
    ///
    /// Returns an error if the world cannot be created.
    pub fn new(config: WorldConfig) -> Result<Self, AppError> {
        Ok(Self {
            world: WeaverWorld::new(config)?,
            max_steps: None,
            shutdown_signal: crate::ShutdownSignal::default(),
        })
    }

    /// Limit the number of steps before automatic shutdown.
    #[must_use]
    pub const fn with_max_steps(mut self, steps: usize) -> Self {
        self.max_steps = Some(steps);
        self
    }

    /// Install a cooperative stop signal, checked before each simulation step.
    #[must_use]
    pub fn with_shutdown_signal(mut self, signal: crate::ShutdownSignal) -> Self {
        self.shutdown_signal = signal;
        self
    }

    /// Run the headless loop.
    ///
    /// # Errors
    ///
    /// Returns an error if the simulation step fails.
    pub fn run(&mut self) -> Result<(), AppError> {
        self.world.start();
        let mut steps = 0;
        while !self.shutdown_signal.is_requested() {
            if let Some(max) = self.max_steps
                && steps >= max
            {
                break;
            }
            self.world.step()?;
            steps += 1;
        }
        Ok(())
    }

    /// Step once.
    ///
    /// # Errors
    ///
    /// Returns an error if the step fails.
    pub fn step(&mut self) -> Result<Option<Duration>, AppError> {
        self.world.step()
    }

    /// Access the underlying world.
    #[must_use]
    pub fn world(&self) -> &WeaverWorld {
        &self.world
    }

    /// Mutable access to the underlying world.
    #[must_use]
    pub fn world_mut(&mut self) -> &mut WeaverWorld {
        &mut self.world
    }

    /// Handle an input event.
    pub fn handle_input(&mut self, event: InputEvent) {
        self.world.handle_input(event);
    }

    /// Shut the headless app down cleanly.
    pub fn shutdown(&mut self) {
        self.world.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_headless_loop_does_not_step() {
        let signal = crate::ShutdownSignal::default();
        signal.request();
        let mut app = HeadlessApp::new(WorldConfig::default())
            .unwrap()
            .with_shutdown_signal(signal);
        let revision = app.world.revision();
        app.run().unwrap();
        assert_eq!(app.world.revision(), revision);
    }

    #[test]
    fn headless_start_shutdown() {
        let mut app = HeadlessApp::new(WorldConfig::default()).unwrap();
        app.world.start();
        app.step().unwrap();
        app.shutdown();
        assert_eq!(app.world.revision().get(), 0);
    }
}
