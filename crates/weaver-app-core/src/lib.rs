//! Platform-neutral Weaver application contract and input vocabulary.

#![warn(missing_docs)]

#[cfg(feature = "glb")]
pub mod animated_glb;
#[cfg(feature = "glb")]
pub mod glb;

use glam::{UVec2, Vec2, Vec3};
use weaver_render::{Camera, MeshHandle, SceneSnapshot, SpriteHandle};
use weaver_render_wgpu::Vertex;

/// CPU-side resources uploaded by a platform shell before the first frame.
#[derive(Clone, Debug, Default)]
pub struct AppAssets {
    /// Indexed meshes.
    pub meshes: Vec<MeshAsset>,
    /// RGBA8 textures.
    pub textures: Vec<TextureAsset>,
}

/// CPU-side indexed mesh data.
#[derive(Clone, Debug)]
pub struct MeshAsset {
    /// Renderer resource handle.
    pub handle: MeshHandle,
    /// Mesh vertices.
    pub vertices: Vec<Vertex>,
    /// Triangle-list indices.
    pub indices: Vec<u16>,
    /// Optional eight-influence stream, one entry per vertex.
    pub skin: Option<Vec<weaver_render_wgpu::SkinVertex>>,
}

impl MeshAsset {
    /// Create a mesh asset.
    #[must_use]
    pub fn new(handle: MeshHandle, vertices: Vec<Vertex>, indices: Vec<u16>) -> Self {
        Self {
            handle,
            vertices,
            indices,
            skin: None,
        }
    }
}

/// CPU-side RGBA8 texture data.
#[derive(Clone, Debug)]
pub struct TextureAsset {
    /// Renderer resource handle.
    pub handle: SpriteHandle,
    /// Texture width.
    pub width: u32,
    /// Texture height.
    pub height: u32,
    /// Row-major RGBA8 pixels.
    pub rgba: Vec<u8>,
}

/// Timing information for one rendered application frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameContext {
    /// Wall-clock seconds since the previous frame, capped by the platform shell.
    pub delta_seconds: f32,
    /// Wall-clock seconds since application startup.
    pub elapsed_seconds: f64,
    /// Current physical render-target size in pixels.
    pub viewport_size: UVec2,
}

/// High-level actions shared by desktop and browser shells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppAction {
    /// Toggle simulation pause.
    TogglePause,
    /// Set the simulation time multiplier.
    SetTimeMultiplier(f64),
    /// Toggle coordinate-frame visualization.
    ToggleCoordinateFrames,
    /// Toggle trajectory visualization.
    ToggleTrajectoryHistory,
}

/// Text editing focus requested by an application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextInputMode {
    /// The application does not expose a text editor.
    #[default]
    Disabled,
    /// Text entry is available but does not currently own keyboard input.
    Available,
    /// Text entry owns committed text and editing keys.
    Focused,
}

/// A bounded, platform-normalized text editing event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextInputEvent {
    /// Request that the application's text editor gain focus.
    Open,
    /// Insert committed text at the editor's current insertion point.
    Insert(String),
    /// Delete the previous committed Unicode scalar value.
    Backspace,
    /// Submit the current value.
    Submit,
    /// Cancel editing without submitting.
    Cancel,
}

/// A held button and accumulated transitions since the previous presentation frame.
/// Both edges may be true after a press and release within the same frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonState {
    /// Whether the button is currently down.
    pub held: bool,
    /// Whether an up-to-down transition occurred.
    pub pressed: bool,
    /// Whether a down-to-up transition occurred.
    pub released: bool,
}

impl ButtonState {
    /// Update the held state, accumulating edges; duplicate states do not add edges.
    pub fn set(&mut self, held: bool) {
        self.pressed |= held && !self.held;
        self.released |= !held && self.held;
        self.held = held;
    }

    /// Clear accumulated edges without changing the held state.
    pub fn clear_transient(&mut self) {
        self.pressed = false;
        self.released = false;
    }
}

