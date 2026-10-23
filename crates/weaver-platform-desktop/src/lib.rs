//! Native desktop shell for platform-neutral Weaver applications.

#![warn(missing_docs)]

use glam::Vec2;
use std::sync::Arc;
use std::time::Instant;
use weaver_app_core::{
    AppAction, ButtonState, FrameContext, InputFrame, PointerMode, RealtimeCommand, RealtimeDriver,
    RealtimeEvent, TextInputEvent, TextInputMode, WeaverApp,
};
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Desktop shell configuration.
#[derive(Clone, Copy, Debug)]
pub struct DesktopConfig {
    /// Initial logical width.
    pub width: u32,
    /// Initial logical height.
    pub height: u32,
    /// Surface presentation mode.
    pub present_mode: wgpu::PresentMode,
    /// Requested scene MSAA: highest supported of 8, 4, 1 up to this request.
    pub requested_sample_count: u32,
    /// Static-texture anisotropy, clamped to 1..=16 and capability-gated.
    pub requested_anisotropy: u16,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            present_mode: wgpu::PresentMode::AutoVsync,
            requested_sample_count: 4,
            requested_anisotropy: 8,
        }
    }
}

/// Desktop shell startup failures.
#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    /// Event-loop or window failure.
    #[error("desktop event loop failed: {0}")]
    EventLoop(String),
    /// Renderer initialization failure.
    #[error("desktop renderer failed: {0}")]
    Renderer(String),
}

/// Run an application until its desktop window closes.
///
/// # Errors
///
/// Returns an error if the event loop, window, renderer, or resource upload fails.
pub fn run(app: Box<dyn WeaverApp>, config: DesktopConfig) -> Result<(), DesktopError> {
    run_optional_realtime(app, config, None)
}

/// Run an application with an optional-transport realtime driver until its desktop window closes.
///
/// # Errors
///
/// Returns an error if the event loop, window, renderer, or resource upload fails.
pub fn run_with_realtime(
    app: Box<dyn WeaverApp>,
    config: DesktopConfig,
    realtime: Box<dyn RealtimeDriver>,
) -> Result<(), DesktopError> {
    run_optional_realtime(app, config, Some(realtime))
}

fn run_optional_realtime(
    app: Box<dyn WeaverApp>,
    config: DesktopConfig,
    realtime: Option<Box<dyn RealtimeDriver>>,
) -> Result<(), DesktopError> {
    let event_loop =
        EventLoop::new().map_err(|error| DesktopError::EventLoop(error.to_string()))?;
    let mut shell = DesktopShell::new(app, config, realtime);
    event_loop
        .run_app(&mut shell)
        .map_err(|error| DesktopError::EventLoop(error.to_string()))?;
    shell.startup_error.map_or(Ok(()), Err)
}

const MAX_REALTIME_EVENTS_PER_REDRAW: usize = 256;
const MAX_REALTIME_COMMANDS_PER_REDRAW: usize = 32;
const MAX_TEXT_EVENTS_PER_REDRAW: usize = 32;
const MAX_TEXT_EVENT_BYTES: usize = 1_024;

fn poll_realtime(
    app: &mut dyn WeaverApp,
    driver: &mut dyn RealtimeDriver,
    commands: &mut Vec<RealtimeCommand>,
    events: &mut Vec<RealtimeEvent>,
) -> bool {
    commands.clear();
    events.clear();
    app.drain_realtime_commands(commands);
    let commands_overflowed = commands.len() > MAX_REALTIME_COMMANDS_PER_REDRAW;
    commands.truncate(MAX_REALTIME_COMMANDS_PER_REDRAW);

    driver.poll(commands, events);

    let command_notice = usize::from(commands_overflowed);
    let events_overflowed =
        events.len().saturating_add(command_notice) > MAX_REALTIME_EVENTS_PER_REDRAW;
    let notice_count = command_notice + usize::from(events_overflowed);
    events.truncate(MAX_REALTIME_EVENTS_PER_REDRAW - notice_count);
    if commands_overflowed {
        events.push(RealtimeEvent::Disconnected {
            reason: format!(
                "realtime command limit exceeded ({MAX_REALTIME_COMMANDS_PER_REDRAW} per redraw)"
            ),
        });
    }
    if events_overflowed {
        events.push(RealtimeEvent::Disconnected {
            reason: format!(
                "realtime event limit exceeded ({MAX_REALTIME_EVENTS_PER_REDRAW} per redraw)"
            ),
        });
    }

    let disconnected = events
        .iter()
        .any(|event| matches!(event, RealtimeEvent::Disconnected { .. }));
    for event in events.drain(..) {
        app.handle_realtime_event(event);
    }
    !disconnected
}

