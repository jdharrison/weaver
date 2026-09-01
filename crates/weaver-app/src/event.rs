//! Application input events.

/// Events produced by the platform and consumed by the application.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// A window event.
    Window(WindowEvent),
    /// Application should close.
    CloseRequested,
}

/// Window-related event.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowEvent {
    /// Window was resized.
    Resized {
        /// New width in pixels.
        width: u32,
        /// New height in pixels.
        height: u32,
    },
    /// A keyboard key was pressed or released.
    Keyboard {
        /// Key identifier.
        key: String,
        /// Pressed state.
        pressed: bool,
    },
    /// Mouse moved.
    MouseMoved {
        /// X position in pixels.
        x: f64,
        /// Y position in pixels.
        y: f64,
    },
    /// Mouse button pressed or released.
    MouseButton {
        /// Button identifier.
        button: u16,
        /// Pressed state.
        pressed: bool,
    },
}

/// Typed input events for the simulation layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// Toggle pause/resume.
    TogglePause,
    /// Set time multiplier.
    SetTimeMultiplier(f64),
    /// Toggle coordinate frame visualization.
    ToggleCoordinateFrames,
    /// Toggle trajectory history visualization.
    ToggleTrajectoryHistory,
    /// Window resize.
    Resized {
        /// New width in pixels.
        width: u32,
        /// New height in pixels.
        height: u32,
    },
}
