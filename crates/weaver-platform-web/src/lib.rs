//! Browser/WASM shell for platform-neutral Weaver applications.

#![warn(missing_docs)]

use weaver_app_core::WeaverApp;

#[cfg(any(test, target_arch = "wasm32"))]
const MAX_REALTIME_EVENTS: usize = 256;
#[cfg(target_arch = "wasm32")]
const MAX_REALTIME_COMMANDS_PER_REDRAW: usize = 32;

#[cfg(any(test, target_arch = "wasm32"))]
struct RealtimeInbox {
    events: std::collections::VecDeque<weaver_app_core::RealtimeEvent>,
    overflowed: bool,
}

#[cfg(any(test, target_arch = "wasm32"))]
impl RealtimeInbox {
    fn new() -> Self {
        Self {
            events: std::collections::VecDeque::with_capacity(MAX_REALTIME_EVENTS),
            overflowed: false,
        }
    }

    fn push(&mut self, event: weaver_app_core::RealtimeEvent) -> bool {
        if self.overflowed {
            return false;
        }
        if self.events.len() == MAX_REALTIME_EVENTS {
            self.events.pop_back();
            self.events
                .push_back(weaver_app_core::RealtimeEvent::Disconnected {
                    reason: format!(
                        "realtime inbound event limit exceeded ({MAX_REALTIME_EVENTS} pending)"
                    ),
                });
            self.overflowed = true;
            false
        } else {
            self.events.push_back(event);
            true
        }
    }

    fn drain(&mut self) -> Vec<weaver_app_core::RealtimeEvent> {
        self.overflowed = false;
        self.events.drain(..).collect()
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Clone, Copy)]
struct ActiveTouch {
    id: u64,
    origin: glam::Vec2,
    position: glam::Vec2,
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Default)]
struct TouchControls {
    mode: Option<weaver_app_core::PointerMode>,
    movement: Option<ActiveTouch>,
    look: Option<ActiveTouch>,
}

#[cfg(any(test, target_arch = "wasm32"))]
impl TouchControls {
    const MOVEMENT_RADIUS: f32 = 64.0;

    fn clear(&mut self) {
        self.movement = None;
        self.look = None;
    }

    fn sync_mode(&mut self, mode: weaver_app_core::PointerMode) {
        if self.mode != Some(mode) {
            self.clear();
            self.mode = Some(mode);
        }
    }

    fn handle(
        &mut self,
        mode: weaver_app_core::PointerMode,
        id: u64,
        phase: winit::event::TouchPhase,
        position: glam::Vec2,
        width: f32,
        look_delta: &mut glam::Vec2,
    ) {
        self.sync_mode(mode);
        match phase {
            winit::event::TouchPhase::Started => {
                let touch = ActiveTouch {
                    id,
                    origin: position,
                    position,
                };
                match mode {
                    weaver_app_core::PointerMode::LockedLook if position.x < width * 0.5 => {
                        if self.movement.is_none() {
                            self.movement = Some(touch);
                        }
                    }
                    weaver_app_core::PointerMode::LockedLook
                    | weaver_app_core::PointerMode::OrbitDrag => {
                        if self.look.is_none() {
                            self.look = Some(touch);
                        }
                    }
                    weaver_app_core::PointerMode::None => {}
                }
            }
            winit::event::TouchPhase::Moved => {
                if let Some(touch) = self.movement.as_mut().filter(|touch| touch.id == id) {
                    touch.position = position;
                }
                if let Some(touch) = self.look.as_mut().filter(|touch| touch.id == id) {
                    *look_delta += position - touch.position;
                    touch.position = position;
                }
            }
            winit::event::TouchPhase::Ended => {
                if self.movement.is_some_and(|touch| touch.id == id) {
                    self.movement = None;
                }
                if self.look.is_some_and(|touch| touch.id == id) {
                    self.look = None;
                }
            }
            winit::event::TouchPhase::Cancelled => self.clear(),
        }
    }