#[derive(Default)]
struct MovementKeys(u16);

impl MovementKeys {
    const KEY_W: u16 = 1;
    const ARROW_UP: u16 = 2;
    const KEY_S: u16 = 4;
    const ARROW_DOWN: u16 = 8;
    const KEY_A: u16 = 16;
    const ARROW_LEFT: u16 = 32;
    const KEY_D: u16 = 64;
    const ARROW_RIGHT: u16 = 128;
    const SHIFT_LEFT: u16 = 256;
    const SHIFT_RIGHT: u16 = 512;
    const CONTROL_LEFT: u16 = 1024;

    fn set(&mut self, key: PhysicalKey, pressed: bool) {
        let bit = match key {
            PhysicalKey::Code(KeyCode::KeyW) => Self::KEY_W,
            PhysicalKey::Code(KeyCode::ArrowUp) => Self::ARROW_UP,
            PhysicalKey::Code(KeyCode::KeyS) => Self::KEY_S,
            PhysicalKey::Code(KeyCode::ArrowDown) => Self::ARROW_DOWN,
            PhysicalKey::Code(KeyCode::KeyA) => Self::KEY_A,
            PhysicalKey::Code(KeyCode::ArrowLeft) => Self::ARROW_LEFT,
            PhysicalKey::Code(KeyCode::KeyD) => Self::KEY_D,
            PhysicalKey::Code(KeyCode::ArrowRight) => Self::ARROW_RIGHT,
            PhysicalKey::Code(KeyCode::ShiftLeft) => Self::SHIFT_LEFT,
            PhysicalKey::Code(KeyCode::ShiftRight) => Self::SHIFT_RIGHT,
            PhysicalKey::Code(KeyCode::ControlLeft) => Self::CONTROL_LEFT,
            _ => return,
        };
        if pressed {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    fn axis(&self) -> Vec2 {
        let contains_any = |bits| self.0 & bits != 0;
        let right = contains_any(Self::KEY_D | Self::ARROW_RIGHT);
        let left = contains_any(Self::KEY_A | Self::ARROW_LEFT);
        let forward = contains_any(Self::KEY_W | Self::ARROW_UP);
        let backward = contains_any(Self::KEY_S | Self::ARROW_DOWN);
        Vec2::new(
            f32::from(right) - f32::from(left),
            f32::from(forward) - f32::from(backward),
        )
        .clamp_length_max(1.0)
    }

    fn sprint(&self) -> bool {
        self.0 & (Self::SHIFT_LEFT | Self::SHIFT_RIGHT) != 0
    }

    fn crouch(&self) -> bool {
        self.0 & Self::CONTROL_LEFT != 0
    }

    fn clear(&mut self) {
        self.0 = 0;
    }
}

/// Pure routing policy: a capture click is consumed, never forwarded as fire.
#[derive(Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent application and window gates, not mutually exclusive states"
)]
struct GameplayRouting {
    pointer: PointerMode,
    enabled: bool,
    focused: bool,
    text_focused: bool,
    captured: bool,
}

impl GameplayRouting {
    fn available(self) -> bool {
        self.pointer == PointerMode::LockedLook && self.focused && !self.text_focused
    }

    fn left_button(self, input: &mut InputFrame, pressed: bool) -> bool {
        if !self.available() {
            return false;
        }
        if !self.captured {
            return pressed;
        }
        if self.enabled {
            input.fire.set(pressed);
        }
        false
    }

    fn reload(self, input: &mut InputFrame, pressed: bool, repeat: bool) {
        if self.available() && self.enabled && self.captured && !repeat {
            input.reload.set(pressed);
        }
    }
}

fn reset_controls(input: &mut InputFrame) {
    input.movement = Vec2::ZERO;
    input.sprint = false;
    input.crouch = false;
    input.look_delta = Vec2::ZERO;
    input.zoom_delta = 0.0;
    input.toggle_perspective = false;
    input.fire = ButtonState::default();
    input.reload = ButtonState::default();
    input.controls_reset = true;
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "Tracks independent window, pointer, and application state"
)]
struct DesktopShell {
    app: Box<dyn WeaverApp>,
    config: DesktopConfig,
    realtime: Option<Box<dyn RealtimeDriver>>,
    realtime_commands: Vec<RealtimeCommand>,
    realtime_events: Vec<RealtimeEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<weaver_render_wgpu::WgpuRenderer>,
    startup_error: Option<DesktopError>,
    input: InputFrame,
    movement: MovementKeys,
    cursor_position: Option<Vec2>,
    last_cursor: Option<Vec2>,
    dragging: bool,
    cursor_captured: bool,
    blocked_until_left_release: bool,
    focused: bool,
    last_pointer_mode: PointerMode,
    last_gameplay_enabled: bool,
    last_text_input_mode: TextInputMode,
    started_at: Instant,
    last_frame: Instant,
}