/// Input accumulated by a platform shell for one frame.
#[derive(Clone, Debug, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent held inputs and frame signals, not mutually exclusive states"
)]
pub struct InputFrame {
    /// Right/forward movement axis in `[-1, 1]`.
    pub movement: Vec2,
    /// Horizontal/vertical look delta in input units.
    pub look_delta: Vec2,
    /// Absolute pointer position in physical viewport pixels, origin at top-left, Y down.
    /// Desktop supplies this while focused and uncaptured; unavailable until cursor motion,
    /// and cleared on cursor leave, capture, focus loss, or pointer-mode changes.
    pub pointer_position: Option<Vec2>,
    /// Primary UI pointer button (desktop left mouse in `PointerMode::None`).
    /// Independent of gameplay opt-in; mode changes cancel edges/holds without a release
    /// edge, and a button held across a transition must be released before another press.
    pub pointer_primary: ButtonState,
    /// Signed zoom delta.
    pub zoom_delta: f32,
    /// Held sprint request; controllers opt in via their sprint multiplier.
    pub sprint: bool,
    /// Held crouch request; mapped to Left Ctrl on desktop, with behavior owned by the application.
    pub crouch: bool,
    /// Held aim-down-sights request; mapped to right mouse for captured desktop gameplay.
    pub aiming_down_sights: bool,
    /// Jump button; mapped to Space for captured gameplay input.
    pub jump: ButtonState,
    /// Primary fire button; currently mapped only by the opted-in desktop adapter.
    pub fire: ButtonState,
    /// Reload button; currently mapped only by the opted-in desktop adapter.
    pub reload: ButtonState,
    /// Generic interact intent (physical F on opted-in desktop); application-owned behavior.
    pub interact: ButtonState,
    /// Generic pause intent (physical P on opted-in desktop); application-owned behavior.
    pub pause: ButtonState,
    /// Generic menu toggle intent (physical B on opted-in desktop); application-owned behavior.
    /// Desktop menu intents work in `None` or captured `LockedLook`, require window focus,
    /// and are suppressed by focused/pending text input. Repeated key presses are ignored.
    pub menu_toggle: ButtonState,
    /// Generic menu confirm intent (physical Enter on opted-in desktop); same gates as toggle.
    /// Text input takes priority: `Available` opens text input and `Focused` submits it.
    pub menu_confirm: ButtonState,
    /// Escape/back intent for applications opting into `WeaverApp::escape_menu_enabled`.
    /// Independent of gameplay opt-in and capture; text editing takes priority.
    pub menu_cancel: ButtonState,
    /// Generic secondary menu intent (physical Backspace on opted-in desktop); same gates as toggle.
    pub menu_secondary: ButtonState,
    /// Last weapon-slot press this frame: slots 1..=10 (desktop digits 1..9, then 0).
    /// Transient intent only; applications own slot availability and selection rules.
    pub weapon_slot: Option<u8>,
    /// Input was cancelled by focus, text-mode, suspend, explicit cursor release,
    /// or gameplay-enable changes. Desktop pointer-mode changes alone do not emit this.
    /// Applications must discard buffered gameplay presses even when no simulation tick runs,
    /// and must also discard their own buffered controls when requesting a pointer-mode change.
    pub controls_reset: bool,
    /// One-frame request to toggle first/third-person presentation.
    pub toggle_perspective: bool,
    /// Discrete actions received since the previous frame.
    pub actions: Vec<AppAction>,
    /// Committed text and editing events received since the previous frame.
    pub text_events: Vec<TextInputEvent>,
}

impl InputFrame {
    /// Clear transient fields while retaining allocated event capacity.
    pub fn clear_transient(&mut self) {
        self.look_delta = Vec2::ZERO;
        self.zoom_delta = 0.0;
        self.toggle_perspective = false;
        self.pointer_primary.clear_transient();
        self.jump.clear_transient();
        self.fire.clear_transient();
        self.reload.clear_transient();
        self.interact.clear_transient();
        self.pause.clear_transient();
        self.menu_toggle.clear_transient();
        self.menu_confirm.clear_transient();
        self.menu_cancel.clear_transient();
        self.menu_secondary.clear_transient();
        self.weapon_slot = None;
        self.controls_reset = false;
        self.actions.clear();
        self.text_events.clear();
    }
}

