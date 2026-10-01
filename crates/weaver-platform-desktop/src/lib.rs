//! Native desktop shell for platform-neutral Weaver applications.

#![warn(missing_docs)]

use glam::Vec2;
use std::sync::Arc;
use std::time::Instant;
use weaver_app_core::{
    AppAction, FrameContext, InputFrame, PointerMode, RealtimeCommand, RealtimeDriver,
    RealtimeEvent, WeaverApp,
};
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
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
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            present_mode: wgpu::PresentMode::AutoVsync,
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
struct MovementKeys(u8);

impl MovementKeys {
    const KEY_W: u8 = 1;
    const ARROW_UP: u8 = 2;
    const KEY_S: u8 = 4;
    const ARROW_DOWN: u8 = 8;
    const KEY_A: u8 = 16;
    const ARROW_LEFT: u8 = 32;
    const KEY_D: u8 = 64;
    const ARROW_RIGHT: u8 = 128;

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

    fn clear(&mut self) {
        self.0 = 0;
    }
}

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
    }
}

impl ApplicationHandler for DesktopShell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
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
        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => {
                self.movement.clear();
                self.dragging = false;
                self.release_cursor();
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
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.zoom_delta += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.02,
                };
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
                PointerMode::LockedLook if state == ElementState::Pressed => {
                    if let Some(window) = self.window.as_ref()
                        && window.set_cursor_grab(CursorGrabMode::Locked).is_ok()
                    {
                        window.set_cursor_visible(false);
                        self.cursor_captured = true;
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
                        ..
                    },
                ..
            } => {
                if state == ElementState::Pressed && logical_key == Key::Named(NamedKey::Escape) {
                    if self.cursor_captured {
                        self.release_cursor();
                    } else {
                        event_loop.exit();
                    }
                    return;
                }
                self.movement
                    .set(physical_key, state == ElementState::Pressed);
                if state == ElementState::Pressed {
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
                let frame = FrameContext {
                    delta_seconds: now.duration_since(self.last_frame).as_secs_f32().min(0.1),
                    elapsed_seconds: now.duration_since(self.started_at).as_secs_f64(),
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
                self.app.update(frame, &self.input);
                self.input.clear_transient();
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

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
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
    use std::cell::RefCell;
    use std::rc::Rc;
    use weaver_app_core::AppAssets;
    use weaver_render::SceneSnapshot;

    struct RealtimeTestApp {
        scene: SceneSnapshot,
        assets: AppAssets,
        command_count: usize,
        events: Vec<RealtimeEvent>,
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

        fn handle_realtime_event(&mut self, event: RealtimeEvent) {
            self.events.push(event);
        }

        fn drain_realtime_commands(&mut self, commands: &mut Vec<RealtimeCommand>) {
            commands.extend(
                (0..self.command_count).map(|sequence| RealtimeCommand::Publish {
                    sequence: sequence as u64,
                    payload: String::new(),
                }),
            );
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
