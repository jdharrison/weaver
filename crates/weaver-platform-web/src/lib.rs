//! Browser/WASM shell for platform-neutral Weaver applications.

#![warn(missing_docs)]

mod video;

#[cfg(target_arch = "wasm32")]
pub use video::{attach_external_video, detach_external_video, mark_external_video_frame};

use weaver_app_core::WeaverApp;

/// Generate a shared two-adjective display name unless other options are supplied.
///
/// Names are not unique or authenticated. Counts and affixes obey Weaver core's bounds.
///
/// # Errors
///
/// Returns an error for an invalid adjective count or affix.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = GenerateUserName)]
pub fn generate_user_name(
    adjectives: Option<u32>,
    prefix: Option<String>,
    postfix: Option<String>,
) -> Result<String, wasm_bindgen::JsValue> {
    weaver_core::generate_user_name(
        adjectives.unwrap_or(2) as usize,
        prefix.as_deref().unwrap_or(""),
        postfix.as_deref().unwrap_or(""),
    )
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

/// Generate a friendly `Adjective Noun` guest display name.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = GenerateGuestName)]
pub fn generate_guest_name() -> String {
    weaver_core::generate_guest_name()
}

/// Recognize the exact legacy two-distinct-adjective default name format.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = IsLegacyUserName)]
pub fn is_legacy_user_name(name: &str) -> bool {
    weaver_core::is_legacy_user_name(name)
}

/// Generate a fresh UUIDv4; persist and reuse it when a stable local ID is needed.
///
/// This is not an authenticated identity or a Woven entity ID.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = GenerateUserID)]
pub fn generate_user_id() -> String {
    weaver_core::generate_user_id()
}

#[cfg(any(test, target_arch = "wasm32"))]
const MAX_REALTIME_EVENTS: usize = 256;
#[cfg(target_arch = "wasm32")]
const MAX_REALTIME_COMMANDS_PER_REDRAW: usize = 32;
#[cfg(target_arch = "wasm32")]
const MAX_TEXT_EVENTS_PER_REDRAW: usize = 32;
#[cfg(target_arch = "wasm32")]
const MAX_TEXT_EVENT_BYTES: usize = 1_024;

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
    pinch: Option<ActiveTouch>,
}

#[cfg(any(test, target_arch = "wasm32"))]
impl TouchControls {
    const MOVEMENT_RADIUS: f32 = 64.0;
    // Use the existing touch-coordinate scale, matching the pixel-wheel sensitivity.
    const PINCH_PIXELS_PER_STEP: f64 = 50.0;
    const MAX_PINCH_ZOOM_DELTA: f64 = 4.0;

    fn clear(&mut self) {
        self.movement = None;
        self.look = None;
        self.pinch = None;
    }

    fn owns(&self, id: u64) -> bool {
        self.movement.is_some_and(|touch| touch.id == id)
            || self.look.is_some_and(|touch| touch.id == id)
            || self.pinch.is_some_and(|touch| touch.id == id)
    }