/// Pointer behavior requested by an application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerMode {
    /// Visible, ungrabbed cursor; desktop routes left mouse to `pointer_primary`, not fire.
    None,
    /// Drag to orbit.
    #[default]
    OrbitDrag,
    /// Click to enter relative pointer-lock mouse look. Desktop consumes the capture click;
    /// returning from another mode requires a fresh click, not automatic capture.
    LockedLook,
}

/// Realtime state delivered to a Weaver application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RealtimeEvent {
    /// A realtime entity connected.
    Connected {
        /// Server-assigned entity identifier.
        entity_id: u64,
    },
    /// A realtime entity left.
    EntityLeft {
        /// Server-assigned entity identifier.
        entity_id: u64,
    },
    /// A realtime entity entered the subscribed space.
    EntityEntered {
        /// Server-assigned entity identifier.
        entity_id: u64,
    },
    /// An unreliable sequenced byte payload was received.
    UnreliablePayload {
        /// Sending entity identifier.
        entity_id: u64,
        /// Application-defined message sequence.
        sequence: u64,
        /// Opaque application bytes; never implicitly decoded as UTF-8.
        payload: Vec<u8>,
    },
    /// An entity payload was received.
    Payload {
        /// Sending entity identifier.
        entity_id: u64,
        /// Application-defined message sequence.
        sequence: u64,
        /// Opaque application payload.
        payload: String,
    },
    /// The native QUIC transport's smoothed round-trip time to the Woven server.
    /// Not one-way latency, peer-pose age, or application/server processing time.
    /// Best-effort telemetry; unsupported transports do not emit this event.
    LatencyUpdated {
        /// Transport RTT in whole microseconds, saturated to `u64::MAX`.
        rtt_micros: u64,
    },
    /// The realtime connection stopped.
    Disconnected {
        /// Human-readable disconnect or failure reason.
        reason: String,
    },
}

/// Realtime work requested by a Weaver application.
#[derive(Clone, Debug, PartialEq)]
pub enum RealtimeCommand {
    /// Publish a replaceable byte payload using unreliable sequenced datagrams.
    PublishUnreliable {
        /// Application-defined message sequence.
        sequence: u64,
        /// Opaque application bytes; never implicitly encoded as JSON.
        payload: Vec<u8>,
    },
    /// Publish replaceable state with atomic 3D routing metadata over an unreliable datagram.
    PublishPositionedUnreliable {
        /// Application-defined message sequence.
        sequence: u64,
        /// Routing position in the configured spatial subspace.
        position: [f64; 3],
        /// Opaque application bytes; never implicitly encoded as JSON.
        payload: Vec<u8>,
    },
    /// Publish replaceable state; an adapter may retain only the newest pending value.
    PublishLatest {
        /// Application-defined message sequence.
        sequence: u64,
        /// Opaque application payload.
        payload: String,
    },
    /// Publish an ordered event that must not be silently overwritten.
    PublishReliable {
        /// Application-defined message sequence.
        sequence: u64,
        /// Opaque application payload.
        payload: String,
    },
}

impl RealtimeCommand {
    /// Return the application-defined message sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::PublishUnreliable { sequence, .. }
            | Self::PublishPositionedUnreliable { sequence, .. }
            | Self::PublishLatest { sequence, .. }
            | Self::PublishReliable { sequence, .. } => *sequence,
        }
    }

    /// Return the opaque application payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        match self {
            Self::PublishUnreliable { payload, .. }
            | Self::PublishPositionedUnreliable { payload, .. } => payload,
            Self::PublishLatest { payload, .. } | Self::PublishReliable { payload, .. } => {
                payload.as_bytes()
            }
        }
    }
}

/// Transport-neutral realtime adapter driven by a platform shell.
pub trait RealtimeDriver {
    /// Process bounded application commands and append newly available events.
    fn poll(&mut self, commands: &[RealtimeCommand], events: &mut Vec<RealtimeEvent>);
}