impl DesktopShell {
    fn new(
        app: Box<dyn WeaverApp>,
        config: DesktopConfig,
        realtime: Option<Box<dyn RealtimeDriver>>,
    ) -> Self {
        let now = Instant::now();
        let last_pointer_mode = app.pointer_mode();
        let last_gameplay_enabled = app.gameplay_input_enabled();
        Self {
            app,
            config,
            realtime,
            realtime_commands: Vec::with_capacity(MAX_REALTIME_COMMANDS_PER_REDRAW),
            realtime_events: Vec::with_capacity(MAX_REALTIME_EVENTS_PER_REDRAW),
            window: None,
            renderer: None,
            startup_error: None,
            input: InputFrame::default(),
            movement: MovementKeys::default(),
            cursor_position: None,
            last_cursor: None,
            dragging: false,
            cursor_captured: false,
            blocked_until_left_release: false,
            focused: false,
            last_pointer_mode,
            last_gameplay_enabled,
            last_text_input_mode: TextInputMode::Disabled,
            started_at: now,
            last_frame: now,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: DesktopError) {
        tracing::error!("{error}");
        self.startup_error = Some(error);
        event_loop.exit();
    }

    fn release_cursor(&mut self) {
        if let Some(window) = self.window.as_ref() {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.cursor_captured = false;
        self.blocked_until_left_release = false;
        self.movement.clear();
        self.dragging = false;
        self.last_cursor = None;
        reset_controls(&mut self.input);
    }

    fn capture_succeeded(&mut self) {
        self.cursor_captured = true;
        self.blocked_until_left_release = true;
    }

    fn left_button(&mut self, pressed: bool) -> bool {
        // Capture consumes the entire click, including duplicate presses and holds
        // across redraws. Keep fire neutral so held-button consumers cannot fire.
        if self.blocked_until_left_release {
            if !pressed {
                self.blocked_until_left_release = false;
            }
            return false;
        }
        let routing = self.gameplay_routing();
        routing.left_button(&mut self.input, pressed)
    }

    fn gameplay_routing(&self) -> GameplayRouting {
        GameplayRouting {
            pointer: self.app.pointer_mode(),
            enabled: self.app.gameplay_input_enabled(),
            focused: self.focused,
            text_focused: self.app.text_input_mode() == TextInputMode::Focused
                || self.input.text_events.contains(&TextInputEvent::Open),
            captured: self.cursor_captured,
        }
    }

    fn sync_input_modes(&mut self) {
        let pointer = self.app.pointer_mode();
        let enabled = self.app.gameplay_input_enabled();
        if (pointer != self.last_pointer_mode || enabled != self.last_gameplay_enabled)
            && (enabled || self.last_gameplay_enabled)
        {
            self.release_cursor();
        }
        self.last_pointer_mode = pointer;
        self.last_gameplay_enabled = enabled;
        self.sync_text_input();
    }

    fn push_text_event(&mut self, event: TextInputEvent) {
        if self.input.text_events.len() < MAX_TEXT_EVENTS_PER_REDRAW {
            self.input.text_events.push(event);
        }
    }

    fn push_committed_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut end = text.len().min(MAX_TEXT_EVENT_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end != 0 {
            self.push_text_event(TextInputEvent::Insert(text[..end].to_owned()));
        }
    }

    fn sync_text_input(&mut self) {
        let focused = self.app.text_input_mode() == TextInputMode::Focused;
        if focused {
            self.movement.clear();
            self.input.movement = Vec2::ZERO;
            self.input.sprint = false;
            self.input.crouch = false;
            self.input.toggle_perspective = false;
            self.input.zoom_delta = 0.0;
            self.input.look_delta = Vec2::ZERO;
            self.dragging = false;
            if self.last_text_input_mode != TextInputMode::Focused || self.cursor_captured {
                self.release_cursor();
            }
        }
        let mode = self.app.text_input_mode();
        if mode != self.last_text_input_mode {
            if let Some(window) = self.window.as_ref() {
                window.set_ime_allowed(focused);
            }
            self.last_text_input_mode = mode;
        }
    }
}

impl ApplicationHandler for DesktopShell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            self.focused = window.has_focus();
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(self.app.title())
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.width,
                self.config.height,
            ));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, DesktopError::EventLoop(error.to_string()));
                return;
            }
        };
        let mut renderer = match pollster::block_on(weaver_render_wgpu::WgpuRenderer::from_window(
            Arc::clone(&window),
            weaver_render_wgpu::WgpuContextConfig {
                present_mode: self.config.present_mode,
                requested_sample_count: self.config.requested_sample_count,
                requested_anisotropy: self.config.requested_anisotropy,
                ..weaver_render_wgpu::WgpuContextConfig::default()
            },
        )) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fail(event_loop, DesktopError::Renderer(error.to_string()));
                return;
            }
        };
        for mesh in &self.app.assets().meshes {
            if let Err(error) = renderer.upload_mesh(mesh.handle, &mesh.vertices, &mesh.indices) {
                self.fail(event_loop, DesktopError::Renderer(error.to_string()));
                return;
            }
        }
        for texture in &self.app.assets().textures {
            if let Err(error) = renderer.upload_texture_rgba(
                texture.handle,
                texture.width,
                texture.height,
                &texture.rgba,
            ) {
                self.fail(event_loop, DesktopError::Renderer(error.to_string()));
                return;
            }
        }
        self.focused = window.has_focus();
        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        self.sync_input_modes();
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.input = InputFrame::default();
                    self.release_cursor();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = Vec2::new(position.x as f32, position.y as f32);
                self.cursor_position = Some(position);
                if self.dragging && self.app.pointer_mode() == PointerMode::OrbitDrag {
                    if let Some(last) = self.last_cursor {
                        self.input.look_delta += position - last;
                    }
                    self.last_cursor = Some(position);
                }
            }
            WindowEvent::MouseWheel { delta, .. }
                if self.app.pointer_mode() != PointerMode::None
                    && self.app.text_input_mode() != TextInputMode::Focused =>
            {
                self.input.zoom_delta += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.02,
                };
            }
            WindowEvent::Ime(Ime::Commit(text))
                if self.app.text_input_mode() == TextInputMode::Focused =>
            {
                self.push_committed_text(&text);
            }
            WindowEvent::MouseInput {
                button: MouseButton::Middle,
                state: ElementState::Pressed,
                ..
            } if self.app.pointer_mode() == PointerMode::LockedLook
                && self.app.text_input_mode() != TextInputMode::Focused =>
            {
                self.input.toggle_perspective = !self.input.toggle_perspective;
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state,
                ..
            } => match self.app.pointer_mode() {
                PointerMode::OrbitDrag => {
                    self.dragging = state == ElementState::Pressed;
                    self.last_cursor = self.cursor_position;
                }
                PointerMode::LockedLook => {
                    if self.left_button(state == ElementState::Pressed)
                        && let Some(window) = self.window.as_ref()
                        && window
                            .set_cursor_grab(CursorGrabMode::Locked)
                            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
                            .is_ok()
                    {
                        window.set_cursor_visible(false);
                        self.capture_succeeded();
                    }
                }
                _ => {}
            },
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        physical_key,
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                let text_mode = self.app.text_input_mode();
                if pressed && !repeat && logical_key == Key::Named(NamedKey::Enter) {
                    match text_mode {
                        TextInputMode::Available => {
                            self.movement.clear();
                            self.release_cursor();
                            self.input.sprint = false;
                            self.input.toggle_perspective = false;
                            self.input.zoom_delta = 0.0;
                            self.push_text_event(TextInputEvent::Open);
                            return;
                        }
                        TextInputMode::Focused => {
                            self.push_text_event(TextInputEvent::Submit);
                            return;
                        }
                        TextInputMode::Disabled => {}
                    }
                }
                if text_mode == TextInputMode::Focused {
                    if pressed {
                        match logical_key.as_ref() {
                            Key::Named(NamedKey::Escape) => {
                                self.push_text_event(TextInputEvent::Cancel);
                            }
                            Key::Named(NamedKey::Backspace) => {
                                self.push_text_event(TextInputEvent::Backspace);
                            }
                            Key::Character(text) => self.push_committed_text(text),
                            _ => {}
                        }
                    }
                    return;
                }
                if pressed && logical_key == Key::Named(NamedKey::Escape) {
                    if self.cursor_captured {
                        self.release_cursor();
                    } else {
                        event_loop.exit();
                    }
                    return;
                }
                if physical_key == PhysicalKey::Code(KeyCode::KeyR) {
                    let routing = self.gameplay_routing();
                    routing.reload(&mut self.input, pressed, repeat);
                }
                self.movement.set(physical_key, pressed);
                self.input.sprint = self.movement.sprint();
                self.input.crouch = self.movement.crouch();
                if pressed {
                    let action = match logical_key.as_ref() {
                        Key::Named(NamedKey::Space) => Some(AppAction::TogglePause),
                        Key::Named(NamedKey::F1) => Some(AppAction::ToggleCoordinateFrames),
                        Key::Named(NamedKey::F2) => Some(AppAction::ToggleTrajectoryHistory),
                        Key::Character("1") => Some(AppAction::SetTimeMultiplier(0.5)),
                        Key::Character("2") => Some(AppAction::SetTimeMultiplier(1.0)),
                        Key::Character("3") => Some(AppAction::SetTimeMultiplier(2.0)),
                        _ => None,
                    };
                    if let Some(action) = action {
                        self.input.actions.push(action);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let viewport_size = self.window.as_ref().map_or(glam::UVec2::ZERO, |window| {
                    let size = window.inner_size();
                    glam::UVec2::new(size.width, size.height)
                });
                let frame = FrameContext {
                    delta_seconds: now.duration_since(self.last_frame).as_secs_f32().min(0.1),
                    elapsed_seconds: now.duration_since(self.started_at).as_secs_f64(),
                    viewport_size,
                };
                self.last_frame = now;
                self.input.movement = self.movement.axis();
                let keep_realtime = self.realtime.as_deref_mut().is_none_or(|realtime| {
                    poll_realtime(
                        self.app.as_mut(),
                        realtime,
                        &mut self.realtime_commands,
                        &mut self.realtime_events,
                    )
                });
                if !keep_realtime {
                    self.realtime = None;
                }
                self.sync_input_modes();
                self.app.update(frame, &self.input);
                self.input.clear_transient();
                self.sync_input_modes();
                if let (Some(window), Some(renderer)) =
                    (self.window.as_ref(), self.renderer.as_mut())
                {
                    match renderer.render(self.app.scene()) {
                        Ok(render_frame) => {
                            window.pre_present_notify();
                            renderer.present(render_frame);
                        }
                        Err(error) => tracing::error!("render failed: {error}"),
                    }
                }
            }
            _ => {}
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.focused = false;
        self.release_cursor();
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        self.sync_input_modes();
        if self.cursor_captured
            && let DeviceEvent::MouseMotion { delta: (dx, dy) } = event
        {
            self.input.look_delta += Vec2::new(dx as f32, dy as f32);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_quality_defaults_match_renderer() {
        let desktop = DesktopConfig::default();
        let renderer = weaver_render_wgpu::WgpuContextConfig::default();
        assert_eq!(desktop.requested_sample_count, 4);
        assert_eq!(desktop.requested_anisotropy, 8);
        assert_eq!(
            desktop.requested_sample_count,
            renderer.requested_sample_count
        );
        assert_eq!(desktop.requested_anisotropy, renderer.requested_anisotropy);
    }
    use std::cell::RefCell;
    use std::rc::Rc;
    use weaver_app_core::AppAssets;
    use weaver_render::SceneSnapshot;

    struct RealtimeTestApp {
        scene: SceneSnapshot,
        assets: AppAssets,
        command_count: usize,
        events: Vec<RealtimeEvent>,
        input_modes: (PointerMode, bool, TextInputMode),
    }

    impl WeaverApp for RealtimeTestApp {
        fn title(&self) -> &'static str {
            "realtime test"
        }

        fn scene(&self) -> &SceneSnapshot {
            &self.scene
        }

        fn scene_mut(&mut self) -> &mut SceneSnapshot {
            &mut self.scene
        }

        fn assets(&self) -> &AppAssets {
            &self.assets
        }

        fn pointer_mode(&self) -> PointerMode {
            self.input_modes.0
        }

        fn gameplay_input_enabled(&self) -> bool {
            self.input_modes.1
        }

        fn text_input_mode(&self) -> TextInputMode {
            self.input_modes.2
        }

        fn handle_realtime_event(&mut self, event: RealtimeEvent) {
            self.events.push(event);
        }

        fn drain_realtime_commands(&mut self, commands: &mut Vec<RealtimeCommand>) {
            commands.extend((0..self.command_count).map(|sequence| {
                RealtimeCommand::PublishReliable {
                    sequence: sequence as u64,
                    payload: String::new(),
                }
            }));
        }
    }

    struct RealtimeTestDriver {
        seen_commands: Rc<RefCell<usize>>,
        event_count: usize,
    }

    impl RealtimeDriver for RealtimeTestDriver {
        fn poll(&mut self, commands: &[RealtimeCommand], events: &mut Vec<RealtimeEvent>) {
            *self.seen_commands.borrow_mut() = commands.len();
            events.extend(
                (0..self.event_count).map(|entity_id| RealtimeEvent::Connected {
                    entity_id: entity_id as u64,
                }),
            );
        }
    }

    #[test]
    fn realtime_poll_caps_commands_and_events() {
        let mut app = RealtimeTestApp {
            scene: SceneSnapshot::default(),
            assets: AppAssets::default(),
            command_count: MAX_REALTIME_COMMANDS_PER_REDRAW + 1,
            events: Vec::new(),
            input_modes: (PointerMode::None, false, TextInputMode::Disabled),
        };
        let seen_commands = Rc::new(RefCell::new(0));
        let mut driver = RealtimeTestDriver {
            seen_commands: Rc::clone(&seen_commands),
            event_count: MAX_REALTIME_EVENTS_PER_REDRAW + 1,
        };
        assert!(!poll_realtime(
            &mut app,
            &mut driver,
            &mut Vec::new(),
            &mut Vec::new(),
        ));

        assert_eq!(*seen_commands.borrow(), MAX_REALTIME_COMMANDS_PER_REDRAW);
        assert_eq!(app.events.len(), MAX_REALTIME_EVENTS_PER_REDRAW);
        assert!(app.events.iter().any(|event| matches!(
            event,
            RealtimeEvent::Disconnected { reason } if reason.contains("command limit")
        )));
        assert!(app.events.iter().any(|event| matches!(
            event,
            RealtimeEvent::Disconnected { reason } if reason.contains("event limit")
        )));
    }

    #[test]
    fn sprint_tracks_both_shift_keys_and_clears_with_movement_on_focus_reset() {
        let mut movement = MovementKeys::default();
        movement.set(PhysicalKey::Code(KeyCode::ShiftLeft), true);
        movement.set(PhysicalKey::Code(KeyCode::ShiftRight), true);
        movement.set(PhysicalKey::Code(KeyCode::ShiftLeft), false);
        assert!(movement.sprint());
        assert_eq!(movement.axis(), Vec2::ZERO);
        movement.set(PhysicalKey::Code(KeyCode::ShiftRight), false);
        assert!(!movement.sprint());
        movement.set(PhysicalKey::Code(KeyCode::KeyW), true);
        movement.set(PhysicalKey::Code(KeyCode::ShiftLeft), true);
        movement.clear();
        assert!(!movement.sprint());
        assert_eq!(movement.axis(), Vec2::ZERO);
    }

    #[test]
    fn crouch_tracks_only_left_control_without_changing_movement_or_sprint() {
        let mut movement = MovementKeys::default();
        movement.set(PhysicalKey::Code(KeyCode::ControlRight), true);
        assert!(!movement.crouch());
        for _ in 0..3 {
            movement.set(PhysicalKey::Code(KeyCode::ControlLeft), true);
            assert!(movement.crouch());
            assert_eq!(movement.axis(), Vec2::ZERO);
            assert!(!movement.sprint());
        }
        movement.set(PhysicalKey::Code(KeyCode::KeyW), true);
        movement.set(PhysicalKey::Code(KeyCode::ShiftLeft), true);
        movement.set(PhysicalKey::Code(KeyCode::ControlRight), false);
        assert!(movement.crouch());
        movement.set(PhysicalKey::Code(KeyCode::ControlLeft), false);
        assert!(!movement.crouch());
        assert_eq!(movement.axis(), Vec2::Y);
        assert!(movement.sprint());
        movement.set(PhysicalKey::Code(KeyCode::ControlLeft), true);
        movement.clear();
        assert!(!movement.crouch());
    }

    #[test]
    fn cursor_release_clears_held_crouch_without_restoring_it_on_capture() {
        let modes = (PointerMode::LockedLook, true, TextInputMode::Disabled);
        let mut shell = DesktopShell::new(input_test_app(modes), DesktopConfig::default(), None);
        shell
            .movement
            .set(PhysicalKey::Code(KeyCode::ControlLeft), true);
        shell.input.crouch = shell.movement.crouch();
        shell.capture_succeeded();
        assert!(shell.input.crouch);
        // Focus loss, suspension, and Escape share this reset path.
        shell.release_cursor();
        assert!(!shell.input.crouch);
        assert!(!shell.movement.crouch());
        assert!(shell.input.controls_reset);
        shell.capture_succeeded();
        shell.input.clear_transient();
        assert!(!shell.input.crouch);
    }

    fn input_test_app(modes: (PointerMode, bool, TextInputMode)) -> Box<dyn WeaverApp> {
        Box::new(RealtimeTestApp {
            scene: SceneSnapshot::default(),
            assets: AppAssets::default(),
            command_count: 0,
            events: Vec::new(),
            input_modes: modes,
        })
    }

    #[test]
    fn shell_mode_changes_and_cursor_release_cancel_pending_controls() {
        let modes = (PointerMode::LockedLook, true, TextInputMode::Available);
        for next in [
            (PointerMode::None, true, TextInputMode::Available),
            (PointerMode::LockedLook, false, TextInputMode::Available),
            (PointerMode::LockedLook, true, TextInputMode::Focused),
        ] {
            let mut shell =
                DesktopShell::new(input_test_app(modes), DesktopConfig::default(), None);
            shell.focused = true;
            shell.cursor_captured = true;
            shell.input.fire.set(true);
            shell.input.reload.set(true);
            shell.movement.set(PhysicalKey::Code(KeyCode::KeyW), true);
            shell
                .movement
                .set(PhysicalKey::Code(KeyCode::ControlLeft), true);
            shell.input.crouch = shell.movement.crouch();
            shell.app = input_test_app(next);
            // Mirrors a mode change after update: reset must survive until the next update.
            shell.input.clear_transient();
            shell.sync_input_modes();
            assert!(shell.input.controls_reset);
            assert_eq!(shell.input.fire, ButtonState::default());
            assert_eq!(shell.input.reload, ButtonState::default());
            assert_eq!(shell.movement.axis(), Vec2::ZERO);
            assert!(!shell.cursor_captured);
            assert!(!shell.input.crouch);
            assert!(!shell.movement.crouch());
            shell.input.clear_transient();
            shell.sync_input_modes();
            assert!(!shell.input.controls_reset);
        }
        let mut shell = DesktopShell::new(input_test_app(modes), DesktopConfig::default(), None);
        shell.focused = true;
        shell.cursor_captured = true;
        shell.input.fire.set(true);
        shell.input.reload.set(true);
        shell.release_cursor();
        assert!(shell.input.controls_reset);
        assert_eq!(shell.input.fire, ButtonState::default());
        assert_eq!(shell.input.reload, ButtonState::default());
        shell.input.text_events.push(TextInputEvent::Open);
        shell.cursor_captured = true;
        let routing = shell.gameplay_routing();
        assert!(!routing.left_button(&mut shell.input, true));
        routing.reload(&mut shell.input, true, false);
        assert_eq!(shell.input.fire, ButtonState::default());
        assert_eq!(shell.input.reload, ButtonState::default());
    }

    fn shooter_routing() -> GameplayRouting {
        GameplayRouting {
            pointer: PointerMode::LockedLook,
            enabled: true,
            focused: true,
            text_focused: false,
            captured: false,
        }
    }

    #[test]
    fn capture_click_is_consumed_then_captured_clicks_fire_without_regrabbing() {
        let mut shell = capture_test_shell(true);
        assert!(shell.left_button(true));
        assert_eq!(shell.input.fire, ButtonState::default());
        // The adapter records capture only after a successful native grab.
        shell.capture_succeeded();
        for _ in 0..3 {
            assert!(!shell.left_button(true));
            assert_eq!(shell.input.fire, ButtonState::default());
            shell.input.clear_transient();
        }
        assert!(shell.blocked_until_left_release);
        // Reload is independent of the left-button barrier.
        shell
            .gameplay_routing()
            .reload(&mut shell.input, true, false);
        assert!(shell.input.reload.held && shell.input.reload.pressed);
        assert!(!shell.left_button(false));
        assert!(!shell.blocked_until_left_release);
        assert_eq!(shell.input.fire, ButtonState::default());
        assert!(!shell.left_button(true));
        assert!(shell.input.fire.held && shell.input.fire.pressed);
        shell.input.clear_transient();
        assert!(shell.input.fire.held);
        assert!(!shell.left_button(false));
        assert!(!shell.input.fire.held && shell.input.fire.released);
    }

    fn capture_test_shell(enabled: bool) -> DesktopShell {
        let mut shell = DesktopShell::new(
            input_test_app((PointerMode::LockedLook, enabled, TextInputMode::Disabled)),
            DesktopConfig::default(),
            None,
        );
        shell.focused = true;
        shell
    }

    #[test]
    fn capture_barrier_resets_cleanly_and_rearms_on_recapture() {
        let mut shell = capture_test_shell(true);
        assert!(shell.left_button(true));
        shell.capture_succeeded();
        shell.release_cursor(); // Also used on focus loss, suspend, and mode cancellation.
        assert!(!shell.blocked_until_left_release);
        assert!(!shell.cursor_captured);
        assert!(shell.input.controls_reset);
        assert_eq!(shell.input.fire, ButtonState::default());
        assert!(!shell.left_button(false));
        assert!(shell.left_button(true));
        shell.capture_succeeded();
        assert!(!shell.left_button(true));
        assert_eq!(shell.input.fire, ButtonState::default());
        assert!(!shell.left_button(false));
        assert!(!shell.left_button(true));
        assert!(shell.input.fire.held && shell.input.fire.pressed);
    }

    #[test]
    fn shell_failed_capture_and_legacy_capture_keep_fire_neutral() {
        for enabled in [true, false] {
            let mut shell = capture_test_shell(enabled);
            // No successful grab: repeated presses only request capture.
            for _ in 0..3 {
                assert!(shell.left_button(true));
                assert!(!shell.blocked_until_left_release);
                assert_eq!(shell.input.fire, ButtonState::default());
            }
            assert!(!shell.left_button(false));
        }
        let mut shell = capture_test_shell(false);
        assert!(shell.left_button(true));
        shell.capture_succeeded();
        assert!(!shell.left_button(true));
        assert!(!shell.left_button(false));
        assert!(!shell.left_button(true));
        shell
            .gameplay_routing()
            .reload(&mut shell.input, true, false);
        assert_eq!(shell.input.fire, ButtonState::default());
        assert_eq!(shell.input.reload, ButtonState::default());
    }

    #[test]
    fn failed_capture_never_fires_and_legacy_apps_only_request_capture() {
        let routing = shooter_routing();
        let mut input = InputFrame::default();
        for _ in 0..2 {
            assert!(routing.left_button(&mut input, true));
            assert!(!routing.left_button(&mut input, false));
            assert_eq!(input.fire, Default::default());
        }
        let legacy = GameplayRouting {
            enabled: false,
            ..routing
        };
        assert!(legacy.left_button(&mut input, true));
        let legacy = GameplayRouting {
            captured: true,
            ..legacy
        };
        assert!(!legacy.left_button(&mut input, true));
        legacy.reload(&mut input, true, false);
        assert_eq!(input.fire, Default::default());
        assert_eq!(input.reload, Default::default());
    }

    #[test]
    fn gameplay_requires_all_gates_and_reload_ignores_repeats() {
        let routing = GameplayRouting {
            captured: true,
            ..shooter_routing()
        };
        for blocked in [
            GameplayRouting {
                focused: false,
                ..routing
            },
            GameplayRouting {
                text_focused: true,
                ..routing
            },
            GameplayRouting {
                pointer: PointerMode::OrbitDrag,
                ..routing
            },
            GameplayRouting {
                pointer: PointerMode::None,
                ..routing
            },
        ] {
            let mut input = InputFrame::default();
            assert!(!blocked.left_button(&mut input, true));
            blocked.reload(&mut input, true, false);
            assert_eq!(input.fire, Default::default());
            assert_eq!(input.reload, Default::default());
        }
        let mut input = InputFrame::default();
        routing.reload(&mut input, true, true);
        assert_eq!(input.reload, Default::default());
        routing.reload(&mut input, true, false);
        assert!(input.reload.held && input.reload.pressed);
        input.clear_transient();
        routing.reload(&mut input, true, true);
        assert!(input.reload.held && !input.reload.pressed);
        routing.reload(&mut input, false, false);
        assert!(input.reload.released && !input.reload.held);
        routing.reload(&mut input, true, false);
        reset_controls(&mut input);
        routing.reload(&mut input, true, true);
        assert_eq!(input.reload, Default::default());
        assert!(input.controls_reset);
    }

    #[test]
    fn cancellation_discards_pending_edges_and_preserves_non_gameplay_events() {
        let mut input = InputFrame {
            movement: Vec2::ONE,
            look_delta: Vec2::ONE,
            zoom_delta: 1.0,
            sprint: true,
            toggle_perspective: true,
            actions: vec![AppAction::TogglePause],
            text_events: vec![TextInputEvent::Open],
            ..InputFrame::default()
        };
        input.fire.set(true);
        input.fire.set(false);
        input.reload.set(true);
        reset_controls(&mut input);
        assert_eq!(input.fire, Default::default());
        assert_eq!(input.reload, Default::default());
        assert_eq!(input.movement, Vec2::ZERO);
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(input.zoom_delta, 0.0);
        assert!(!input.sprint && !input.toggle_perspective);
        assert!(input.controls_reset);
        assert_eq!(input.actions, vec![AppAction::TogglePause]);
        assert_eq!(input.text_events, vec![TextInputEvent::Open]);
        input.clear_transient();
        assert!(!input.controls_reset);
    }

    #[test]
    fn movement_aliases_are_tracked_independently() {
        let mut movement = MovementKeys::default();
        movement.set(PhysicalKey::Code(KeyCode::KeyW), true);
        movement.set(PhysicalKey::Code(KeyCode::ArrowUp), true);
        movement.set(PhysicalKey::Code(KeyCode::KeyW), false);
        assert_eq!(movement.axis(), Vec2::Y);
        movement.set(PhysicalKey::Code(KeyCode::ArrowUp), false);
        assert_eq!(movement.axis(), Vec2::ZERO);
    }
}
