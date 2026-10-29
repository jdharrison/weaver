//! Platform-neutral Weaver application contract and input vocabulary.

#![warn(missing_docs)]

#[cfg(feature = "glb")]
pub mod animated_glb;
#[cfg(feature = "glb")]
pub mod cooked;
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
    /// Complete additional mip levels, excluding the base; empty generates at upload.
    pub mipmaps: Vec<Vec<u8>>,
}

/// One prepared CPU resource for a runtime upload.
///
/// Currently serviced only by the desktop shell. This does not perform file I/O,
/// decoding, scheduling, or game loading. Existing handles use backend replacement
/// semantics; successful completion means registered, not GPU-idle or presented.
#[derive(Debug)]
pub enum AssetUpload {
    /// One indexed mesh, including its optional skin stream.
    Mesh(MeshAsset),
    /// One prepared, straight-alpha sRGB RGBA8 texture.
    Texture(TextureAsset),
}

impl AssetUpload {
    /// Validate a runtime payload without a GPU or payload-sized allocation.
    ///
    /// Runtime meshes are nonempty triangle lists with at most 65,536 vertices,
    /// valid indices, finite vertex data, and valid normalized skin influences.
    /// Combined vertex/index/skin bytes must not exceed 64 MiB. Prepared texture
    /// base pixels must not exceed 16 MiB, and base plus supplied mip levels must
    /// not exceed 64 MiB. Nonempty mip chains must be complete through 1×1.
    /// These are payload/work limits, not a
    /// bound on total application memory, GPU residency, or driver overhead.
    /// Device-specific limits are checked separately by the desktop shell.
    /// Startup [`AppAssets`] are unaffected by these runtime-only limits.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Mesh(mesh) => {
                validate_mesh_upload_sizes(
                    mesh.vertices.len(),
                    mesh.indices.len(),
                    mesh.skin.as_ref().map(Vec::len),
                )?;
                if mesh
                    .indices
                    .iter()
                    .any(|&i| usize::from(i) >= mesh.vertices.len())
                {
                    return Err("mesh index is out of bounds".into());
                }
                if mesh.vertices.iter().any(|v| {
                    v.position
                        .iter()
                        .chain(&v.normal)
                        .chain(&v.color)
                        .chain(&v.uv)
                        .any(|value| !value.is_finite())
                }) {
                    return Err("mesh vertex contains non-finite data".into());
                }
                if let Some(skin) = &mesh.skin {
                    for vertex in skin {
                        let sum: f32 = vertex.weights.iter().sum();
                        if !sum.is_finite()
                            || (sum - 1.0).abs() > 0.001
                            || vertex.weights.iter().any(|w| !w.is_finite() || *w < 0.0)
                            || vertex
                                .joints
                                .iter()
                                .any(|&j| usize::from(j) >= weaver_render::mesh::MAX_SKIN_JOINTS)
                        {
                            return Err("invalid skin influences".into());
                        }
                    }
                }
                Ok(())
            }
            Self::Texture(texture) => {
                validate_texture_upload_size(texture.width, texture.height, texture.rgba.len())?;
                let mut bytes = texture.rgba.len();
                if !texture.mipmaps.is_empty() {
                    let expected = (u32::BITS
                        - texture.width.max(texture.height).leading_zeros()
                        - 1) as usize;
                    if texture.mipmaps.len() != expected {
                        return Err("incomplete or excessive texture mip chain".into());
                    }
                    let (mut width, mut height) = (texture.width, texture.height);
                    for mip in &texture.mipmaps {
                        width = (width / 2).max(1);
                        height = (height / 2).max(1);
                        validate_texture_upload_size(width, height, mip.len())?;
                        bytes = bytes
                            .checked_add(mip.len())
                            .ok_or_else(|| "texture mip byte length overflows".to_string())?;
                    }
                }
                if bytes > MAX_RUNTIME_UPLOAD_BYTES {
                    return Err("texture upload exceeds 64 MiB including mips".into());
                }
                Ok(())
            }
        }
    }
}

const MAX_RUNTIME_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_PREPARED_TEXTURE_BYTES: usize = 16 * 1024 * 1024;