/// Platform-neutral application implemented by Weaver labs.
pub trait WeaverApp {
    /// Human-readable application title.
    fn title(&self) -> &str;
    /// Current renderer-neutral scene.
    fn scene(&self) -> &SceneSnapshot;
    /// Mutable renderer-neutral scene.
    fn scene_mut(&mut self) -> &mut SceneSnapshot;
    /// CPU-side resources referenced by the scene.
    fn assets(&self) -> &AppAssets;
    /// Optional texture accepting a platform-owned external image override.
    /// The original uploaded asset remains the idle/unsupported placeholder.
    fn external_image_texture(&self) -> Option<SpriteHandle> {
        None
    }
    /// Requested pointer behavior.
    fn pointer_mode(&self) -> PointerMode {
        PointerMode::None
    }

    /// Opt into gameplay buttons (default: disabled).
    /// Currently only the desktop adapter maps these, while focused, captured,
    /// in `LockedLook`, and outside text entry. Other adapters leave them neutral.
    /// Applications own gameplay rules and any fixed-tick edge buffering.
    fn gameplay_input_enabled(&self) -> bool {
        false
    }
    /// Route desktop Escape to `InputFrame::menu_cancel` instead of the legacy
    /// release-cursor/exit shortcut. Text editing retains priority.
    fn escape_menu_enabled(&self) -> bool {
        false
    }
    /// Request a graceful native event-loop exit after the next application update.
    /// Platform shutdown drops the application and its resources normally.
    fn exit_requested(&self) -> bool {
        false
    }
    /// Activate the primary interaction at normalized device coordinates.
    /// Returns whether the application handled the activation.
    fn primary_activation(&mut self, _ndc: Vec2) -> bool {
        false
    }
    /// Current text editing focus requested from the platform shell.
    fn text_input_mode(&self) -> TextInputMode {
        TextInputMode::Disabled
    }
    /// Handle one event from an optional realtime connection.
    fn handle_realtime_event(&mut self, _event: RealtimeEvent) {}
    /// Append commands for an optional realtime connection.
    fn drain_realtime_commands(&mut self, _commands: &mut Vec<RealtimeCommand>) {}
    /// Advance application state by one presentation frame.
    fn update(&mut self, _frame: FrameContext, _input: &InputFrame) {}
}

/// Inclusive world-space bounds for a first-person camera eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBounds {
    /// Minimum position.
    pub min: Vec3,
    /// Maximum position.
    pub max: Vec3,
}

impl CameraBounds {
    /// Create camera bounds.
    #[must_use]
    pub const fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }
}

/// Shared first-person camera controller.
#[derive(Clone, Copy, Debug)]
pub struct FirstPersonController {
    yaw: f32,
    pitch: f32,
    /// Movement speed in world units per second.
    pub move_speed: f32,
    /// Held-sprint speed multiplier; defaults to 1 so existing labs are unchanged.
    pub sprint_multiplier: f32,
    /// Look sensitivity in radians per input unit.
    pub look_sensitivity: f32,
    /// Optional camera bounds.
    pub bounds: Option<CameraBounds>,
}

impl FirstPersonController {
    /// Initialize from an existing camera direction.
    #[must_use]
    pub fn from_camera(camera: &Camera) -> Self {
        let direction = (camera.target - camera.eye).normalize_or_zero();
        let (yaw, pitch) = if direction == Vec3::ZERO {
            (0.0, 0.0)
        } else {
            (direction.x.atan2(-direction.z), direction.y.asin())
        };
        Self {
            yaw,
            pitch,
            move_speed: 4.0,
            sprint_multiplier: 1.0,
            look_sensitivity: 0.0025,
            bounds: None,
        }
    }

    /// Apply one frame of normalized input.
    pub fn update(&mut self, camera: &mut Camera, input: &InputFrame, delta_seconds: f32) {
        self.yaw += input.look_delta.x * self.look_sensitivity;
        self.pitch = (self.pitch - input.look_delta.y * self.look_sensitivity).clamp(-1.55, 1.55);
        let movement = input.movement.clamp_length_max(1.0);
        if movement != Vec2::ZERO {
            let forward = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
            let right = forward.cross(Vec3::Y);
            let next = camera.eye
                + (forward * movement.y + right * movement.x)
                    * self.move_speed.max(0.0)
                    * if input.sprint {
                        self.sprint_multiplier.max(0.0)
                    } else {
                        1.0
                    }
                    * delta_seconds;
            camera.eye = self
                .bounds
                .map_or(next, |bounds| next.clamp(bounds.min, bounds.max));
        }
        camera.target = camera.eye
            + Vec3::new(
                self.yaw.sin() * self.pitch.cos(),
                self.pitch.sin(),
                -self.yaw.cos() * self.pitch.cos(),
            );
    }
}