    fn movement_axis(&self) -> glam::Vec2 {
        self.movement.map_or(glam::Vec2::ZERO, |touch| {
            let delta = (touch.position - touch.origin) / Self::MOVEMENT_RADIUS;
            glam::Vec2::new(delta.x, -delta.y).clamp_length_max(1.0)
        })
    }
}

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static REALTIME_INBOX: std::cell::RefCell<RealtimeInbox> =
        std::cell::RefCell::new(RealtimeInbox::new());
}

#[cfg(target_arch = "wasm32")]
fn enqueue_realtime_event(event: weaver_app_core::RealtimeEvent) -> bool {
    REALTIME_INBOX.with(|inbox| inbox.borrow_mut().push(event))
}

#[cfg(target_arch = "wasm32")]
fn drain_realtime_events() -> Vec<weaver_app_core::RealtimeEvent> {
    REALTIME_INBOX.with(|inbox| inbox.borrow_mut().drain())
}

/// Queue a realtime connection event for delivery before the next application update.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn realtime_connected(entity_id: u64) -> bool {
    enqueue_realtime_event(weaver_app_core::RealtimeEvent::Connected { entity_id })
}

/// Queue a realtime entity departure for delivery before the next application update.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn realtime_entity_left(entity_id: u64) -> bool {
    enqueue_realtime_event(weaver_app_core::RealtimeEvent::EntityLeft { entity_id })
}

/// Queue a realtime payload for delivery before the next application update.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn realtime_payload(entity_id: u64, sequence: u64, payload: String) -> bool {
    enqueue_realtime_event(weaver_app_core::RealtimeEvent::Payload {
        entity_id,
        sequence,
        payload,
    })
}

/// Queue a realtime disconnect for delivery before the next application update.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn realtime_disconnected(reason: String) -> bool {
    enqueue_realtime_event(weaver_app_core::RealtimeEvent::Disconnected { reason })
}

/// Browser shell configuration.
#[derive(Clone, Debug)]
pub struct WebConfig {
    /// HTML canvas element identifier.
    pub canvas_id: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            canvas_id: "scene".to_owned(),
        }
    }
}

/// Browser shell startup failures.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// Called on a non-browser target.
    #[error("the Weaver web shell requires wasm32-unknown-unknown")]
    UnsupportedPlatform,
    /// Browser event-loop creation failed.
    #[error("browser event loop creation failed: {0}")]
    EventLoop(String),
}

/// Start a platform-neutral application in the browser.
///
/// # Errors
///
/// Returns an error outside WASM or when event-loop creation fails.
pub fn run(app: Box<dyn WeaverApp>, config: WebConfig) -> Result<(), WebError> {
    run_platform(app, config)
}

#[cfg(not(target_arch = "wasm32"))]
fn run_platform(_app: Box<dyn WeaverApp>, _config: WebConfig) -> Result<(), WebError> {
    Err(WebError::UnsupportedPlatform)
}