fn validate_mesh_upload_sizes(
    vertices: usize,
    indices: usize,
    skin: Option<usize>,
) -> Result<(), String> {
    let bytes = vertices
        .checked_mul(size_of::<Vertex>())
        .and_then(|bytes| {
            indices
                .checked_mul(size_of::<u16>())
                .and_then(|n| bytes.checked_add(n))
        })
        .and_then(|bytes| {
            skin.unwrap_or(0)
                .checked_mul(size_of::<weaver_render_wgpu::SkinVertex>())
                .and_then(|n| bytes.checked_add(n))
        })
        .ok_or_else(|| "mesh upload byte length overflows".to_string())?;
    if bytes > MAX_RUNTIME_UPLOAD_BYTES {
        return Err("mesh upload exceeds 64 MiB".into());
    }
    if vertices == 0
        || vertices > usize::from(u16::MAX) + 1
        || indices == 0
        || !indices.is_multiple_of(3)
        || u32::try_from(indices).is_err()
    {
        return Err("invalid mesh vertex or triangle index count".into());
    }
    if skin.is_some_and(|count| count != vertices) {
        return Err("skin/vertex count mismatch".into());
    }
    Ok(())
}

fn validate_texture_upload_size(width: u32, height: u32, length: usize) -> Result<(), String> {
    let expected = width
        .checked_mul(4)
        .and_then(|row| usize::try_from(row).ok())
        .and_then(|row| {
            usize::try_from(height)
                .ok()
                .and_then(|height| row.checked_mul(height))
        })
        .filter(|&bytes| isize::try_from(bytes).is_ok())
        .ok_or_else(|| "texture upload byte length overflows".to_string())?;
    if width == 0 || height == 0 || length != expected {
        return Err("invalid texture dimensions or RGBA8 byte length".into());
    }
    if expected > MAX_PREPARED_TEXTURE_BYTES {
        return Err("prepared texture exceeds 16 MiB".into());
    }
    Ok(())
}

/// One owned runtime upload, correlated with an application-chosen identifier.
#[derive(Debug)]
pub struct AssetUploadRequest {
    /// Opaque identifier returned unchanged in the completion callback.
    pub id: u64,
    /// Exactly one prepared resource; the shell drops it after dispatch.
    pub asset: AssetUpload,
}

/// Completion of a runtime upload, including recoverable validation/backend errors.
#[derive(Debug)]
pub struct AssetUploadResult {
    /// Identifier from the consumed request.
    pub id: u64,
    /// Registration success or an error for the application to handle.
    pub result: Result<(), String>,
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
    /// Generic drop-item intent (physical G on opted-in desktop); application-owned behavior.
    pub drop_item: ButtonState,
    /// Ability slot buttons (desktop C/Q/E/X); application-owned behavior.
    pub abilities: [ButtonState; 4],
    /// Generic pause intent (physical P on opted-in desktop); application-owned behavior.
    pub pause: ButtonState,
    /// One-frame request to swap the local first-person viewmodel shoulder.
    /// Presentation remains application-owned and must not affect game rules.
    pub toggle_shoulder: ButtonState,
    /// Generic menu toggle intent (physical B on opted-in desktop); application-owned behavior.
    /// Desktop menu intents work in `None` or captured `LockedLook`, require window focus,
    /// and are suppressed by focused/pending text input. Repeated key presses are ignored.
    pub menu_toggle: ButtonState,
    /// Held scoreboard intent (physical Tab on opted-in desktop); same gates as menu toggle.
    /// Presentation, capture, pause, and game behavior remain application-owned.
    pub scoreboard: ButtonState,
    /// Generic menu confirm intent (physical Enter on opted-in desktop); same gates as toggle.
    /// Text input takes priority: `Available` opens text input and `Focused` submits it.
    pub menu_confirm: ButtonState,
    /// Escape/back intent for applications opting into `WeaverApp::escape_menu_enabled`.
    /// Independent of gameplay opt-in and capture; text editing takes priority.
    pub menu_cancel: ButtonState,
    /// Generic secondary menu intent (physical Backspace on opted-in desktop); same gates as toggle.
    pub menu_secondary: ButtonState,
    /// Generic alternate menu intent (physical M on opted-in desktop); same gates as toggle.
    pub menu_alternate: ButtonState,
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
        self.drop_item.clear_transient();
        for ability in &mut self.abilities {
            ability.clear_transient();
        }
        self.pause.clear_transient();
        self.toggle_shoulder.clear_transient();
        self.menu_toggle.clear_transient();
        self.scoreboard.clear_transient();
        self.menu_confirm.clear_transient();
        self.menu_cancel.clear_transient();
        self.menu_secondary.clear_transient();
        self.menu_alternate.clear_transient();
        self.weapon_slot = None;
        self.controls_reset = false;
        self.actions.clear();
        self.text_events.clear();
    }
}

