//! Platform-neutral Weaver application contract and input vocabulary.

#![warn(missing_docs)]

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
}

impl MeshAsset {
    /// Create a mesh asset.
    #[must_use]
    pub fn new(handle: MeshHandle, vertices: Vec<Vertex>, indices: Vec<u16>) -> Self {
        Self {
            handle,
            vertices,
            indices,
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

/// Input accumulated by a platform shell for one frame.
#[derive(Clone, Debug, Default)]
pub struct InputFrame {
    /// Right/forward movement axis in `[-1, 1]`.
    pub movement: Vec2,
    /// Horizontal/vertical look delta in input units.
    pub look_delta: Vec2,
    /// Signed zoom delta.
    pub zoom_delta: f32,
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
        self.actions.clear();
        self.text_events.clear();
    }
}

/// Pointer behavior requested by an application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerMode {
    /// No camera pointer handling.
    None,
    /// Drag to orbit.
    #[default]
    OrbitDrag,
    /// Click to enter relative pointer-lock mouse look.
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
    /// Requested pointer behavior.
    fn pointer_mode(&self) -> PointerMode {
        PointerMode::None
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
    fn transient_input_clears_text_without_changing_movement() {
        let mut input = InputFrame {
            movement: Vec2::Y,
            look_delta: Vec2::ONE,
            zoom_delta: 1.0,
            actions: vec![AppAction::TogglePause],
            text_events: vec![TextInputEvent::Insert("hello".to_owned())],
        };
        input.clear_transient();
        assert_eq!(input.movement, Vec2::Y);
        assert_eq!(input.look_delta, Vec2::ZERO);
        assert_eq!(input.zoom_delta, 0.0);
        assert!(input.actions.is_empty());
        assert!(input.text_events.is_empty());
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