    fn sync_mode(
        &mut self,
        mode: weaver_app_core::PointerMode,
        input: &mut weaver_app_core::InputFrame,
    ) {
        if self.mode != Some(mode) {
            if self.mode == Some(weaver_app_core::PointerMode::LockedLook) {
                input.look_delta = glam::Vec2::ZERO;
                input.zoom_delta = 0.0;
            }
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
        input: &mut weaver_app_core::InputFrame,
    ) {
        self.sync_mode(mode, input);
        match phase {
            winit::event::TouchPhase::Started => {
                if mode == weaver_app_core::PointerMode::LockedLook
                    && (!position.is_finite()
                        || !width.is_finite()
                        || width <= 0.0
                        || self.owns(id))
                {
                    return;
                }
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
                        } else if mode == weaver_app_core::PointerMode::LockedLook
                            && self.pinch.is_none()
                        {
                            // Only a second right-side owner can pinch; never steal movement.
                            self.pinch = Some(touch);
                            input.look_delta = glam::Vec2::ZERO;
                        }
                    }
                    weaver_app_core::PointerMode::None => {}
                }
            }
            winit::event::TouchPhase::Moved => {
                if mode == weaver_app_core::PointerMode::LockedLook && !position.is_finite() {
                    return;
                }
                if let Some(touch) = self.movement.as_mut().filter(|touch| touch.id == id) {
                    touch.position = position;
                }
                match (&mut self.look, &mut self.pinch) {
                    (Some(first), Some(second)) if first.id == id || second.id == id => {
                        // f64 distance arithmetic also handles extreme finite f32 coordinates.
                        let before = first
                            .position
                            .as_dvec2()
                            .distance(second.position.as_dvec2());
                        if first.id == id {
                            first.position = position;
                        } else {
                            second.position = position;
                        }
                        let after = first
                            .position
                            .as_dvec2()
                            .distance(second.position.as_dvec2());
                        let zoom = if input.zoom_delta.is_finite() {
                            f64::from(input.zoom_delta)
                        } else {
                            0.0
                        };
                        input.zoom_delta = (zoom + (after - before) / Self::PINCH_PIXELS_PER_STEP)
                            .clamp(-Self::MAX_PINCH_ZOOM_DELTA, Self::MAX_PINCH_ZOOM_DELTA)
                            as f32;
                        input.look_delta = glam::Vec2::ZERO;
                    }
                    (Some(touch), None) if touch.id == id => {
                        input.look_delta += position - touch.position;
                        touch.position = position;
                    }
                    _ => {}
                }
            }
            winit::event::TouchPhase::Ended => {
                if self.movement.is_some_and(|touch| touch.id == id) {
                    self.movement = None;
                }
                if self.look.is_some_and(|touch| touch.id == id) {
                    // Both positions stay current during a pinch, so promotion has no jump.
                    self.look = self.pinch.take();
                } else if self.pinch.is_some_and(|touch| touch.id == id) {
                    self.pinch = None;
                }
            }
            winit::event::TouchPhase::Cancelled => {
                if mode != weaver_app_core::PointerMode::LockedLook || self.owns(id) {
                    self.clear();
                    if mode == weaver_app_core::PointerMode::LockedLook {
                        input.look_delta = glam::Vec2::ZERO;
                        input.zoom_delta = 0.0;
                    }
                }
            }
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

/// Queue a realtime entity entry for delivery before the next application update.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn realtime_entity_entered(entity_id: u64) -> bool {
    enqueue_realtime_event(weaver_app_core::RealtimeEvent::EntityEntered { entity_id })
}