#[cfg(target_arch = "wasm32")]
fn run_platform(app: Box<dyn WeaverApp>, config: WebConfig) -> Result<(), WebError> {
    browser::run(app, config)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{
        MAX_REALTIME_COMMANDS_PER_REDRAW, TouchControls, WebConfig, WebError, drain_realtime_events,
    };
    use glam::Vec2;
    use std::sync::Arc;
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::{JsCast, JsValue};
    use weaver_app_core::{
        AppAction, FrameContext, InputFrame, PointerMode, RealtimeCommand, RealtimeEvent, WeaverApp,
    };
    use winit::application::ApplicationHandler;
    use winit::event::{
        DeviceEvent, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
    };
    use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
    use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys};
    use winit::window::{CursorGrabMode, Window, WindowId};

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(
            js_namespace = globalThis,
            js_name = weaverRealtimePublish,
            catch
        )]
        fn weaver_realtime_publish(sequence: u64, payload: &str) -> Result<(), JsValue>;

        #[wasm_bindgen(
            js_namespace = globalThis,
            js_name = weaverRealtimeFatal,
            catch
        )]
        fn weaver_realtime_fatal(reason: &str) -> Result<(), JsValue>;
    }

    enum UserEvent {
        RendererReady(Result<weaver_render_wgpu::WgpuRenderer, String>),
    }

    fn publish_realtime_commands(app: &mut dyn WeaverApp, commands: &mut Vec<RealtimeCommand>) {
        commands.clear();
        app.drain_realtime_commands(commands);
        let overflowed = commands.len() > MAX_REALTIME_COMMANDS_PER_REDRAW;
        commands.truncate(MAX_REALTIME_COMMANDS_PER_REDRAW);
        if overflowed {
            let reason = format!(
                "realtime command limit exceeded ({MAX_REALTIME_COMMANDS_PER_REDRAW} per redraw)"
            );
            let _ = weaver_realtime_fatal(&reason);
            app.handle_realtime_event(RealtimeEvent::Disconnected { reason });
            commands.clear();
            return;
        }

        for command in commands.drain(..) {
            let RealtimeCommand::Publish { sequence, payload } = command;
            if let Err(error) = weaver_realtime_publish(sequence, &payload) {
                let detail = error.as_string().unwrap_or_else(|| format!("{error:?}"));
                let reason = format!("weaverRealtimePublish failed: {detail}");
                let _ = weaver_realtime_fatal(&reason);
                app.handle_realtime_event(RealtimeEvent::Disconnected { reason });
                break;
            }
        }
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

    struct WebShell {
        app: Box<dyn WeaverApp>,
        config: WebConfig,
        proxy: EventLoopProxy<UserEvent>,
        window: Option<Arc<Window>>,
        renderer: Option<weaver_render_wgpu::WgpuRenderer>,
        initialized: bool,
        input: InputFrame,
        movement: MovementKeys,
        touches: TouchControls,
        realtime_commands: Vec<RealtimeCommand>,
        cursor_position: Option<Vec2>,
        last_cursor: Option<Vec2>,
        dragging: bool,
        cursor_captured: bool,
        started_millis: Option<f64>,
        last_frame_millis: Option<f64>,
    }

    impl WebShell {
        fn new(
            app: Box<dyn WeaverApp>,
            config: WebConfig,
            proxy: EventLoopProxy<UserEvent>,
        ) -> Self {
            Self {
                app,
                config,
                proxy,
                window: None,
                renderer: None,
                initialized: false,
                input: InputFrame::default(),
                movement: MovementKeys::default(),
                touches: TouchControls::default(),
                realtime_commands: Vec::with_capacity(MAX_REALTIME_COMMANDS_PER_REDRAW),
                cursor_position: None,
                last_cursor: None,
                dragging: false,
                cursor_captured: false,
                started_millis: None,
                last_frame_millis: None,
            }
        }

        fn release_cursor(&mut self) {
            if let Some(window) = self.window.as_ref() {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                window.set_cursor_visible(true);
            }
            self.cursor_captured = false;
        }

        fn install_renderer(
            &mut self,
            event_loop: &ActiveEventLoop,
            result: Result<weaver_render_wgpu::WgpuRenderer, String>,
        ) {
            let mut renderer = match result {
                Ok(renderer) => renderer,
                Err(error) => {
                    report_error(&format!("Weaver GPU initialization failed: {error}"));
                    event_loop.exit();
                    return;
                }
            };
            for mesh in &self.app.assets().meshes {
                if let Err(error) = renderer.upload_mesh(mesh.handle, &mesh.vertices, &mesh.indices)
                {
                    report_error(&format!("mesh upload failed: {error}"));
                    event_loop.exit();
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
                    report_error(&format!("texture upload failed: {error}"));
                    event_loop.exit();
                    return;
                }
            }
            let backend = renderer.context().adapter.get_info().backend;
            self.renderer = Some(renderer);
            set_status(&format!("Weaver ready on {backend:?}"));
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }

    impl ApplicationHandler<UserEvent> for WebShell {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.initialized {
                return;
            }
            self.initialized = true;
            let Some(browser_window) = web_sys::window() else {
                report_error("browser window unavailable");
                event_loop.exit();
                return;
            };
            let Some(document) = browser_window.document() else {
                report_error("browser document unavailable");
                event_loop.exit();
                return;
            };
            let Some(element) = document.get_element_by_id(&self.config.canvas_id) else {
                report_error("configured canvas not found");
                event_loop.exit();
                return;
            };
            let Ok(canvas) = element.dyn_into::<web_sys::HtmlCanvasElement>() else {
                report_error("configured element is not a canvas");
                event_loop.exit();
                return;
            };
            let width = browser_window
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(1280.0);
            let height = browser_window
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(720.0);
            let attributes = Window::default_attributes()
                .with_title(self.app.title())
                .with_inner_size(winit::dpi::LogicalSize::new(width, height))
                .with_canvas(Some(canvas))
                .with_focusable(true)
                .with_prevent_default(true);
            let window = match event_loop.create_window(attributes) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    report_error(&format!("browser window failed: {error}"));
                    event_loop.exit();
                    return;
                }
            };
            self.window = Some(Arc::clone(&window));
            set_status("Initializing Weaver GPU backend…");
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = weaver_render_wgpu::WgpuRenderer::from_window(
                    window,
                    weaver_render_wgpu::WgpuContextConfig::default(),
                )
                .await
                .map_err(|error| error.to_string());
                let _ = proxy.send_event(UserEvent::RendererReady(result));
            });
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
            let UserEvent::RendererReady(result) = event;
            self.install_renderer(event_loop, result);
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _id: WindowId,
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
                    self.touches.clear();
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
                WindowEvent::Touch(touch) => {
                    let width = self
                        .window
                        .as_ref()
                        .map_or(0.0, |window| window.inner_size().width as f32);
                    self.touches.handle(
                        self.app.pointer_mode(),
                        touch.id,
                        touch.phase,
                        Vec2::new(touch.location.x as f32, touch.location.y as f32),
                        width,
                        &mut self.input.look_delta,
                    );
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
                    if state == ElementState::Pressed && logical_key == Key::Named(NamedKey::Escape)
                    {
                        self.release_cursor();
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
                    let now = web_sys::window()
                        .and_then(|window| window.performance())
                        .map_or(0.0, |p| p.now());
                    let started = *self.started_millis.get_or_insert(now);
                    let delta = self
                        .last_frame_millis
                        .replace(now)
                        .map_or(0.0, |last| ((now - last) / 1000.0) as f32)
                        .clamp(0.0, 0.1);
                    let pointer_mode = self.app.pointer_mode();
                    self.touches.sync_mode(pointer_mode);
                    self.input.movement =
                        (self.movement.axis() + self.touches.movement_axis()).clamp_length_max(1.0);
                    for event in drain_realtime_events() {
                        self.app.handle_realtime_event(event);
                    }
                    self.app.update(
                        FrameContext {
                            delta_seconds: delta,
                            elapsed_seconds: (now - started) / 1000.0,
                        },
                        &self.input,
                    );
                    publish_realtime_commands(self.app.as_mut(), &mut self.realtime_commands);
                    self.input.clear_transient();
                    if let (Some(window), Some(renderer)) =
                        (self.window.as_ref(), self.renderer.as_mut())
                    {
                        match renderer.render(self.app.scene()) {
                            Ok(frame) => {
                                window.pre_present_notify();
                                renderer.present(frame);
                            }
                            Err(error) => report_error(&format!("render failed: {error}")),
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

    pub(super) fn run(app: Box<dyn WeaverApp>, config: WebConfig) -> Result<(), WebError> {
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .build()
            .map_err(|error| WebError::EventLoop(error.to_string()))?;
        let shell = WebShell::new(app, config, event_loop.create_proxy());
        event_loop.spawn_app(shell);
        Ok(())
    }

    fn set_status(message: &str) {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        if let Some(element) = document.get_element_by_id("status") {
            element.set_text_content(Some(message));
        }
    }

    fn report_error(message: &str) {
        web_sys::console::error_1(&JsValue::from_str(message));
        set_status(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use weaver_app_core::{AppAssets, WeaverApp};
    use weaver_render::SceneSnapshot;

    struct EmptyApp {
        scene: SceneSnapshot,
        assets: AppAssets,
    }
    impl WeaverApp for EmptyApp {
        fn title(&self) -> &'static str {
            "empty"
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
    }

    #[test]
    fn realtime_inbox_is_bounded_and_reports_overflow() {
        let mut inbox = RealtimeInbox::new();
        for entity_id in 0..=MAX_REALTIME_EVENTS as u64 {
            inbox.push(weaver_app_core::RealtimeEvent::Connected { entity_id });
        }
        inbox.push(weaver_app_core::RealtimeEvent::Connected { entity_id: 999 });

        let events = inbox.drain();
        assert_eq!(events.len(), MAX_REALTIME_EVENTS);
        assert!(matches!(
            events.last(),
            Some(weaver_app_core::RealtimeEvent::Disconnected { reason })
                if reason.contains("limit exceeded")
        ));
    }

    #[test]
    fn locked_look_touches_split_movement_and_look() {
        let mut touches = TouchControls::default();
        let mut look_delta = glam::Vec2::ZERO;
        touches.handle(
            weaver_app_core::PointerMode::LockedLook,
            1,
            winit::event::TouchPhase::Started,
            glam::Vec2::new(20.0, 100.0),
            200.0,
            &mut look_delta,
        );
        touches.handle(
            weaver_app_core::PointerMode::LockedLook,
            2,
            winit::event::TouchPhase::Started,
            glam::Vec2::new(180.0, 100.0),
            200.0,
            &mut look_delta,
        );
        touches.handle(
            weaver_app_core::PointerMode::LockedLook,
            1,
            winit::event::TouchPhase::Moved,
            glam::Vec2::new(200.0, -100.0),
            200.0,
            &mut look_delta,
        );
        touches.handle(
            weaver_app_core::PointerMode::LockedLook,
            2,
            winit::event::TouchPhase::Moved,
            glam::Vec2::new(190.0, 120.0),
            200.0,
            &mut look_delta,
        );

        let movement = touches.movement_axis();
        assert!((movement.length() - 1.0).abs() < f32::EPSILON);
        assert!(movement.x > 0.0 && movement.y > 0.0);
        assert_eq!(look_delta, glam::Vec2::new(10.0, 20.0));

        touches.handle(
            weaver_app_core::PointerMode::LockedLook,
            1,
            winit::event::TouchPhase::Ended,
            glam::Vec2::ZERO,
            200.0,
            &mut look_delta,
        );
        assert_eq!(touches.movement_axis(), glam::Vec2::ZERO);
        touches.handle(
            weaver_app_core::PointerMode::LockedLook,
            2,
            winit::event::TouchPhase::Cancelled,
            glam::Vec2::ZERO,
            200.0,
            &mut look_delta,
        );
        assert!(touches.look.is_none());
    }

    #[test]
    fn orbit_touch_drags_and_clears_on_focus_loss() {
        let mut touches = TouchControls::default();
        let mut look_delta = glam::Vec2::ZERO;
        touches.handle(
            weaver_app_core::PointerMode::OrbitDrag,
            7,
            winit::event::TouchPhase::Started,
            glam::Vec2::new(10.0, 20.0),
            200.0,
            &mut look_delta,
        );
        touches.handle(
            weaver_app_core::PointerMode::OrbitDrag,
            7,
            winit::event::TouchPhase::Moved,
            glam::Vec2::new(15.0, 12.0),
            200.0,
            &mut look_delta,
        );
        assert_eq!(look_delta, glam::Vec2::new(5.0, -8.0));
        touches.clear();
        assert!(touches.look.is_none());
    }

    #[test]
    fn native_calls_report_unsupported_platform() {
        let app = EmptyApp {
            scene: SceneSnapshot::default(),
            assets: AppAssets::default(),
        };
        assert!(matches!(
            run(Box::new(app), WebConfig::default()),
            Err(WebError::UnsupportedPlatform)
        ));
    }
}