/// Shared orbit camera controller.
#[derive(Clone, Copy, Debug)]
pub struct OrbitController {
    distance: f32,
    azimuth: f32,
    elevation: f32,
    /// Look sensitivity in radians per input unit.
    pub look_sensitivity: f32,
    /// Zoom sensitivity multiplier.
    pub zoom_sensitivity: f32,
}

impl OrbitController {
    /// Initialize from an existing camera.
    #[must_use]
    pub fn from_camera(camera: &Camera) -> Self {
        let offset = camera.eye - camera.target;
        let distance = offset.length().max(0.001);
        Self {
            distance,
            azimuth: offset.x.atan2(offset.z),
            elevation: (offset.y / distance).asin(),
            look_sensitivity: 0.005,
            zoom_sensitivity: 0.1,
        }
    }

    /// Apply one frame of orbit input around the camera's current target.
    pub fn update(&mut self, camera: &mut Camera, input: &InputFrame) {
        self.azimuth -= input.look_delta.x * self.look_sensitivity;
        self.elevation =
            (self.elevation - input.look_delta.y * self.look_sensitivity).clamp(-1.55, 1.55);
        self.distance *= (1.0 - input.zoom_delta * self.zoom_sensitivity).clamp(0.5, 2.0);
        self.distance = self.distance.clamp(0.1, 10_000.0);
        let x = self.distance * self.elevation.cos() * self.azimuth.sin();
        let y = self.distance * self.elevation.sin();
        let z = self.distance * self.elevation.cos() * self.azimuth.cos();
        camera.eye = camera.target + Vec3::new(x, y, z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crouch_defaults_to_released_and_persists_between_frames() {
        let mut input = InputFrame::default();
        assert!(!input.crouch);
        input.crouch = true;
        for _ in 0..3 {
            input.clear_transient();
            assert!(input.crouch);
        }
        input.crouch = false;
        input.clear_transient();
        assert!(!input.crouch);
    }

    #[test]
    fn transient_input_clears_text_without_changing_movement() {
        let mut input = InputFrame {
            movement: Vec2::Y,
            look_delta: Vec2::ONE,
            zoom_delta: 1.0,
            actions: vec![AppAction::TogglePause],
            text_events: vec![TextInputEvent::Insert("hello".to_owned())],
            sprint: true,
            toggle_perspective: true,
            ..InputFrame::default()
        };
        input.clear_transient();
        assert_eq!(input.movement, Vec2::Y);
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(input.zoom_delta, 0.0);
        assert!(input.sprint);
        assert!(!input.toggle_perspective);
        assert!(input.actions.is_empty());
        assert!(input.text_events.is_empty());
    }

    #[test]
    fn buttons_accumulate_edges_and_preserve_held_between_frames() {
        let mut button = ButtonState::default();
        button.set(true);
        button.set(true);
        assert_eq!(
            button,
            ButtonState {
                held: true,
                pressed: true,
                released: false
            }
        );
        button.clear_transient();
        button.set(true);
        assert_eq!(
            button,
            ButtonState {
                held: true,
                ..ButtonState::default()
            }
        );
        button.set(false);
        button.set(true);
        button.set(false);
        assert_eq!(
            button,
            ButtonState {
                held: false,
                pressed: true,
                released: true
            }
        );
        button.clear_transient();
        assert_eq!(button, ButtonState::default());
    }

    #[test]
    fn menu_buttons_default_neutral_and_clear_only_edges() {
        let mut input = InputFrame::default();
        for button in [
            &mut input.menu_toggle,
            &mut input.menu_confirm,
            &mut input.menu_secondary,
        ] {
            assert_eq!(*button, ButtonState::default());
            button.set(true);
            button.set(false);
            button.set(true);
            assert!(button.held && button.pressed && button.released);
        }
        input.clear_transient();
        for button in [input.menu_toggle, input.menu_confirm, input.menu_secondary] {
            assert!(button.held && !button.pressed && !button.released);
        }
    }

    #[test]
    fn interact_defaults_neutral_and_retains_only_held_between_frames() {
        let mut input = InputFrame::default();
        assert_eq!(input.interact, ButtonState::default());
        input.interact.set(true);
        input.interact.set(false);
        assert!(input.interact.pressed && input.interact.released);
        input.interact.set(true);
        input.clear_transient();
        assert_eq!(
            input.interact,
            ButtonState {
                held: true,
                pressed: false,
                released: false
            }
        );
    }

    #[test]
    fn pause_defaults_neutral_and_retains_only_held_between_frames() {
        let mut input = InputFrame::default();
        assert_eq!(input.pause, ButtonState::default());
        input.pause.set(true);
        input.pause.set(false);
        assert!(input.pause.pressed && input.pause.released);
        input.pause.set(true);
        input.clear_transient();
        assert_eq!(
            input.pause,
            ButtonState {
                held: true,
                pressed: false,
                released: false
            }
        );
    }

    #[test]
    fn pointer_defaults_and_frame_edges() {
        let mut input = InputFrame::default();
        assert_eq!(input.pointer_position, None);
        assert_eq!(input.pointer_primary, ButtonState::default());
        input.pointer_position = Some(Vec2::new(10.5, 20.0));
        input.pointer_primary.set(true);
        input.clear_transient();
        assert_eq!(input.pointer_position, Some(Vec2::new(10.5, 20.0)));
        assert!(input.pointer_primary.held);
        assert!(!input.pointer_primary.pressed && !input.pointer_primary.released);
        input.pointer_primary.set(false);
        assert!(input.pointer_primary.released);
    }

    #[test]
    fn weapon_slot_defaults_to_none_and_clears_between_frames() {
        let mut input = InputFrame::default();
        assert_eq!(input.weapon_slot, None);
        input.weapon_slot = Some(10);
        input.clear_transient();
        assert_eq!(input.weapon_slot, None);
    }

    #[test]
    fn frame_clears_button_edges_and_reset_but_not_held() {
        let mut input = InputFrame {
            controls_reset: true,
            ..InputFrame::default()
        };
        input.fire.set(true);
        input.reload.set(true);
        input.clear_transient();
        assert!(input.fire.held && input.reload.held);
        assert!(!input.fire.pressed && !input.reload.pressed);
        assert!(!input.fire.released && !input.reload.released);
        assert!(!input.controls_reset);
    }

    #[test]
    fn sprint_defaults_to_no_speed_change_and_is_opt_in() {
        let original = Camera {
            eye: Vec3::ZERO,
            target: Vec3::NEG_Z,
            ..Camera::default()
        };
        let mut controller = FirstPersonController::from_camera(&original);
        let input = InputFrame {
            movement: Vec2::Y,
            sprint: true,
            ..InputFrame::default()
        };
        let mut camera = original.clone();
        controller.update(&mut camera, &input, 1.0);
        assert_eq!(camera.eye, Vec3::new(0.0, 0.0, -4.0));
        controller.sprint_multiplier = 2.0;
        camera = original.clone();
        controller.update(&mut camera, &input, 1.0);
        assert_eq!(camera.eye, Vec3::new(0.0, 0.0, -8.0));
        controller.update(
            &mut camera,
            &InputFrame {
                sprint: false,
                ..input
            },
            1.0,
        );
        assert_eq!(camera.eye, Vec3::new(0.0, 0.0, -12.0));
    }

    #[test]
    fn first_person_movement_is_bounded() {
        let mut camera = Camera {
            eye: Vec3::ZERO,
            target: Vec3::NEG_Z,
            ..Camera::default()
        };
        let mut controller = FirstPersonController::from_camera(&camera);
        controller.bounds = Some(CameraBounds::new(Vec3::splat(-1.0), Vec3::splat(1.0)));
        controller.update(
            &mut camera,
            &InputFrame {
                movement: Vec2::Y,
                ..InputFrame::default()
            },
            10.0,
        );
        assert_eq!(camera.eye, Vec3::new(0.0, 0.0, -1.0));
    }
}