/// Queue an unreliable byte payload without a UTF-8 or JSON conversion.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn realtime_unreliable_payload(entity_id: u64, sequence: u64, payload: &[u8]) -> bool {
    if payload.len() > 65_536 {
        return false;
    }
    enqueue_realtime_event(weaver_app_core::RealtimeEvent::UnreliablePayload {
        entity_id,
        sequence,
        payload: payload.to_vec(),
    })
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
        MAX_REALTIME_COMMANDS_PER_REDRAW, MAX_TEXT_EVENT_BYTES, MAX_TEXT_EVENTS_PER_REDRAW,
        TouchControls, WebConfig, WebError, drain_realtime_events,
    };
    use glam::Vec2;
    use std::sync::Arc;
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::{JsCast, JsValue};
    use weaver_app_core::{
        AppAction, FrameContext, InputFrame, PointerMode, RealtimeCommand, RealtimeEvent,
        TextInputEvent, TextInputMode, WeaverApp,
    };
    use winit::application::ApplicationHandler;
    use winit::event::{
        DeviceEvent, ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
    };
    use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
    use winit::platform::web::{EventLoopExtWebSys, WindowAttributesExtWebSys, WindowExtWebSys};
    use winit::window::{CursorGrabMode, Window, WindowId};

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(
            js_namespace = globalThis,
            js_name = weaverRealtimePublishLatest,
            catch
        )]
        fn weaver_realtime_publish_latest(sequence: u64, payload: &str) -> Result<(), JsValue>;

        #[wasm_bindgen(
            js_namespace = globalThis,
            js_name = weaverRealtimePublishReliable,
            catch
        )]
        fn weaver_realtime_publish_reliable(sequence: u64, payload: &str) -> Result<(), JsValue>;

        #[wasm_bindgen(
                    js_namespace = globalThis,
                    js_name = weaverRealtimePublishUnreliable,
                    catch
                )]
        fn weaver_realtime_publish_unreliable(sequence: u64, payload: &[u8])
        -> Result<(), JsValue>;

        #[wasm_bindgen(
            js_namespace = globalThis,
            js_name = weaverRealtimePublishPositionedUnreliable,
            catch
        )]
        fn weaver_realtime_publish_positioned_unreliable(
            sequence: u64,
            x: f64,
            y: f64,
            z: f64,
            payload: &[u8],
        ) -> Result<(), JsValue>;

        #[wasm_bindgen(
            js_namespace = globalThis,
            js_name = weaverRealtimeFatal,
            catch
        )]
        fn weaver_realtime_fatal(reason: &str) -> Result<(), JsValue>;

        #[wasm_bindgen(js_namespace = globalThis, js_name = weaverSceneReady, catch)]
        fn weaver_scene_ready() -> Result<(), JsValue>;

        #[wasm_bindgen(js_namespace = globalThis, js_name = weaverSceneFatal, catch)]
        fn weaver_scene_fatal(message: &str) -> Result<(), JsValue>;
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
            let result = match command {
                RealtimeCommand::PublishLatest { sequence, payload } => {
                    weaver_realtime_publish_latest(sequence, &payload)
                }
                RealtimeCommand::PublishReliable { sequence, payload } => {
                    weaver_realtime_publish_reliable(sequence, &payload)
                }
                RealtimeCommand::PublishUnreliable { sequence, payload } => {
                    weaver_realtime_publish_unreliable(sequence, &payload)
                }
                RealtimeCommand::PublishPositionedUnreliable {
                    sequence,
                    position,
                    payload,
                } => weaver_realtime_publish_positioned_unreliable(
                    sequence,
                    position[0],
                    position[1],
                    position[2],
                    &payload,
                ),
            };
            if let Err(error) = result {
                let detail = error.as_string().unwrap_or_else(|| format!("{error:?}"));
                let reason = format!("browser realtime publish bridge failed: {detail}");
                let _ = weaver_realtime_fatal(&reason);
                app.handle_realtime_event(RealtimeEvent::Disconnected { reason });
                break;
            }
        }
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
        last_text_input_mode: TextInputMode,
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
                last_text_input_mode: TextInputMode::Disabled,
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
                self.touches.clear();
                self.input.movement = Vec2::ZERO;
                self.input.sprint = false;
                self.input.toggle_perspective = false;
                self.input.zoom_delta = 0.0;
                self.input.look_delta = Vec2::ZERO;
                self.dragging = false;
                self.release_cursor();
            }
            let mode = self.app.text_input_mode();
            if mode != self.last_text_input_mode {
                if let Some(window) = self.window.as_ref() {
                    window.set_ime_allowed(focused);
                }
                self.last_text_input_mode = mode;
            }
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
            super::video::initialize(&renderer, self.app.external_image_texture().is_some());
            renderer
                .context()
                .device
                .on_uncaptured_error(Box::new(|error| {
                    super::video::fail(format!("GPU texture/render error: {error}"));
                }));
            self.renderer = Some(renderer);
            set_status(&format!("Weaver ready on {backend:?}"));
            // Offline lab shells may omit these optional lifecycle callbacks.
            let _ = weaver_scene_ready();
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }

    impl Drop for WebShell {
        fn drop(&mut self) {
            super::video::reset();
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
                    self.input = InputFrame::default();
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
                WindowEvent::MouseWheel { delta, .. }
                    if self.app.pointer_mode() != PointerMode::None
                        && self.app.text_input_mode() != TextInputMode::Focused =>
                {
                    self.input.zoom_delta += match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.02,
                    };
                }
                WindowEvent::Touch(touch) => {
                    if self.app.text_input_mode() == TextInputMode::Focused {
                        self.sync_text_input();
                        return;
                    }
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
                        &mut self.input,
                    );
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
                                self.touches.clear();
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
                        self.release_cursor();
                        return;
                    }
                    self.movement.set(physical_key, pressed);
                    self.input.sprint = self.movement.sprint();
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
                    self.touches.sync_mode(pointer_mode, &mut self.input);
                    self.input.movement =
                        (self.movement.axis() + self.touches.movement_axis()).clamp_length_max(1.0);
                    for event in drain_realtime_events() {
                        self.app.handle_realtime_event(event);
                    }
                    let viewport_size = self.window.as_ref().map_or(glam::UVec2::ZERO, |window| {
                        let size = window.inner_size();
                        glam::UVec2::new(size.width, size.height)
                    });
                    self.app.update(
                        FrameContext {
                            delta_seconds: delta,
                            elapsed_seconds: (now - started) / 1000.0,
                            viewport_size,
                        },
                        &self.input,
                    );
                    self.sync_text_input();
                    publish_realtime_commands(self.app.as_mut(), &mut self.realtime_commands);
                    self.input.clear_transient();
                    if let (Some(window), Some(renderer)) =
                        (self.window.as_ref(), self.renderer.as_mut())
                    {
                        if let Err(error) =
                            super::video::before_render(renderer, self.app.external_image_texture())
                        {
                            report_error(&format!("external video/render failed: {error}"));
                            event_loop.exit();
                            return;
                        }
                        match renderer.render(self.app.scene()) {
                            Ok(frame) => {
                                window.pre_present_notify();
                                renderer.present(frame);
                            }
                            Err(error) if super::recoverable_surface_error(&error) => {
                                if !matches!(
                                    error,
                                    weaver_render_wgpu::WgpuRenderError::SurfaceAcquisitionFailed
                                ) {
                                    let size = window.inner_size();
                                    renderer.resize(size.width, size.height);
                                }
                            }
                            Err(error) => {
                                report_error(&format!("render failed: {error}"));
                                event_loop.exit();
                            }
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
            // The host can acquire pointer lock without a winit left click, and
            // browser Escape can release it without updating our cached flag.
            self.cursor_captured = self.app.pointer_mode() == PointerMode::LockedLook
                && self
                    .window
                    .as_ref()
                    .and_then(|window| window.canvas())
                    .is_some_and(|canvas| {
                        web_sys::window()
                            .and_then(|window| window.document())
                            .and_then(|document| document.pointer_lock_element())
                            .is_some_and(|element| {
                                element == canvas.unchecked_into::<web_sys::Element>()
                            })
                    });
            if self.cursor_captured
                && self.touches.pinch.is_none()
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
        super::video::reset();
        web_sys::console::error_1(&JsValue::from_str(message));
        set_status(message);
        let _ = weaver_scene_fatal(message);
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn recoverable_surface_error(error: &weaver_render_wgpu::WgpuRenderError) -> bool {
    matches!(
        error,
        weaver_render_wgpu::WgpuRenderError::SurfaceAcquisitionFailed
            | weaver_render_wgpu::WgpuRenderError::SurfaceLost
            | weaver_render_wgpu::WgpuRenderError::SurfaceOutOfDate
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use weaver_app_core::{AppAssets, InputFrame, PointerMode, WeaverApp};
    use weaver_render::SceneSnapshot;
    use winit::event::TouchPhase::{self, Cancelled, Ended, Moved, Started};

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
    fn transient_surface_failures_do_not_disable_the_scene() {
        use weaver_render_wgpu::WgpuRenderError;

        for error in [
            WgpuRenderError::SurfaceAcquisitionFailed,
            WgpuRenderError::SurfaceLost,
            WgpuRenderError::SurfaceOutOfDate,
        ] {
            assert!(recoverable_surface_error(&error));
        }
        for error in [
            WgpuRenderError::Surface("out of memory".to_owned()),
            WgpuRenderError::Internal("invalid scene".to_owned()),
            WgpuRenderError::NoAdapter,
        ] {
            assert!(!recoverable_surface_error(&error));
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

    fn locked_touch(
        touches: &mut TouchControls,
        input: &mut InputFrame,
        id: u64,
        phase: TouchPhase,
        position: [f32; 2],
    ) {
        touches.handle(
            PointerMode::LockedLook,
            id,
            phase,
            Vec2::from_array(position),
            200.0,
            input,
        );
    }

    fn start_pinch(touches: &mut TouchControls, input: &mut InputFrame) {
        locked_touch(touches, input, 1, Started, [120.0, 40.0]);
        locked_touch(touches, input, 2, Started, [150.0, 80.0]);
    }

    #[test]
    fn locked_look_touches_split_movement_and_look() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        locked_touch(&mut touches, &mut input, 1, Started, [20.0, 100.0]);
        locked_touch(&mut touches, &mut input, 2, Started, [180.0, 100.0]);
        locked_touch(&mut touches, &mut input, 1, Moved, [200.0, -100.0]);
        locked_touch(&mut touches, &mut input, 2, Moved, [190.0, 120.0]);

        let movement = touches.movement_axis();
        assert!((movement.length() - 1.0).abs() < f32::EPSILON);
        assert!(movement.x > 0.0 && movement.y > 0.0);
        assert_eq!(input.look_delta, Vec2::new(10.0, 20.0));
        assert_eq!(input.zoom_delta, 0.0);
        assert!(touches.pinch.is_none());

        locked_touch(&mut touches, &mut input, 1, Ended, [0.0, 0.0]);
        assert_eq!(touches.movement_axis(), Vec2::ZERO);
        locked_touch(&mut touches, &mut input, 2, Cancelled, [0.0, 0.0]);
        assert!(touches.look.is_none());
    }

    #[test]
    fn right_half_pinch_starts_without_camera_motion() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        locked_touch(&mut touches, &mut input, 1, Started, [120.0, 40.0]);
        locked_touch(&mut touches, &mut input, 1, Moved, [125.0, 45.0]);
        assert_eq!(input.look_delta, Vec2::splat(5.0));
        locked_touch(&mut touches, &mut input, 1, Started, [190.0, 190.0]);
        assert!(touches.pinch.is_none());
        locked_touch(&mut touches, &mut input, 2, Started, [150.0, 80.0]);
        assert!(touches.pinch.is_some());
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(input.zoom_delta, 0.0);

        locked_touch(&mut touches, &mut input, 1, Moved, [125.0, 45.0]);
        locked_touch(&mut touches, &mut input, 2, Moved, [150.0, 80.0]);
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(input.zoom_delta, 0.0);
    }

    #[test]
    fn pinch_apart_zooms_in_and_together_zooms_out_without_look() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        start_pinch(&mut touches, &mut input);
        locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
        touches.sync_mode(PointerMode::LockedLook, &mut input);
        assert_eq!(input.zoom_delta, 1.0);
        assert_eq!(input.look_delta, Vec2::ZERO);

        input.clear_transient();
        locked_touch(&mut touches, &mut input, 1, Moved, [150.0, 80.0]);
        assert_eq!(input.zoom_delta, -1.0);
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(touches.movement_axis(), Vec2::ZERO);

        input.clear_transient();
        locked_touch(&mut touches, &mut input, 1, Moved, [150.0, 80.0]);
        locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
        assert_eq!(input.zoom_delta, 0.0);
    }

    #[test]
    fn pinch_preserves_movement_and_ignores_unowned_touches() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        locked_touch(&mut touches, &mut input, 10, Started, [20.0, 100.0]);
        locked_touch(&mut touches, &mut input, 10, Moved, [52.0, 68.0]);
        start_pinch(&mut touches, &mut input);
        locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
        for (id, phase, position) in [
            (10, Started, [180.0, 120.0]),
            (11, Started, [40.0, 100.0]),
            (3, Started, [180.0, 180.0]),
            (3, Moved, [999.0, -999.0]),
            (3, Cancelled, [0.0, 0.0]),
            (3, Ended, [0.0, 0.0]),
            (11, Moved, [180.0, 180.0]),
            (11, Ended, [0.0, 0.0]),
            (11, Cancelled, [0.0, 0.0]),
        ] {
            locked_touch(&mut touches, &mut input, id, phase, position);
        }
        assert_eq!(touches.movement_axis(), Vec2::splat(0.5));
        assert_eq!(touches.movement.unwrap().id, 10);
        assert_eq!(touches.look.unwrap().id, 1);
        assert_eq!(touches.pinch.unwrap().id, 2);
        assert_eq!(input.zoom_delta, 1.0);
        assert_eq!(input.look_delta, Vec2::ZERO);

        locked_touch(&mut touches, &mut input, 10, Moved, [180.0, 100.0]);
        assert_eq!(touches.movement_axis(), Vec2::X);
        locked_touch(&mut touches, &mut input, 10, Ended, [180.0, 100.0]);
        locked_touch(&mut touches, &mut input, 11, Moved, [20.0, 100.0]);
        assert_eq!(touches.movement_axis(), Vec2::ZERO);
        assert!(touches.pinch.is_some());
        assert_eq!(input.zoom_delta, 1.0);
    }

    #[test]
    fn ending_either_pinch_finger_resumes_look_without_a_jump() {
        for ended_id in [1, 2] {
            let mut touches = TouchControls::default();
            let mut input = InputFrame::default();
            locked_touch(&mut touches, &mut input, 10, Started, [20.0, 100.0]);
            locked_touch(&mut touches, &mut input, 10, Moved, [52.0, 68.0]);
            start_pinch(&mut touches, &mut input);
            locked_touch(&mut touches, &mut input, 1, Moved, [130.0, 50.0]);
            locked_touch(&mut touches, &mut input, 2, Moved, [170.0, 130.0]);
            input.clear_transient();
            locked_touch(&mut touches, &mut input, ended_id, Ended, [f32::NAN; 2]);
            let (remaining_id, position) = if ended_id == 1 {
                (2, [170.0, 130.0])
            } else {
                (1, [130.0, 50.0])
            };
            assert!(touches.pinch.is_none());
            assert_eq!(touches.look.unwrap().id, remaining_id);
            assert_eq!(touches.look.unwrap().position, Vec2::from_array(position));
            assert_eq!(touches.movement_axis(), Vec2::splat(0.5));
            assert_eq!(input.look_delta, Vec2::ZERO);
            assert_eq!(input.zoom_delta, 0.0);

            let position = [position[0] + 3.0, position[1] - 4.0];
            locked_touch(&mut touches, &mut input, remaining_id, Moved, position);
            assert_eq!(input.look_delta, Vec2::new(3.0, -4.0));
            assert_eq!(input.zoom_delta, 0.0);
            locked_touch(&mut touches, &mut input, ended_id, Started, [190.0, 190.0]);
            locked_touch(&mut touches, &mut input, remaining_id, Moved, position);
            assert!(touches.pinch.is_some());
            assert_eq!(input.look_delta, Vec2::ZERO);
            assert_eq!(input.zoom_delta, 0.0);
        }
    }

    #[test]
    fn cancelled_pinch_clears_owners_and_pending_camera_input() {
        for cancelled_id in [1, 2, 10] {
            let mut touches = TouchControls::default();
            let mut input = InputFrame::default();
            locked_touch(&mut touches, &mut input, 10, Started, [20.0, 100.0]);
            locked_touch(&mut touches, &mut input, 10, Moved, [52.0, 68.0]);
            start_pinch(&mut touches, &mut input);
            locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
            assert_eq!(input.zoom_delta, 1.0);
            locked_touch(
                &mut touches,
                &mut input,
                cancelled_id,
                Cancelled,
                [f32::NAN; 2],
            );
            assert!(touches.movement.is_none());
            assert!(touches.look.is_none());
            assert!(touches.pinch.is_none());
            for id in [1, 2, 10] {
                locked_touch(&mut touches, &mut input, id, Moved, [190.0, 190.0]);
                locked_touch(&mut touches, &mut input, id, Ended, [190.0, 190.0]);
            }
            assert_eq!(touches.movement_axis(), Vec2::ZERO);
            assert_eq!(input.look_delta, Vec2::ZERO);
            assert_eq!(input.zoom_delta, 0.0);
            start_pinch(&mut touches, &mut input);
            assert_eq!(input.zoom_delta, 0.0);
            locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
            assert_eq!(input.zoom_delta, 1.0);
        }
    }

    #[test]
    fn touch_clear_discards_pinch_and_ignores_stale_moves() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        locked_touch(&mut touches, &mut input, 10, Started, [20.0, 100.0]);
        start_pinch(&mut touches, &mut input);
        locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
        // Focus loss and text focus clear touch ownership and pending camera input.
        touches.clear();
        input.clear_transient();
        assert!(touches.movement.is_none());
        assert!(touches.look.is_none());
        assert!(touches.pinch.is_none());
        for id in [1, 2, 10] {
            locked_touch(&mut touches, &mut input, id, Moved, [190.0, 190.0]);
        }
        assert_eq!(touches.movement_axis(), Vec2::ZERO);
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(input.zoom_delta, 0.0);
        locked_touch(&mut touches, &mut input, 1, Started, [190.0, 180.0]);
        locked_touch(&mut touches, &mut input, 1, Moved, [188.0, 176.0]);
        assert_eq!(input.look_delta, Vec2::new(-2.0, -4.0));
        assert_eq!(input.zoom_delta, 0.0);
    }

    #[test]
    fn pointer_mode_changes_cancel_pinch_and_ignore_stale_fingers() {
        for mode in [PointerMode::OrbitDrag, PointerMode::None] {
            let mut touches = TouchControls::default();
            let mut input = InputFrame::default();
            locked_touch(&mut touches, &mut input, 10, Started, [20.0, 100.0]);
            start_pinch(&mut touches, &mut input);
            locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
            touches.sync_mode(mode, &mut input);
            assert!(touches.movement.is_none());
            assert!(touches.look.is_none());
            assert!(touches.pinch.is_none());
            for id in [1, 2, 10] {
                touches.handle(mode, id, Moved, Vec2::splat(190.0), 200.0, &mut input);
            }
            assert_eq!(input.look_delta, Vec2::ZERO);
            assert_eq!(input.zoom_delta, 0.0);
            touches.sync_mode(PointerMode::LockedLook, &mut input);
            locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
            assert!(touches.look.is_none());
            start_pinch(&mut touches, &mut input);
            locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
            // Touch events, not only redraws, must observe mode changes.
            touches.handle(mode, 2, Moved, Vec2::splat(190.0), 200.0, &mut input);
            assert!(touches.look.is_none());
            assert!(touches.pinch.is_none());
            assert_eq!(input.look_delta, Vec2::ZERO);
            assert_eq!(input.zoom_delta, 0.0);
        }
    }

    #[test]
    fn orbit_and_none_do_not_pinch_or_promote_unowned_fingers() {
        for mode in [PointerMode::OrbitDrag, PointerMode::None] {
            let mut touches = TouchControls::default();
            let mut input = InputFrame::default();
            for (id, position) in [(1, Vec2::new(120.0, 40.0)), (2, Vec2::new(150.0, 80.0))] {
                touches.handle(mode, id, Started, position, 200.0, &mut input);
            }
            touches.handle(mode, 2, Moved, Vec2::new(180.0, 120.0), 200.0, &mut input);
            touches.handle(mode, 1, Moved, Vec2::new(125.0, 50.0), 200.0, &mut input);
            assert_eq!(
                input.look_delta,
                if mode == PointerMode::OrbitDrag {
                    Vec2::new(5.0, 10.0)
                } else {
                    Vec2::ZERO
                }
            );
            assert_eq!(input.zoom_delta, 0.0);
            assert!(touches.pinch.is_none());
            assert_eq!(touches.movement_axis(), Vec2::ZERO);
            input.clear_transient();
            touches.handle(mode, 1, Ended, Vec2::ZERO, 200.0, &mut input);
            touches.handle(mode, 2, Moved, Vec2::splat(190.0), 200.0, &mut input);
            assert!(touches.look.is_none());
            assert_eq!(input.look_delta, Vec2::ZERO);
            assert_eq!(input.zoom_delta, 0.0);
        }
    }

    #[test]
    fn pinch_zoom_is_finite_and_bounded() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        for (position, width) in [
            ([f32::NAN, 40.0], 200.0),
            ([120.0, f32::INFINITY], 200.0),
            ([120.0, 40.0], 0.0),
            ([120.0, 40.0], -200.0),
            ([120.0, 40.0], f32::INFINITY),
            ([120.0, 40.0], f32::NAN),
        ] {
            touches.handle(
                PointerMode::LockedLook,
                1,
                Started,
                Vec2::from_array(position),
                width,
                &mut input,
            );
            assert!(!touches.owns(1));
        }
        start_pinch(&mut touches, &mut input);
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for id in [1, 2] {
                locked_touch(&mut touches, &mut input, id, Moved, [invalid; 2]);
            }
        }
        assert_eq!(input.zoom_delta, 0.0);
        locked_touch(&mut touches, &mut input, 2, Moved, [180.0, 120.0]);
        assert_eq!(input.zoom_delta, 1.0);
        input.clear_transient();
        for step in 1..=16 {
            locked_touch(
                &mut touches,
                &mut input,
                2,
                Moved,
                [180.0, 120.0 + 50.0 * step as f32],
            );
            assert!(input.zoom_delta.is_finite());
            assert!(f64::from(input.zoom_delta.abs()) <= TouchControls::MAX_PINCH_ZOOM_DELTA);
        }
        assert_eq!(
            f64::from(input.zoom_delta),
            TouchControls::MAX_PINCH_ZOOM_DELTA
        );
        input.clear_transient();
        for step in (0..16).rev() {
            locked_touch(
                &mut touches,
                &mut input,
                2,
                Moved,
                [180.0, 120.0 + 50.0 * step as f32],
            );
        }
        assert_eq!(
            f64::from(input.zoom_delta),
            -TouchControls::MAX_PINCH_ZOOM_DELTA
        );
        for (zoom, position) in [
            (f32::NAN, [f32::MAX; 2]),
            (f32::INFINITY, [-f32::MAX; 2]),
            (f32::NEG_INFINITY, [f32::MAX, -f32::MAX]),
            (f32::MAX, [180.0, 120.0]),
        ] {
            input.zoom_delta = zoom;
            locked_touch(&mut touches, &mut input, 2, Moved, position);
            assert!(input.zoom_delta.is_finite());
            assert!(f64::from(input.zoom_delta.abs()) <= TouchControls::MAX_PINCH_ZOOM_DELTA);
            assert_eq!(input.look_delta, Vec2::ZERO);
        }
    }

    #[test]
    fn orbit_touch_drags_and_clears_on_focus_loss() {
        let mut touches = TouchControls::default();
        let mut input = InputFrame::default();
        touches.handle(
            PointerMode::OrbitDrag,
            7,
            Started,
            Vec2::new(10.0, 20.0),
            200.0,
            &mut input,
        );
        touches.handle(
            PointerMode::OrbitDrag,
            7,
            Moved,
            Vec2::new(15.0, 12.0),
            200.0,
            &mut input,
        );
        assert_eq!(input.look_delta, Vec2::new(5.0, -8.0));
        assert_eq!(input.zoom_delta, 0.0);
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