/// Native window presentation requested by an application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WindowMode {
    /// Ordinary resizable desktop window.
    #[default]
    Windowed,
    /// Borderless fullscreen on the current display.
    Fullscreen,
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
    /// Take at most one prepared runtime resource (default: none).
    ///
    /// Desktop calls this at most once per redraw after `update`, before rendering,
    /// when a renderer is available. Other shells currently do not service it.
    /// Applications own pending work, cancellation, IDs, and retry policy; this
    /// method must not block on I/O or decoding. See [`AssetUpload::validate`] for
    /// runtime limits. No requests are consumed during startup asset registration.
    fn take_asset_upload(&mut self) -> Option<AssetUploadRequest> {
        None
    }
    /// Report the one consumed runtime request before rendering (default: no-op).
    ///
    /// Validation and backend errors are returned here, not treated as fatal shell
    /// errors. The app may publish scene references on success or choose recovery.
    /// Requests queued here are not consumed until a later redraw.
    fn asset_upload_finished(&mut self, _result: AssetUploadResult) {}
    /// Optional texture accepting a platform-owned external image override.
    /// The original uploaded asset remains the idle/unsupported placeholder.
    fn external_image_texture(&self) -> Option<SpriteHandle> {
        None
    }
    /// Requested native window presentation. Desktop applies changes after the
    /// next application update; other adapters may ignore this (default: windowed).
    fn window_mode(&self) -> WindowMode {
        WindowMode::Windowed
    }

    /// Requested pointer behavior.
    fn pointer_mode(&self) -> PointerMode {
        PointerMode::None
    }

    /// Opt into desktop cursor capture on an app-requested `None` -> `LockedLook`
    /// transition (default: disabled). Requires window focus, disabled text input,
    /// and no external controls reset. Focus gain and Escape release do not capture;
    /// a failed native grab retains the usual click-to-capture fallback.
    /// Other platform adapters do not currently implement this policy.
    fn recapture_pointer_on_mode_change(&self) -> bool {
        false
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

    fn upload_test_mesh() -> MeshAsset {
        MeshAsset::new(
            MeshHandle::default(),
            vec![
                Vertex {
                    position: [0.0; 3],
                    normal: [0.0, 1.0, 0.0],
                    color: [1.0; 4],
                    uv: [0.0; 2]
                };
                3
            ],
            vec![0, 1, 2],
        )
    }

    #[test]
    fn runtime_upload_validates_complete_mip_chains() {
        for (width, height) in [(7, 5), (1, 17), (19, 1)] {
            let rgba = vec![127; (width * height * 4) as usize];
            let mipmaps = weaver_render_wgpu::prepare_rgba_mipmaps(width, height, &rgba).unwrap();
            let mut texture = TextureAsset {
                handle: SpriteHandle::default(),
                width,
                height,
                rgba,
                mipmaps,
            };
            assert!(AssetUpload::Texture(texture.clone()).validate().is_ok());
            for level in 0..texture.mipmaps.len() {
                let mut invalid = texture.clone();
                invalid.mipmaps[level].pop();
                assert!(AssetUpload::Texture(invalid).validate().is_err());
            }
            let mut incomplete = texture.clone();
            incomplete.mipmaps.pop();
            assert!(AssetUpload::Texture(incomplete).validate().is_err());
            texture.mipmaps.push(vec![0; 4]);
            assert!(AssetUpload::Texture(texture).validate().is_err());
        }
        let texture = TextureAsset {
            handle: SpriteHandle::default(),
            width: 1,
            height: 1,
            rgba: vec![0; 4],
            mipmaps: vec![vec![0; 4]],
        };
        assert!(AssetUpload::Texture(texture).validate().is_err());
    }

    #[test]
    fn runtime_upload_texture_sizes_and_overflow() {
        assert!(validate_texture_upload_size(1, 1, 4).is_ok());
        assert!(validate_texture_upload_size(2048, 2048, MAX_PREPARED_TEXTURE_BYTES).is_ok());
        for (width, height, length) in [
            (0, 1, 0),
            (1, 0, 0),
            (1, 1, 3),
            (1, 1, 5),
            (u32::MAX, u32::MAX, 0),
        ] {
            assert!(validate_texture_upload_size(width, height, length).is_err());
        }
        assert!(
            validate_texture_upload_size(2048, 2049, 2048 * 2049 * 4)
                .unwrap_err()
                .contains("16 MiB")
        );
        let texture = AssetUpload::Texture(TextureAsset {
            mipmaps: Vec::new(),
            handle: SpriteHandle::default(),
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        });
        assert!(texture.validate().is_ok());
    }

    #[test]
    fn runtime_upload_mesh_counts_limits_and_overflow_without_allocations() {
        assert!(validate_mesh_upload_sizes(65_536, 3, Some(65_536)).is_ok());
        for counts in [
            (0, 3, None),
            (3, 0, None),
            (3, 4, None),
            (65_537, 3, None),
            (3, 3, Some(2)),
            (3, 3, Some(0)),
        ] {
            assert!(validate_mesh_upload_sizes(counts.0, counts.1, counts.2).is_err());
        }
        // Largest triangle-list index payload fitting exactly under the combined cap.
        let indices =
            ((MAX_RUNTIME_UPLOAD_BYTES - 3 * size_of::<Vertex>()) / size_of::<u16>() / 3) * 3;
        assert!(validate_mesh_upload_sizes(3, indices, None).is_ok());
        assert!(
            validate_mesh_upload_sizes(3, indices + 3, None)
                .unwrap_err()
                .contains("64 MiB")
        );
        assert!(
            validate_mesh_upload_sizes(3, indices, Some(3))
                .unwrap_err()
                .contains("64 MiB")
        );
        for counts in [
            (usize::MAX, 3, None),
            (3, usize::MAX, None),
            (3, 3, Some(usize::MAX)),
        ] {
            assert!(
                validate_mesh_upload_sizes(counts.0, counts.1, counts.2)
                    .unwrap_err()
                    .contains("overflows")
            );
        }
    }

    #[test]
    fn runtime_upload_mesh_rejects_invalid_indices_and_nonfinite_vertices() {
        let mesh = upload_test_mesh();
        assert!(AssetUpload::Mesh(mesh.clone()).validate().is_ok());
        let mut invalid = mesh.clone();
        invalid.indices[0] = 3;
        assert!(AssetUpload::Mesh(invalid).validate().is_err());
        let mut invalid = mesh;
        invalid.vertices[0].uv[0] = f32::NAN;
        assert!(AssetUpload::Mesh(invalid).validate().is_err());
    }

    #[test]
    fn runtime_upload_skin_lengths_weights_and_joints() {
        let mut mesh = upload_test_mesh();
        let valid = weaver_render_wgpu::SkinVertex {
            joints: [0; 8],
            weights: [0.125; 8],
        };
        mesh.skin = Some(vec![valid; 3]);
        assert!(AssetUpload::Mesh(mesh.clone()).validate().is_ok());
        mesh.skin = Some(vec![valid; 2]);
        assert!(AssetUpload::Mesh(mesh.clone()).validate().is_err());
        for weight in [f32::NAN, f32::INFINITY, -0.1, 0.0, f32::MAX] {
            let mut invalid = valid;
            invalid.weights[0] = weight;
            mesh.skin = Some(vec![invalid; 3]);
            assert!(AssetUpload::Mesh(mesh.clone()).validate().is_err());
        }
        let mut invalid = valid;
        invalid.joints[7] = u16::try_from(weaver_render::mesh::MAX_SKIN_JOINTS).unwrap();
        mesh.skin = Some(vec![invalid; 3]);
        assert!(AssetUpload::Mesh(mesh).validate().is_err());
    }

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
    fn scoreboard_retains_hold_but_clears_edges() {
        let mut input = InputFrame::default();
        assert_eq!(input.scoreboard, ButtonState::default());
        input.scoreboard.set(true);
        assert!(input.scoreboard.pressed);
        input.clear_transient();
        assert!(input.scoreboard.held);
        assert!(!input.scoreboard.pressed && !input.scoreboard.released);
        input.scoreboard.set(false);
        assert!(!input.scoreboard.held && input.scoreboard.released);
        input.clear_transient();
        assert_eq!(input.scoreboard, ButtonState::default());
    }

    #[test]
    fn menu_buttons_default_neutral_and_clear_only_edges() {
        let mut input = InputFrame::default();
        for button in [
            &mut input.menu_toggle,
            &mut input.menu_confirm,
            &mut input.menu_secondary,
            &mut input.menu_alternate,
        ] {
            assert_eq!(*button, ButtonState::default());
            button.set(true);
            button.set(false);
            button.set(true);
            assert!(button.held && button.pressed && button.released);
        }
        input.clear_transient();
        for button in [
            input.menu_toggle,
            input.menu_confirm,
            input.menu_secondary,
            input.menu_alternate,
        ] {
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
    fn drop_item_defaults_neutral_and_retains_only_held_between_frames() {
        let mut input = InputFrame::default();
        assert_eq!(input.drop_item, ButtonState::default());
        input.drop_item.set(true);
        input.drop_item.set(false);
        assert!(input.drop_item.pressed && input.drop_item.released);
        input.drop_item.set(true);
        input.clear_transient();
        assert_eq!(
            input.drop_item,
            ButtonState {
                held: true,
                pressed: false,
                released: false
            }
        );
        input.drop_item.set(false);
        assert!(input.drop_item.released && !input.drop_item.held);
        input.clear_transient();
        assert_eq!(input.drop_item, ButtonState::default());
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
