//! Shared first-person multiplayer room for desktop and browser shells.

use glam::{Quat, UVec2, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use weaver_app_core::{
    AppAssets, CameraBounds, FirstPersonController, FrameContext, InputFrame, MeshAsset,
    PointerMode, RealtimeCommand, RealtimeEvent, TextInputEvent, TextInputMode, WeaverApp,
};
use weaver_render::{
    Camera, MeshHandle, MeshInstance, Projection, RenderTransform, SceneSnapshot, TextAnchor,
    TextRun, UiElement, UiRect,
};
use weaver_render_wgpu::Vertex;

const ROOM_HALF_WIDTH: f32 = 6.0;
const ROOM_HALF_DEPTH: f32 = 8.0;
const ROOM_HEIGHT: f32 = 4.0;
const EYE_HEIGHT: f32 = 1.7;
const WALL_CLEARANCE: f32 = 0.3;
const PLAYER_BODY_OFFSET: f32 = 0.82;
const PLAYER_SCALE: f32 = 1.0;
const MESSAGE_VERSION: u8 = 1;
const POSE_VERSION: u8 = 1;
const POSE_PAYLOAD_BYTES: usize = 25;
const PUBLISH_INTERVAL_SECONDS: f64 = 0.1;
const MAX_REMOTE_PLAYERS: usize = 128;
const MAX_DEPARTED_ENTITIES: usize = MAX_REMOTE_PLAYERS;
const MAX_PENDING_PUBLISHES: usize = 32;
const MAX_MESSAGE_BYTES: usize = 2_048;
const MAX_DISPLAY_NAME_CHARS: usize = 24;
const MAX_DISPLAY_NAME_BYTES: usize = 96;
const MAX_CHAT_CHARS: usize = 256;
const MAX_CHAT_BYTES: usize = 1_024;
const MAX_CHAT_HISTORY: usize = 30;
const MAX_VISIBLE_CHAT_LINES: usize = 8;
const DEFAULT_DISPLAY_NAME: &str = "Guest";

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static PENDING_DISPLAY_NAME: std::cell::RefCell<Option<String>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = weaverDisplayNameChanged, catch)]
    fn browser_display_name_changed(value: &str) -> Result<(), wasm_bindgen::JsValue>;
}

/// Set the browser visitor's untrusted display name.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_display_name(value: &str) -> bool {
    let Some(value) = validate_display_name(value) else {
        return false;
    };
    PENDING_DISPLAY_NAME.with(|pending| pending.replace(Some(value)));
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FirstPersonMessage {
    // Receive-only compatibility with clients that published JSON poses.
    #[serde(skip_serializing)]
    Pose {
        version: u8,
        display_name: String,
        position: [f32; 3],
        forward: [f32; 3],
    },
    Profile {
        version: u8,
        display_name: String,
    },
    Chat {
        version: u8,
        display_name: String,
        text: String,
    },
}

#[derive(Clone, Debug)]
struct RemotePlayer {
    displayed_position: Vec3,
    target_position: Vec3,
    forward: Vec3,
    display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChatEntry {
    entity_id: u64,
    display_name: String,
    text: String,
}

/// Enclosed room, local camera, and server-backed remote-player state.
pub struct FirstPersonLab {
    scene: SceneSnapshot,
    assets: AppAssets,
    controller: FirstPersonController,
    avatar_mesh: MeshHandle,
    base_mesh_count: usize,
    local_entity_id: Option<u64>,
    remote_players: BTreeMap<u64, RemotePlayer>,
    // EntityEntered or a validated reliable profile authorizes a peer, never a datagram.
    remote_profiles: BTreeMap<u64, String>,
    departed_entities: BTreeSet<u64>,
    profile_only_admission_closed: bool,
    last_remote_reliable_sequences: BTreeMap<u64, u64>,
    last_remote_pose_sequences: BTreeMap<u64, u64>,
    pending_publishes: VecDeque<RealtimeCommand>,
    profile_pending: bool,
    next_sequence: u64,
    next_publish_at: f64,
    display_name: String,
    chat_open: bool,
    chat_draft: String,
    chat_history: VecDeque<ChatEntry>,
    chat_feedback: Option<String>,
    viewport_size: UVec2,
    network_status: String,
}

impl FirstPersonLab {
    /// Create the enclosed room and local first-person camera.
    #[must_use]
    pub fn new() -> Self {
        Self::with_display_name("")
    }

    /// Create the room with a validated temporary display name.
    #[must_use]
    pub fn with_display_name(display_name: &str) -> Self {
        let display_name = validate_display_name(display_name).unwrap_or_else(|| {
            weaver_core::generate_user_name(2, "", "")
                .expect("default user-name generation options are valid")
        });
        let room_mesh = MeshHandle::new();
        let avatar_mesh = MeshHandle::new();
        let (room_vertices, room_indices) = build_room_mesh();
        let (avatar_vertices, avatar_indices) = build_avatar_mesh();
        let meshes = vec![MeshInstance {
            mesh: room_mesh,
            transform: RenderTransform {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: 1.0,
            },
            color: [1.0; 4],
            emissive: 0.35,
        }];
        let base_mesh_count = meshes.len();
        let scene = SceneSnapshot {
            background_color: [0.01, 0.01, 0.015, 1.0],
            camera: Camera {
                eye: Vec3::new(0.0, EYE_HEIGHT, 5.0),
                target: Vec3::new(0.0, EYE_HEIGHT, 0.0),
                projection: Projection::Perspective {
                    fov_y: 70.0_f32.to_radians(),
                    near: 0.05,
                    far: 50.0,
                },
                ..Camera::default()
            },
            meshes,
            status_lines: vec!["offline · 0 remote".to_owned()],
            ..SceneSnapshot::default()
        };
        let mut controller = FirstPersonController::from_camera(&scene.camera);
        controller.move_speed = 4.0;
        controller.look_sensitivity = 0.0025;
        controller.bounds = Some(CameraBounds::new(
            Vec3::new(
                -ROOM_HALF_WIDTH + WALL_CLEARANCE,
                EYE_HEIGHT,
                -ROOM_HALF_DEPTH + WALL_CLEARANCE,
            ),
            Vec3::new(
                ROOM_HALF_WIDTH - WALL_CLEARANCE,
                EYE_HEIGHT,
                ROOM_HALF_DEPTH - WALL_CLEARANCE,
            ),
        ));

        Self {
            scene,
            assets: AppAssets {
                meshes: vec![
                    MeshAsset::new(room_mesh, room_vertices, room_indices),
                    MeshAsset::new(avatar_mesh, avatar_vertices, avatar_indices),
                ],
                ..AppAssets::default()
            },
            controller,
            avatar_mesh,
            base_mesh_count,
            local_entity_id: None,
            remote_players: BTreeMap::new(),
            remote_profiles: BTreeMap::new(),
            departed_entities: BTreeSet::new(),
            profile_only_admission_closed: false,
            last_remote_reliable_sequences: BTreeMap::new(),
            last_remote_pose_sequences: BTreeMap::new(),
            pending_publishes: VecDeque::with_capacity(MAX_PENDING_PUBLISHES),
            profile_pending: false,
            next_sequence: 0,
            next_publish_at: 0.0,
            display_name,
            chat_open: false,
            chat_draft: String::with_capacity(MAX_CHAT_BYTES),
            chat_history: VecDeque::with_capacity(MAX_CHAT_HISTORY),
            chat_feedback: None,
            viewport_size: UVec2::ZERO,
            network_status: "offline · 0 remote".to_owned(),
        }
    }

    fn apply_display_name(&mut self, display_name: String) {
        if self.display_name == display_name {
            return;
        }
        self.display_name = display_name;
        #[cfg(target_arch = "wasm32")]
        let _ = browser_display_name_changed(&self.display_name);
        self.profile_pending = true;
        self.queue_pending_profile();
        self.refresh_status();
    }

    fn queue_message(&mut self, message: &FirstPersonMessage) -> bool {
        let Ok(payload) = serde_json::to_string(message) else {
            self.chat_feedback = Some("Unable to encode the outgoing message".to_owned());
            return false;
        };
        if payload.len() > MAX_MESSAGE_BYTES {
            self.chat_feedback = Some("Outgoing message exceeded the room limit".to_owned());
            return false;
        }

        if self.pending_publishes.len() == MAX_PENDING_PUBLISHES {
            self.chat_feedback = Some("Chat send queue is full; try again".to_owned());
            return false;
        }
        let Some(sequence) = self.next_sequence.checked_add(1) else {
            self.chat_feedback = Some("Outgoing message sequence is exhausted".to_owned());
            return false;
        };
        self.next_sequence = sequence;
        self.pending_publishes
            .push_back(RealtimeCommand::PublishReliable { sequence, payload });
        true
    }

    fn queue_pending_profile(&mut self) {
        if !self.profile_pending || self.local_entity_id.is_none() {
            return;
        }
        let message = FirstPersonMessage::Profile {
            version: MESSAGE_VERSION,
            display_name: self.display_name.clone(),
        };
        if self.queue_message(&message) {
            self.profile_pending = false;
        }
    }

    fn queue_local_pose(&mut self, elapsed_seconds: f64) {
        if self.local_entity_id.is_none() || elapsed_seconds < self.next_publish_at {
            return;
        }
        self.next_publish_at = elapsed_seconds + PUBLISH_INTERVAL_SECONDS;
        let camera = &self.scene.camera;
        let Some(payload) = encode_pose(camera.eye, camera.target - camera.eye) else {
            "online · local pose is invalid".clone_into(&mut self.network_status);
            return;
        };
        self.pending_publishes.retain(|command| {
            !matches!(command, RealtimeCommand::PublishPositionedUnreliable { .. })
        });
        if self.pending_publishes.len() == MAX_PENDING_PUBLISHES {
            "online · local pose queue is saturated".clone_into(&mut self.network_status);
            return;
        }
        let Some(sequence) = self.next_sequence.checked_add(1) else {
            "online · outgoing message sequence is exhausted".clone_into(&mut self.network_status);
            return;
        };
        self.next_sequence = sequence;
        self.pending_publishes
            .push_back(RealtimeCommand::PublishPositionedUnreliable {
                sequence,
                position: camera.eye.as_dvec3().to_array(),
                payload: payload.to_vec(),
            });
    }

    fn accepts_remote_sequence(
        &self,
        entity_id: u64,
        sequence: u64,
        watermarks: &BTreeMap<u64, u64>,
    ) -> bool {
        Some(entity_id) != self.local_entity_id
            && !self.departed_entities.contains(&entity_id)
            && watermarks
                .get(&entity_id)
                .is_none_or(|last| sequence > *last)
    }

    fn receive_unreliable_pose(&mut self, entity_id: u64, sequence: u64, payload: &[u8]) {
        if !self.remote_profiles.contains_key(&entity_id)
            || !self.accepts_remote_sequence(entity_id, sequence, &self.last_remote_pose_sequences)
        {
            return;
        }
        let Some(pose) = decode_pose(payload) else {
            return;
        };
        if self.receive_pose(entity_id, pose) {
            self.last_remote_pose_sequences.insert(entity_id, sequence);
            self.refresh_status();
        }
    }

    fn receive_message(&mut self, entity_id: u64, sequence: u64, payload: &str) {
        if payload.len() > MAX_MESSAGE_BYTES
            || !self.accepts_remote_sequence(
                entity_id,
                sequence,
                &self.last_remote_reliable_sequences,
            )
        {
            return;
        }
        let Ok(message) = serde_json::from_str::<FirstPersonMessage>(payload) else {
            return;
        };

        let accepted = match message {
            FirstPersonMessage::Pose {
                version,
                display_name,
                position,
                forward,
            } => self.receive_legacy_pose(entity_id, version, &display_name, position, forward),
            FirstPersonMessage::Profile {
                version,
                display_name,
            } => self.receive_profile(entity_id, version, &display_name),
            FirstPersonMessage::Chat {
                version,
                display_name,
                text,
            } => self.receive_chat(entity_id, version, &display_name, &text),
        };
        if accepted {
            self.last_remote_reliable_sequences
                .insert(entity_id, sequence);
            self.refresh_status();
        }
    }

    fn receive_legacy_pose(
        &mut self,
        entity_id: u64,
        version: u8,
        display_name: &str,
        position: [f32; 3],
        forward: [f32; 3],
    ) -> bool {
        let Some(pose) = validate_pose(Vec3::from_array(position), Vec3::from_array(forward))
        else {
            return false;
        };
        self.receive_profile(entity_id, version, display_name) && self.receive_pose(entity_id, pose)
    }

    fn receive_pose(&mut self, entity_id: u64, pose: PlayerPose) -> bool {
        let Some(display_name) = self.remote_profiles.get(&entity_id) else {
            return false;
        };
        if let Some(player) = self.remote_players.get_mut(&entity_id) {
            player.target_position = pose.position;
            player.forward = pose.forward;
        } else if self.remote_players.len() < MAX_REMOTE_PLAYERS {
            self.remote_players.insert(
                entity_id,
                RemotePlayer {
                    displayed_position: pose.position,
                    target_position: pose.position,
                    forward: pose.forward,
                    display_name: display_name.clone(),
                },
            );
        } else {
            return false;
        }
        true
    }

    fn receive_profile(&mut self, entity_id: u64, version: u8, display_name: &str) -> bool {
        if version != MESSAGE_VERSION {
            return false;
        }
        let Some(display_name) = validate_display_name(display_name) else {
            return false;
        };
        if self.departed_entities.contains(&entity_id) {
            return false;
        }
        if !self.remote_profiles.contains_key(&entity_id)
            && (self.local_entity_id.is_none()
                || self.profile_only_admission_closed
                || self.remote_profiles.len() >= MAX_REMOTE_PLAYERS)
        {
            return false;
        }
        // New subscribers receive no initial roster; reliable profiles discover existing peers.
        self.remote_profiles.insert(entity_id, display_name.clone());
        if let Some(player) = self.remote_players.get_mut(&entity_id) {
            player.display_name = display_name;
        }
        true
    }

    fn receive_chat(
        &mut self,
        entity_id: u64,
        version: u8,
        display_name: &str,
        text: &str,
    ) -> bool {
        if version != MESSAGE_VERSION || !self.remote_profiles.contains_key(&entity_id) {
            return false;
        }
        let Some(display_name) = validate_display_name(display_name) else {
            return false;
        };
        let Some(text) = validate_chat_message(text) else {
            return false;
        };
        if !self.receive_profile(entity_id, version, &display_name) {
            return false;
        }
        self.push_chat(ChatEntry {
            entity_id,
            display_name,
            text,
        });
        true
    }

    fn push_chat(&mut self, entry: ChatEntry) {
        if self.chat_history.len() == MAX_CHAT_HISTORY {
            self.chat_history.pop_front();
        }
        self.chat_history.push_back(entry);
    }

    fn handle_text_input(&mut self, events: &[TextInputEvent]) {
        for event in events {
            match event {
                TextInputEvent::Open => {
                    self.chat_open = true;
                    self.chat_feedback = None;
                }
                TextInputEvent::Insert(text) if self.chat_open => {
                    append_bounded_text(&mut self.chat_draft, text);
                }
                TextInputEvent::Backspace if self.chat_open => {
                    self.chat_draft.pop();
                }
                TextInputEvent::Submit if self.chat_open => self.submit_chat(),
                TextInputEvent::Cancel if self.chat_open => {
                    self.chat_draft.clear();
                    self.chat_feedback = None;
                    self.chat_open = false;
                }
                _ => {}
            }
        }
    }

    fn submit_chat(&mut self) {
        let draft = self.chat_draft.trim();
        if draft.is_empty() {
            self.chat_draft.clear();
            self.chat_open = false;
            return;
        }
        if draft == "/name" {
            self.chat_feedback = Some("Usage: /name Your Name".to_owned());
            return;
        }
        if let Some(requested_name) = draft.strip_prefix("/name ") {
            let Some(display_name) = validate_display_name(requested_name) else {
                self.chat_feedback = Some(format!(
                    "Names must be 1–{MAX_DISPLAY_NAME_CHARS} visible characters"
                ));
                return;
            };
            self.apply_display_name(display_name);
            self.chat_draft.clear();
            self.chat_feedback = None;
            self.chat_open = false;
            return;
        }
        if self.local_entity_id.is_none() {
            self.chat_feedback = Some("Connect to Woven before sending chat".to_owned());
            return;
        }
        let Some(text) = validate_chat_message(draft) else {
            self.chat_feedback = Some(format!(
                "Messages must be 1–{MAX_CHAT_CHARS} visible characters"
            ));
            return;
        };
        let message = FirstPersonMessage::Chat {
            version: MESSAGE_VERSION,
            display_name: self.display_name.clone(),
            text: text.clone(),
        };
        if !self.queue_message(&message) {
            return;
        }
        self.push_chat(ChatEntry {
            entity_id: self.local_entity_id.unwrap_or_default(),
            display_name: self.display_name.clone(),
            text,
        });
        self.chat_draft.clear();
        self.chat_feedback = None;
        self.chat_open = false;
    }

    fn update_remote_players(&mut self, delta_seconds: f32) {
        let blend = 1.0 - (-12.0 * delta_seconds.max(0.0)).exp();
        for player in self.remote_players.values_mut() {
            player.displayed_position = player
                .displayed_position
                .lerp(player.target_position, blend);
        }
        self.scene.meshes.truncate(self.base_mesh_count);
        self.scene
            .meshes
            .extend(self.remote_players.iter().map(|(entity_id, player)| {
                avatar_instance(
                    self.avatar_mesh,
                    player.displayed_position,
                    player.forward,
                    player_color(*entity_id),
                )
            }));
    }

    fn rebuild_overlay(&mut self) {
        self.scene.text.clear();
        self.scene.ui.clear();
        let width = self.viewport_size.x as f32;
        let height = self.viewport_size.y as f32;
        if width <= 1.0 || height <= 1.0 {
            return;
        }

        self.scene.text.push(screen_text(
            format!("{} · Enter: chat · /name: rename", self.network_status),
            Vec2::new(16.0, 16.0),
            14.0,
            [0.78, 0.86, 1.0, 1.0],
            TextAnchor::TopLeft,
        ));
        let aspect = width / height;
        for (entity_id, player) in &self.remote_players {
            let head = player.displayed_position + Vec3::Y * 0.35;
            if let Some(position) =
                project_to_screen(&self.scene.camera, aspect, self.viewport_size, head)
            {
                self.scene.text.push(screen_text(
                    player.display_name.clone(),
                    position,
                    15.0,
                    player_color(*entity_id),
                    TextAnchor::BottomCenter,
                ));
            }
        }

        if self.chat_history.is_empty() && !self.chat_open && self.chat_feedback.is_none() {
            return;
        }
        let visible_messages = self.chat_history.len().min(MAX_VISIBLE_CHAT_LINES);
        let extra_lines = usize::from(self.chat_open) + usize::from(self.chat_feedback.is_some());
        let line_count = visible_messages + extra_lines;
        let panel_width = 540.0_f32.min((width - 32.0).max(240.0));
        let panel_height = 24.0 + line_count.max(1) as f32 * 21.0;
        let panel_top = (height - panel_height - 16.0).max(52.0);
        self.scene.ui.push(UiElement::Rect(UiRect {
            position: Vec2::new(16.0, panel_top),
            size: Vec2::new(panel_width, panel_height),
            background: [0.02, 0.03, 0.05, 0.82],
            border_color: [0.34, 0.48, 0.72, 0.8],
            border_width: 1.0,
            corner_radius: 7.0,
            anchor: TextAnchor::TopLeft,
            layer: 20,
            interactive: false,
        }));

        let first = self.chat_history.len().saturating_sub(visible_messages);
        let mut line = 0_usize;
        for entry in self.chat_history.iter().skip(first) {
            let color = player_color(entry.entity_id);
            self.scene.text.push(TextRun {
                text: truncate_visible(&format!("{}: {}", entry.display_name, entry.text), 78),
                position: Vec2::new(28.0, panel_top + 12.0 + line as f32 * 21.0),
                size: 15.0,
                color,
                anchor: TextAnchor::TopLeft,
                bounds: Some(Vec2::new(panel_width - 24.0, 20.0)),
                world_space: false,
                world_height: 1.0,
            });
            line += 1;
        }
        if let Some(feedback) = &self.chat_feedback {
            self.scene.text.push(screen_text(
                feedback.clone(),
                Vec2::new(28.0, panel_top + 12.0 + line as f32 * 21.0),
                14.0,
                [1.0, 0.55, 0.48, 1.0],
                TextAnchor::TopLeft,
            ));
            line += 1;
        }
        if self.chat_open {
            self.scene.text.push(TextRun {
                text: format!("> {}_", self.chat_draft),
                position: Vec2::new(28.0, panel_top + 12.0 + line as f32 * 21.0),
                size: 15.0,
                color: [0.92, 0.96, 1.0, 1.0],
                anchor: TextAnchor::TopLeft,
                bounds: Some(Vec2::new(panel_width - 24.0, 20.0)),
                world_space: false,
                world_height: 1.0,
            });
        }
    }

    fn refresh_status(&mut self) {
        let connection = self
            .local_entity_id
            .map_or("offline".to_owned(), |entity_id| {
                format!("online as {} (entity {entity_id})", self.display_name)
            });
        self.network_status = format!("{connection} · {} remote", self.remote_players.len());
        self.scene.status_lines = vec![self.network_status.clone()];
    }
}

impl Default for FirstPersonLab {
    fn default() -> Self {
        Self::new()
    }
}

impl WeaverApp for FirstPersonLab {
    fn title(&self) -> &'static str {
        "WVR | First-Person Lab | Click for mouse look | WASD / arrows"
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
        if self.chat_open {
            PointerMode::None
        } else {
            PointerMode::LockedLook
        }
    }

    fn text_input_mode(&self) -> TextInputMode {
        if self.chat_open {
            TextInputMode::Focused
        } else {
            TextInputMode::Available
        }
    }

    fn handle_realtime_event(&mut self, event: RealtimeEvent) {
        match event {
            RealtimeEvent::Connected { entity_id } => {
                self.local_entity_id = Some(entity_id);
                self.remote_players.clear();
                self.remote_profiles.clear();
                self.departed_entities.clear();
                self.profile_only_admission_closed = false;
                self.last_remote_reliable_sequences.clear();
                self.last_remote_pose_sequences.clear();
                self.pending_publishes.clear();
                self.next_publish_at = 0.0;

                self.profile_pending = true;
                self.queue_pending_profile();
                self.refresh_status();
            }
            RealtimeEvent::EntityEntered { entity_id } => {
                if Some(entity_id) == self.local_entity_id {
                    return;
                }
                self.departed_entities.remove(&entity_id);
                if self.remote_profiles.contains_key(&entity_id)
                    || self.remote_profiles.len() >= MAX_REMOTE_PLAYERS
                {
                    return;
                }
                self.remote_profiles
                    .insert(entity_id, DEFAULT_DISPLAY_NAME.to_owned());
                // Broadcast our profile again so a late joiner learns existing players' names.
                self.profile_pending = true;
                self.queue_pending_profile();
            }
            RealtimeEvent::EntityLeft { entity_id } => {
                if Some(entity_id) == self.local_entity_id {
                    return;
                }
                if self.departed_entities.len() < MAX_DEPARTED_ENTITIES {
                    self.departed_entities.insert(entity_id);
                } else if !self.departed_entities.contains(&entity_id) {
                    // Never evict tombstones: disable profile-only discovery until reconnect
                    // when bounded history cannot remember another departure safely.
                    self.profile_only_admission_closed = true;
                }
                self.remote_players.remove(&entity_id);
                self.remote_profiles.remove(&entity_id);
                self.last_remote_reliable_sequences.remove(&entity_id);
                self.last_remote_pose_sequences.remove(&entity_id);
                self.refresh_status();
            }
            RealtimeEvent::Payload {
                entity_id,
                sequence,
                payload,
            } => self.receive_message(entity_id, sequence, &payload),
            RealtimeEvent::UnreliablePayload {
                entity_id,
                sequence,
                payload,
            } => self.receive_unreliable_pose(entity_id, sequence, &payload),
            RealtimeEvent::Disconnected { reason } => {
                self.local_entity_id = None;
                self.remote_players.clear();
                self.remote_profiles.clear();
                self.departed_entities.clear();
                self.profile_only_admission_closed = false;
                self.last_remote_reliable_sequences.clear();
                self.last_remote_pose_sequences.clear();
                self.pending_publishes.clear();
                self.profile_pending = false;
                self.network_status = format!("offline · {reason}");
                self.scene.status_lines = vec![self.network_status.clone()];
            }
        }
    }

    fn drain_realtime_commands(&mut self, commands: &mut Vec<RealtimeCommand>) {
        self.queue_pending_profile();
        commands.extend(self.pending_publishes.drain(..));
    }

    fn update(&mut self, frame: FrameContext, input: &InputFrame) {
        #[cfg(target_arch = "wasm32")]
        if let Some(display_name) = PENDING_DISPLAY_NAME.with(std::cell::RefCell::take) {
            self.apply_display_name(display_name);
        }
        self.handle_text_input(&input.text_events);
        if !self.chat_open {
            self.controller
                .update(&mut self.scene.camera, input, frame.delta_seconds);
        }
        self.viewport_size = frame.viewport_size;
        self.update_remote_players(frame.delta_seconds);
        self.queue_pending_profile();
        self.queue_local_pose(frame.elapsed_seconds);
        self.rebuild_overlay();
    }
}

/// Start First-Person Lab in its browser shell.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = globalThis, js_name = weaverSceneFatal, catch)]
        fn scene_fatal(message: &str) -> Result<(), wasm_bindgen::JsValue>;
    }

    std::panic::set_hook(Box::new(|panic| {
        let message = panic.to_string();
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&message));
        let _ = scene_fatal(&message);
    }));
    weaver_platform_web::run(
        Box::new(FirstPersonLab::new()),
        weaver_platform_web::WebConfig::default(),
    )
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayerPose {
    position: Vec3,
    forward: Vec3,
}

fn validate_pose(position: Vec3, forward: Vec3) -> Option<PlayerPose> {
    if !position.is_finite()
        || !forward.is_finite()
        || position.x.abs() > ROOM_HALF_WIDTH
        || position.z.abs() > ROOM_HALF_DEPTH
        || (position.y - EYE_HEIGHT).abs() > 0.25
    {
        return None;
    }
    let length_squared = forward.length_squared();
    // Finite components can still overflow the norm and normalize into a zero direction.
    if !length_squared.is_finite() || length_squared < 0.001 {
        return None;
    }
    Some(PlayerPose {
        position,
        forward: forward / length_squared.sqrt(),
    })
}

fn encode_pose(position: Vec3, forward: Vec3) -> Option<[u8; POSE_PAYLOAD_BYTES]> {
    let pose = validate_pose(position, forward)?;
    let mut payload = [0; POSE_PAYLOAD_BYTES];
    payload[0] = POSE_VERSION;
    for (index, value) in pose
        .position
        .to_array()
        .into_iter()
        .chain(pose.forward.to_array())
        .enumerate()
    {
        let offset = 1 + index * 4;
        payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    Some(payload)
}

fn decode_pose(payload: &[u8]) -> Option<PlayerPose> {
    if payload.len() != POSE_PAYLOAD_BYTES || payload[0] != POSE_VERSION {
        return None;
    }
    let mut values = [0.0; 6];
    for (value, bytes) in values.iter_mut().zip(payload[1..].as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*bytes);
    }
    validate_pose(
        Vec3::new(values[0], values[1], values[2]),
        Vec3::new(values[3], values[4], values[5]),
    )
}

fn validate_display_name(value: &str) -> Option<String> {
    let value = value.trim();
    let character_count = value.chars().count();
    if character_count == 0
        || character_count > MAX_DISPLAY_NAME_CHARS
        || value.len() > MAX_DISPLAY_NAME_BYTES
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_owned())
}

fn validate_chat_message(value: &str) -> Option<String> {
    let value = value.trim();
    let character_count = value.chars().count();
    if character_count == 0
        || character_count > MAX_CHAT_CHARS
        || value.len() > MAX_CHAT_BYTES
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_owned())
}

fn append_bounded_text(target: &mut String, text: &str) {
    let starting_count = target.chars().count();
    for (character_count, character) in
        (starting_count..).zip(text.chars().filter(|character| !character.is_control()))
    {
        if character_count == MAX_CHAT_CHARS {
            break;
        }
        let additional_bytes = character.len_utf8();
        if target.len() + additional_bytes > MAX_CHAT_BYTES {
            break;
        }
        target.push(character);
    }
}

fn truncate_visible(value: &str, max_characters: usize) -> String {
    let mut characters = value.chars();
    let mut visible = characters.by_ref().take(max_characters).collect::<String>();
    if characters.next().is_some() {
        visible.push('…');
    }
    visible
}

fn screen_text(
    text: String,
    position: Vec2,
    size: f32,
    color: [f32; 4],
    anchor: TextAnchor,
) -> TextRun {
    TextRun {
        text,
        position,
        size,
        color,
        anchor,
        bounds: None,
        world_space: false,
        world_height: 1.0,
    }
}

fn project_to_screen(
    camera: &Camera,
    aspect: f32,
    viewport_size: UVec2,
    position: Vec3,
) -> Option<Vec2> {
    let clip = camera.view_projection(aspect) * position.extend(1.0);
    if !clip.is_finite() || clip.w <= 0.0 {
        return None;
    }
    let normalized = clip.truncate() / clip.w;
    if normalized.x.abs() > 1.05
        || normalized.y.abs() > 1.05
        || !(0.0..=1.0).contains(&normalized.z)
    {
        return None;
    }
    Some(Vec2::new(
        (normalized.x * 0.5 + 0.5) * viewport_size.x as f32,
        (0.5 - normalized.y * 0.5) * viewport_size.y as f32,
    ))
}

fn avatar_instance(
    mesh: MeshHandle,
    eye_position: Vec3,
    forward: Vec3,
    color: [f32; 4],
) -> MeshInstance {
    let yaw = forward.x.atan2(-forward.z);
    MeshInstance {
        mesh,
        transform: RenderTransform {
            translation: eye_position - Vec3::Y * PLAYER_BODY_OFFSET,
            rotation: Quat::from_rotation_y(yaw),
            scale: PLAYER_SCALE,
        },
        color,
        emissive: 0.12,
    }
}

fn player_color(entity_id: u64) -> [f32; 4] {
    let hue = (entity_id.wrapping_mul(2_654_435_761) % 360) as f32;
    let radians = hue.to_radians();
    [
        0.55 + 0.35 * radians.sin(),
        0.55 + 0.35 * (radians + 2.1).sin(),
        0.55 + 0.35 * (radians + 4.2).sin(),
        1.0,
    ]
}

fn build_room_mesh() -> (Vec<Vertex>, Vec<u16>) {
    let x = ROOM_HALF_WIDTH;
    let z = ROOM_HALF_DEPTH;
    let y = ROOM_HEIGHT;
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);

    // Winding and normals face into the room because the mesh pipeline culls back faces.
    push_quad(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-x, 0.0, -z),
            Vec3::new(-x, 0.0, z),
            Vec3::new(x, 0.0, z),
            Vec3::new(x, 0.0, -z),
        ],
        Vec3::Y,
        [0.48, 0.50, 0.54, 1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-x, y, z),
            Vec3::new(-x, y, -z),
            Vec3::new(x, y, -z),
            Vec3::new(x, y, z),
        ],
        Vec3::NEG_Y,
        [0.72, 0.73, 0.76, 1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-x, 0.0, -z),
            Vec3::new(x, 0.0, -z),
            Vec3::new(x, y, -z),
            Vec3::new(-x, y, -z),
        ],
        Vec3::Z,
        [0.55, 0.61, 0.68, 1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(x, 0.0, z),
            Vec3::new(-x, 0.0, z),
            Vec3::new(-x, y, z),
            Vec3::new(x, y, z),
        ],
        Vec3::NEG_Z,
        [0.62, 0.57, 0.53, 1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(-x, 0.0, z),
            Vec3::new(-x, 0.0, -z),
            Vec3::new(-x, y, -z),
            Vec3::new(-x, y, z),
        ],
        Vec3::X,
        [0.50, 0.58, 0.55, 1.0],
    );
    push_quad(
        &mut vertices,
        &mut indices,
        [
            Vec3::new(x, 0.0, -z),
            Vec3::new(x, 0.0, z),
            Vec3::new(x, y, z),
            Vec3::new(x, y, -z),
        ],
        Vec3::NEG_X,
        [0.58, 0.53, 0.62, 1.0],
    );

    (vertices, indices)
}

fn build_avatar_mesh() -> (Vec<Vertex>, Vec<u16>) {
    const RADIUS: f32 = 0.32;
    const CYLINDER_HALF_HEIGHT: f32 = 0.5;
    const HEMISPHERE_STACKS: u16 = 6;
    const SECTORS: u16 = 16;

    let vertices_per_ring = usize::from(SECTORS + 1);
    let ring_count = usize::from((HEMISPHERE_STACKS + 1) * 2);
    let mut vertices = Vec::with_capacity(ring_count * vertices_per_ring);
    for hemisphere in 0..2 {
        let center_y = if hemisphere == 0 {
            -CYLINDER_HALF_HEIGHT
        } else {
            CYLINDER_HALF_HEIGHT
        };
        for stack in 0..=HEMISPHERE_STACKS {
            let fraction = f32::from(stack) / f32::from(HEMISPHERE_STACKS);
            let latitude = if hemisphere == 0 {
                -std::f32::consts::FRAC_PI_2 + fraction * std::f32::consts::FRAC_PI_2
            } else {
                fraction * std::f32::consts::FRAC_PI_2
            };
            let ring_radius = RADIUS * latitude.cos();
            let y = center_y + RADIUS * latitude.sin();
            for sector in 0..=SECTORS {
                let longitude = std::f32::consts::TAU * f32::from(sector) / f32::from(SECTORS);
                let normal = Vec3::new(
                    latitude.cos() * longitude.cos(),
                    latitude.sin(),
                    latitude.cos() * longitude.sin(),
                );
                vertices.push(Vertex {
                    position: [
                        ring_radius * longitude.cos(),
                        y,
                        ring_radius * longitude.sin(),
                    ],
                    normal: normal.normalize().to_array(),
                    color: [1.0; 4],
                    uv: [
                        f32::from(sector) / f32::from(SECTORS),
                        f32::midpoint(hemisphere as f32, fraction),
                    ],
                });
            }
        }
    }

    let mut indices = Vec::with_capacity((ring_count - 1) * usize::from(SECTORS) * 6);
    for ring in 0..u16::try_from(ring_count - 1).expect("capsule ring count fits u16") {
        for sector in 0..SECTORS {
            let a = ring * (SECTORS + 1) + sector;
            let b = a + SECTORS + 1;
            indices.extend_from_slice(&[a, b, a + 1, b, b + 1, a + 1]);
        }
    }
    (vertices, indices)
}

fn push_quad(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u16>,
    positions: [Vec3; 4],
    normal: Vec3,
    color: [f32; 4],
) {
    let base = u16::try_from(vertices.len()).expect("room mesh fits in u16 indices");
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (position, uv) in positions.into_iter().zip(uvs) {
        vertices.push(Vertex {
            position: position.to_array(),
            normal: normal.to_array(),
            color,
            uv,
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_starts_with_only_the_room_and_retains_avatar_assets() {
        let app = FirstPersonLab::new();
        assert_eq!(app.scene.meshes.len(), 1);
        assert_eq!(app.base_mesh_count, 1);
        assert_eq!(app.assets.meshes.len(), 2);
        assert_eq!(app.assets.meshes[0].vertices.len(), 24);
        assert_eq!(app.assets.meshes[0].indices.len(), 36);
    }

    #[test]
    fn avatar_mesh_is_a_standing_capsule() {
        let (vertices, indices) = build_avatar_mesh();
        let min_y = vertices
            .iter()
            .map(|vertex| vertex.position[1])
            .fold(f32::INFINITY, f32::min);
        let max_y = vertices
            .iter()
            .map(|vertex| vertex.position[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((min_y + 0.82).abs() < 1e-5);
        assert!((max_y - 0.82).abs() < 1e-5);
        assert!(
            vertices
                .iter()
                .all(|vertex| { (Vec3::from_array(vertex.normal).length() - 1.0).abs() < 1e-5 })
        );
        assert!(
            indices
                .iter()
                .all(|index| usize::from(*index) < vertices.len())
        );
        let (triangles, remainder) = indices.as_chunks::<3>();
        assert!(remainder.is_empty());
        for triangle in triangles {
            let a = Vec3::from_array(vertices[usize::from(triangle[0])].position);
            let b = Vec3::from_array(vertices[usize::from(triangle[1])].position);
            let c = Vec3::from_array(vertices[usize::from(triangle[2])].position);
            let face_normal = (b - a).cross(c - a);
            if face_normal.length_squared() > 1e-8 {
                let normal = Vec3::from_array(vertices[usize::from(triangle[0])].normal);
                assert!(face_normal.dot(normal) > 0.0);
            }
        }
    }

    fn frame(elapsed_seconds: f64) -> FrameContext {
        FrameContext {
            delta_seconds: 1.0 / 60.0,
            elapsed_seconds,
            viewport_size: UVec2::new(1280, 720),
        }
    }

    fn pose_payload() -> Vec<u8> {
        pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, -2.0))
    }

    fn pose_payload_at(position: Vec3) -> Vec<u8> {
        encode_pose(position, -Vec3::Z).unwrap().to_vec()
    }

    fn unchecked_pose_payload(position: [f32; 3], forward: [f32; 3]) -> Vec<u8> {
        let mut payload = vec![POSE_VERSION];
        for value in position.into_iter().chain(forward) {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload
    }

    fn legacy_pose_payload(display_name: &str) -> String {
        serde_json::json!({
            "kind": "pose",
            "version": MESSAGE_VERSION,
            "display_name": display_name,
            "position": [1.0, EYE_HEIGHT, -2.0],
            "forward": [0.0, 0.0, -1.0],
        })
        .to_string()
    }

    fn profile_payload(display_name: &str) -> String {
        serde_json::to_string(&FirstPersonMessage::Profile {
            version: MESSAGE_VERSION,
            display_name: display_name.to_owned(),
        })
        .unwrap()
    }

    fn chat_payload(display_name: &str, text: &str) -> String {
        serde_json::to_string(&FirstPersonMessage::Chat {
            version: MESSAGE_VERSION,
            display_name: display_name.to_owned(),
            text: text.to_owned(),
        })
        .unwrap()
    }

    fn enter_peer(app: &mut FirstPersonLab, entity_id: u64) {
        app.handle_realtime_event(RealtimeEvent::EntityEntered { entity_id });
    }

    #[test]
    fn default_name_uses_two_adjectives_and_survives_reconnection() {
        let mut app = FirstPersonLab::new();
        let name = app.display_name.clone();
        assert_eq!(name.chars().filter(char::is_ascii_uppercase).count(), 2);
        assert!(validate_display_name(&name).is_some());
        assert_ne!(name, DEFAULT_DISPLAY_NAME);
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        assert_eq!(app.display_name, name);
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
        assert_eq!(app.display_name, name);
    }

    #[test]
    fn explicit_names_are_preserved_and_invalid_names_generate_a_default() {
        let mut named = FirstPersonLab::with_display_name("Guest");
        named.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        assert_eq!(named.display_name, "Guest");
        let invalid = FirstPersonLab::with_display_name("bad\nname");
        assert_eq!(
            invalid
                .display_name
                .chars()
                .filter(char::is_ascii_uppercase)
                .count(),
            2
        );
        assert!(validate_display_name(&invalid.display_name).is_some());
    }

    #[test]
    fn names_and_chat_are_strictly_validated() {
        assert_eq!(validate_display_name("  Ada  ").as_deref(), Some("Ada"));
        assert!(validate_display_name("").is_none());
        assert!(validate_display_name("bad\nname").is_none());
        assert!(validate_display_name(&"x".repeat(MAX_DISPLAY_NAME_CHARS + 1)).is_none());
        assert_eq!(validate_chat_message(" hello ").as_deref(), Some("hello"));
        assert!(validate_chat_message("bad\tmessage").is_none());
        assert!(validate_chat_message(&"x".repeat(MAX_CHAT_CHARS + 1)).is_none());
    }

    #[test]
    fn peer_payloads_are_validated_ordered_and_rendered() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &profile_payload("Visitor"));
        app.handle_realtime_event(RealtimeEvent::UnreliablePayload {
            entity_id: 9,
            sequence: 2,
            payload: pose_payload(),
        });
        let stale_position = Vec3::new(4.0, EYE_HEIGHT, 3.0);
        for sequence in [1, 2] {
            app.handle_realtime_event(RealtimeEvent::UnreliablePayload {
                entity_id: 9,
                sequence,
                payload: pose_payload_at(stale_position),
            });
        }
        assert_eq!(app.remote_players.len(), 1);
        assert_eq!(app.remote_players[&9].display_name, "Visitor");
        assert_eq!(
            app.remote_players[&9].target_position,
            Vec3::new(1.0, EYE_HEIGHT, -2.0)
        );
        assert_eq!(app.last_remote_pose_sequences[&9], 2);
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        app.update_remote_players(1.0 / 60.0);
        assert_eq!(app.scene.meshes.len(), app.base_mesh_count + 1);
    }

    #[test]
    fn malformed_messages_do_not_advance_sender_sequence() {
        let mut app = FirstPersonLab::new();
        enter_peer(&mut app, 9);
        app.receive_message(9, 2, "not json");
        app.receive_message(9, 1, &profile_payload("Visitor"));
        app.receive_unreliable_pose(9, 2, b"not a pose");
        app.receive_unreliable_pose(9, 1, &pose_payload());
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        assert_eq!(app.last_remote_pose_sequences[&9], 1);
        assert_eq!(app.remote_players.len(), 1);
    }

    #[test]
    fn connected_player_publishes_binary_unreliable_pose() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.update(frame(1.0), &InputFrame::default());
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        assert_eq!(commands.len(), 2);
        let RealtimeCommand::PublishReliable { payload, .. } = &commands[0] else {
            panic!("profile must use reliable JSON publishing");
        };
        assert!(matches!(
            serde_json::from_str::<FirstPersonMessage>(payload).unwrap(),
            FirstPersonMessage::Profile { version: MESSAGE_VERSION, display_name }
                if display_name == app.display_name
        ));
        let RealtimeCommand::PublishPositionedUnreliable {
            sequence,
            position,
            payload,
        } = &commands[1]
        else {
            panic!("pose must use binary unreliable publishing, never reliable fallback");
        };
        assert_eq!(payload.len(), POSE_PAYLOAD_BYTES);
        assert_eq!(*sequence, 2);
        assert_eq!(*position, app.scene.camera.eye.as_dvec3().to_array());
        assert_eq!(decode_pose(payload).unwrap().position, app.scene.camera.eye);
        assert_eq!(decode_pose(payload).unwrap().forward, -Vec3::Z);
    }

    #[test]
    fn reliable_chat_is_not_overwritten_by_newer_pose() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.queue_local_pose(1.0);
        app.chat_open = true;
        app.chat_draft = "hello".to_owned();
        app.submit_chat();
        app.queue_local_pose(1.2);

        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        assert_eq!(commands.len(), 3);
        let RealtimeCommand::PublishReliable {
            payload: profile, ..
        } = &commands[0]
        else {
            panic!("profile must not be overwritten by poses");
        };
        assert!(matches!(
            serde_json::from_str::<FirstPersonMessage>(profile).unwrap(),
            FirstPersonMessage::Profile { .. }
        ));
        let RealtimeCommand::PublishReliable { payload: chat, .. } = &commands[1] else {
            panic!("chat must remain reliable JSON");
        };
        assert!(
            matches!(serde_json::from_str::<FirstPersonMessage>(chat).unwrap(),
            FirstPersonMessage::Chat { display_name, text, .. }
                if display_name == "Ada" && text == "hello")
        );
        assert!(matches!(
            commands[2],
            RealtimeCommand::PublishPositionedUnreliable { .. }
        ));
        assert!(
            commands
                .windows(2)
                .all(|pair| pair[0].sequence() < pair[1].sequence())
        );
    }

    #[test]
    fn reliable_publish_queue_rejects_overflow_without_eviction() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        let message = FirstPersonMessage::Chat {
            version: MESSAGE_VERSION,
            display_name: "Ada".to_owned(),
            text: "hello".to_owned(),
        };
        for _ in 0..MAX_PENDING_PUBLISHES {
            assert!(app.queue_message(&message));
        }
        let first_sequence = app.pending_publishes.front().unwrap().sequence();
        assert!(!app.queue_message(&message));
        assert_eq!(app.pending_publishes.len(), MAX_PENDING_PUBLISHES);
        assert_eq!(
            app.pending_publishes.front().unwrap().sequence(),
            first_sequence
        );
        assert!(
            app.chat_feedback
                .as_deref()
                .unwrap()
                .contains("queue is full")
        );
    }

    #[test]
    fn chat_focus_suppresses_movement_and_enter_escape_transitions() {
        let mut app = FirstPersonLab::new();
        let starting_eye = app.scene.camera.eye;
        app.update(
            frame(1.0),
            &InputFrame {
                movement: Vec2::Y,
                text_events: vec![TextInputEvent::Open, TextInputEvent::Insert("w".to_owned())],
                ..InputFrame::default()
            },
        );
        assert!(app.chat_open);
        assert_eq!(app.chat_draft, "w");
        assert_eq!(app.scene.camera.eye, starting_eye);
        assert_eq!(app.pointer_mode(), PointerMode::None);

        app.update(
            frame(2.0),
            &InputFrame {
                text_events: vec![TextInputEvent::Cancel],
                ..InputFrame::default()
            },
        );
        assert!(!app.chat_open);
        assert!(app.chat_draft.is_empty());
    }

    #[test]
    fn chat_history_is_bounded_and_entity_leave_removes_profile() {
        let mut app = FirstPersonLab::new();
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &profile_payload("Visitor"));
        app.receive_unreliable_pose(9, 1, &pose_payload());
        for index in 0..=MAX_CHAT_HISTORY {
            app.push_chat(ChatEntry {
                entity_id: 9,
                display_name: "Visitor".to_owned(),
                text: index.to_string(),
            });
        }
        assert_eq!(app.chat_history.len(), MAX_CHAT_HISTORY);
        assert_eq!(app.chat_history.front().unwrap().text, "1");

        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 9 });
        assert!(!app.remote_players.contains_key(&9));
        assert!(!app.remote_profiles.contains_key(&9));
        assert!(!app.last_remote_reliable_sequences.contains_key(&9));
        assert!(!app.last_remote_pose_sequences.contains_key(&9));
    }

    #[test]
    fn remote_chat_and_pose_use_independent_ordered_sequences() {
        let mut app = FirstPersonLab::new();
        enter_peer(&mut app, 9);
        app.receive_message(9, 2, &chat_payload("Visitor", "hello"));
        app.receive_unreliable_pose(9, 1, &pose_payload());
        assert_eq!(app.chat_history.len(), 1);
        assert_eq!(app.remote_players[&9].display_name, "Visitor");
        assert_eq!(app.last_remote_reliable_sequences[&9], 2);
        assert_eq!(app.last_remote_pose_sequences[&9], 1);

        app.receive_unreliable_pose(9, 100, &pose_payload());
        app.receive_message(9, 3, &profile_payload("New Name"));
        app.receive_message(9, 4, &chat_payload("New Name", "still reliable"));
        app.receive_message(9, 4, &chat_payload("Duplicate", "must not appear"));
        app.receive_message(9, 3, &profile_payload("Stale Name"));
        assert_eq!(app.chat_history.len(), 2);
        assert_eq!(app.remote_players[&9].display_name, "New Name");
        assert_eq!(app.last_remote_reliable_sequences[&9], 4);
        assert_eq!(app.last_remote_pose_sequences[&9], 100);
    }

    #[test]
    fn pose_codec_has_a_golden_25_byte_little_endian_layout() {
        let position = Vec3::new(1.0, EYE_HEIGHT, -2.0);
        let golden = [
            1, 0, 0, 128, 63, 154, 153, 217, 63, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128,
            191,
        ];
        assert_eq!(golden.len(), 25);
        assert_eq!(
            encode_pose(position, Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            golden
        );
        assert_eq!(
            decode_pose(&golden),
            Some(PlayerPose {
                position,
                forward: Vec3::new(0.0, 0.0, -1.0),
            })
        );
    }

    #[test]
    fn pose_codec_requires_exact_length_and_known_version() {
        let payload = pose_payload();
        for length in 0..POSE_PAYLOAD_BYTES {
            assert!(decode_pose(&payload[..length]).is_none());
        }
        let mut too_long = payload.clone();
        too_long.push(0);
        assert!(decode_pose(&too_long).is_none());
        for version in [0, POSE_VERSION + 1, u8::MAX] {
            let mut wrong_version = payload.clone();
            wrong_version[0] = version;
            assert!(decode_pose(&wrong_version).is_none());
        }
        assert!(decode_pose(legacy_pose_payload("Visitor").as_bytes()).is_none());
    }

    #[test]
    fn pose_codec_rejects_nonfinite_components_and_finite_overflow() {
        let position = [1.0, EYE_HEIGHT, -2.0];
        let forward = [0.0, 0.0, -1.0];
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
            for component in 0..3 {
                let mut invalid_position = position;
                invalid_position[component] = invalid;
                assert!(decode_pose(&unchecked_pose_payload(invalid_position, forward)).is_none());
                assert!(
                    encode_pose(
                        Vec3::from_array(invalid_position),
                        Vec3::from_array(forward)
                    )
                    .is_none()
                );
                let mut invalid_forward = forward;
                invalid_forward[component] = invalid;
                assert!(decode_pose(&unchecked_pose_payload(position, invalid_forward)).is_none());
                assert!(
                    encode_pose(
                        Vec3::from_array(position),
                        Vec3::from_array(invalid_forward)
                    )
                    .is_none()
                );
            }
        }
        assert!(decode_pose(&unchecked_pose_payload(position, [1.0e20; 3])).is_none());
    }

    #[test]
    fn pose_codec_validates_room_and_nonzero_direction() {
        for position in [
            [ROOM_HALF_WIDTH + 0.01, EYE_HEIGHT, 0.0],
            [-ROOM_HALF_WIDTH - 0.01, EYE_HEIGHT, 0.0],
            [0.0, EYE_HEIGHT, ROOM_HALF_DEPTH + 0.01],
            [0.0, EYE_HEIGHT, -ROOM_HALF_DEPTH - 0.01],
            [0.0, EYE_HEIGHT + 0.26, 0.0],
            [0.0, EYE_HEIGHT - 0.26, 0.0],
        ] {
            assert!(decode_pose(&unchecked_pose_payload(position, [0.0, 0.0, -1.0])).is_none());
            assert!(encode_pose(Vec3::from_array(position), -Vec3::Z).is_none());
        }
        for forward in [[0.0; 3], [0.001, 0.0, 0.0]] {
            assert!(
                decode_pose(&unchecked_pose_payload([0.0, EYE_HEIGHT, 0.0], forward)).is_none()
            );
            assert!(
                encode_pose(Vec3::new(0.0, EYE_HEIGHT, 0.0), Vec3::from_array(forward)).is_none()
            );
        }
        let boundary = Vec3::new(ROOM_HALF_WIDTH, EYE_HEIGHT + 0.25, -ROOM_HALF_DEPTH);
        let payload = encode_pose(boundary, Vec3::new(0.0, 0.0, -2.0)).unwrap();
        assert_eq!(decode_pose(&payload).unwrap().position, boundary);
        assert_eq!(decode_pose(&payload).unwrap().forward, -Vec3::Z);
    }

    #[test]
    fn pose_publishing_is_10_hz_and_replaces_only_pending_pose() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.queue_local_pose(0.0);
        let first_sequence = app.next_sequence;
        app.queue_local_pose(0.099);
        assert_eq!(app.next_sequence, first_sequence);
        app.scene.camera.eye.x = 2.0;
        app.scene.camera.target.x = 2.0;
        app.queue_local_pose(0.1);
        assert_eq!(app.pending_publishes.len(), 2);
        assert_eq!(app.pending_publishes.front().unwrap().sequence(), 1);
        let RealtimeCommand::PublishPositionedUnreliable {
            sequence,
            position,
            payload,
        } = app.pending_publishes.back().unwrap()
        else {
            panic!("only the newest binary pose should remain pending");
        };
        assert_eq!(*sequence, first_sequence + 1);
        assert_eq!(*position, [2.0, f64::from(EYE_HEIGHT), 5.0]);
        assert_eq!(decode_pose(payload).unwrap().position.x, 2.0);

        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        app.queue_local_pose(0.199);
        assert!(app.pending_publishes.is_empty());
        app.queue_local_pose(0.2);
        assert_eq!(app.pending_publishes.len(), 1);
        assert!(matches!(
            app.pending_publishes[0],
            RealtimeCommand::PublishPositionedUnreliable { .. }
        ));
    }

    #[test]
    fn saturated_queue_drops_pose_without_reliable_fallback_or_chat_eviction() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        let message = FirstPersonMessage::Chat {
            version: MESSAGE_VERSION,
            display_name: "Ada".to_owned(),
            text: "hello".to_owned(),
        };
        for _ in 1..MAX_PENDING_PUBLISHES {
            assert!(app.queue_message(&message));
        }
        let before = app.pending_publishes.clone();
        app.queue_local_pose(1.0);
        assert_eq!(app.pending_publishes, before);
        assert_eq!(app.next_sequence, MAX_PENDING_PUBLISHES as u64);
    }

    #[test]
    fn new_pose_cannot_be_serialized_as_json() {
        let legacy = FirstPersonMessage::Pose {
            version: MESSAGE_VERSION,
            display_name: "Visitor".to_owned(),
            position: [1.0, EYE_HEIGHT, -2.0],
            forward: [0.0, 0.0, -1.0],
        };
        assert!(serde_json::to_string(&legacy).is_err());
        let mut app = FirstPersonLab::new();
        assert!(!app.queue_message(&legacy));
        assert!(app.pending_publishes.is_empty());
    }

    #[test]
    fn legacy_json_poses_remain_receive_only_and_strictly_validated() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.receive_message(9, 1, &legacy_pose_payload("Legacy Visitor"));
        assert_eq!(app.remote_players[&9].display_name, "Legacy Visitor");
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        assert!(app.last_remote_pose_sequences.is_empty());
        let mut invalid: serde_json::Value =
            serde_json::from_str(&legacy_pose_payload("Bad Direction")).unwrap();
        invalid["forward"] = serde_json::json!([f32::MAX, f32::MAX, f32::MAX]);
        app.receive_message(9, 99, &invalid.to_string());
        assert_eq!(app.remote_players[&9].display_name, "Legacy Visitor");
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        app.receive_message(9, 2, &profile_payload("Updated Visitor"));
        assert_eq!(app.remote_players[&9].display_name, "Updated Visitor");
    }

    #[test]
    fn profile_arrival_before_or_after_pose_preserves_display_name() {
        let mut app = FirstPersonLab::new();
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &profile_payload("Before Pose"));
        assert!(app.remote_players.is_empty());
        app.receive_unreliable_pose(9, 2, &pose_payload());
        assert_eq!(app.remote_players[&9].display_name, "Before Pose");
        enter_peer(&mut app, 10);
        app.receive_unreliable_pose(10, 100, &pose_payload());
        assert_eq!(app.remote_players[&10].display_name, DEFAULT_DISPLAY_NAME);
        app.receive_message(10, 1, &profile_payload("After Pose"));
        app.receive_unreliable_pose(10, 101, &pose_payload());
        assert_eq!(app.remote_players[&10].display_name, "After Pose");
    }

    #[test]
    fn entity_entered_announces_existing_profile_to_late_joiner() {
        let mut existing = FirstPersonLab::with_display_name("Ada");
        existing.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        existing.queue_local_pose(0.0);
        let mut old_commands = Vec::new();
        existing.drain_realtime_commands(&mut old_commands);
        let RealtimeCommand::PublishPositionedUnreliable {
            sequence: pose_sequence,
            payload: pose,
            ..
        } = &old_commands[1]
        else {
            panic!("expected an existing binary pose");
        };

        let mut late_joiner = FirstPersonLab::with_display_name("Grace");
        late_joiner.handle_realtime_event(RealtimeEvent::Connected { entity_id: 9 });
        // The newcomer receives its own entry, not an initial roster of existing peers.
        enter_peer(&mut late_joiner, 9);
        late_joiner.handle_realtime_event(RealtimeEvent::UnreliablePayload {
            entity_id: 7,
            sequence: *pose_sequence,
            payload: pose.clone(),
        });
        assert!(late_joiner.remote_profiles.is_empty());
        assert!(late_joiner.remote_players.is_empty());
        assert!(late_joiner.last_remote_pose_sequences.is_empty());
        enter_peer(&mut existing, 9);
        let mut announcements = Vec::new();
        existing.drain_realtime_commands(&mut announcements);
        assert_eq!(announcements.len(), 1);
        let RealtimeCommand::PublishReliable { sequence, payload } = &announcements[0] else {
            panic!("late joiner needs a reliable JSON profile announcement");
        };
        assert!(
            matches!(serde_json::from_str::<FirstPersonMessage>(payload).unwrap(),
            FirstPersonMessage::Profile { display_name, .. } if display_name == "Ada")
        );
        late_joiner.handle_realtime_event(RealtimeEvent::Payload {
            entity_id: 7,
            sequence: *sequence,
            payload: payload.clone(),
        });
        assert_eq!(late_joiner.remote_profiles[&7], "Ada");
        assert!(late_joiner.remote_players.is_empty());
        late_joiner.handle_realtime_event(RealtimeEvent::UnreliablePayload {
            entity_id: 7,
            sequence: *pose_sequence,
            payload: pose.clone(),
        });
        assert_eq!(late_joiner.remote_players[&7].display_name, "Ada");
        assert!(late_joiner.chat_history.is_empty());
    }

    #[test]
    fn name_change_announces_profile_without_repeating_name_in_pose() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.queue_local_pose(0.0);
        let original_pose = match app.pending_publishes.back().unwrap() {
            RealtimeCommand::PublishPositionedUnreliable { payload, .. } => payload.clone(),
            _ => panic!("expected a binary pose"),
        };
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        app.chat_open = true;
        app.chat_draft = "/name Grace".to_owned();
        app.submit_chat();
        assert_eq!(app.display_name, "Grace");
        assert!(!app.chat_open);
        assert!(app.chat_history.is_empty());
        app.queue_local_pose(0.05);
        assert_eq!(app.pending_publishes.len(), 1);
        app.queue_local_pose(0.1);
        commands.clear();
        app.drain_realtime_commands(&mut commands);
        assert_eq!(commands.len(), 2);
        let RealtimeCommand::PublishReliable { payload, .. } = &commands[0] else {
            panic!("name change must announce a reliable profile");
        };
        assert!(
            matches!(serde_json::from_str::<FirstPersonMessage>(payload).unwrap(),
            FirstPersonMessage::Profile { display_name, .. } if display_name == "Grace")
        );
        let RealtimeCommand::PublishPositionedUnreliable { payload, .. } = &commands[1] else {
            panic!("pose must remain binary unreliable");
        };
        assert_eq!(payload, &original_pose);
        assert_eq!(payload.len(), 25);
    }

    #[test]
    fn pending_profile_retries_after_reliable_queue_drains() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        let message = FirstPersonMessage::Chat {
            version: MESSAGE_VERSION,
            display_name: "Ada".to_owned(),
            text: "hello".to_owned(),
        };
        for _ in 1..MAX_PENDING_PUBLISHES {
            assert!(app.queue_message(&message));
        }
        let before = app.pending_publishes.clone();
        app.apply_display_name("Grace".to_owned());
        assert!(app.profile_pending);
        assert_eq!(app.pending_publishes, before);
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        assert_eq!(commands.len(), MAX_PENDING_PUBLISHES);
        commands.clear();
        app.drain_realtime_commands(&mut commands);
        assert!(!app.profile_pending);
        assert_eq!(commands.len(), 1);
        let RealtimeCommand::PublishReliable { payload, .. } = &commands[0] else {
            panic!("pending profile must retry reliably");
        };
        assert!(
            matches!(serde_json::from_str::<FirstPersonMessage>(payload).unwrap(),
            FirstPersonMessage::Profile { display_name, .. } if display_name == "Grace")
        );
    }

    #[test]
    fn entity_left_rejects_late_pose_profile_and_chat_until_reentered() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.receive_message(9, 1, &profile_payload("Visitor"));
        app.receive_unreliable_pose(9, 2, &pose_payload());
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 9 });
        app.handle_realtime_event(RealtimeEvent::UnreliablePayload {
            entity_id: 9,
            sequence: 3,
            payload: pose_payload(),
        });
        app.receive_message(9, 4, &profile_payload("Ghost"));
        app.receive_message(9, 5, &chat_payload("Ghost", "late chat"));
        app.receive_message(9, 6, &legacy_pose_payload("Ghost"));
        assert!(app.remote_players.is_empty());
        assert!(app.remote_profiles.is_empty());
        assert!(app.last_remote_pose_sequences.is_empty());
        assert!(app.last_remote_reliable_sequences.is_empty());
        assert!(app.chat_history.is_empty());
        app.update_remote_players(1.0 / 60.0);
        assert_eq!(app.scene.meshes.len(), app.base_mesh_count);
        assert!(app.departed_entities.contains(&9));

        enter_peer(&mut app, 9);
        assert!(!app.departed_entities.contains(&9));
        app.receive_message(9, 1, &profile_payload("Rejoined"));
        app.receive_unreliable_pose(9, 1, &pose_payload());
        assert_eq!(app.remote_players[&9].display_name, "Rejoined");
    }

    #[test]
    fn unauthorized_pose_chat_and_self_payloads_cannot_create_players_or_watermarks() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 7);
        for entity_id in [7, 9] {
            app.receive_unreliable_pose(entity_id, 1, &pose_payload());
            app.receive_message(entity_id, 3, &chat_payload("Unauthorized", "hello"));
        }
        app.receive_message(7, 2, &profile_payload("Self"));
        assert!(app.remote_players.is_empty());
        assert!(app.remote_profiles.is_empty());
        assert!(app.last_remote_pose_sequences.is_empty());
        assert!(app.last_remote_reliable_sequences.is_empty());
        assert!(app.chat_history.is_empty());
    }

    #[test]
    fn peer_profiles_and_both_lane_watermarks_remain_bounded() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 10_000 });
        for entity_id in 1..=(MAX_REMOTE_PLAYERS + 10) as u64 {
            app.receive_message(entity_id, 1, &profile_payload("Visitor"));
            app.receive_unreliable_pose(entity_id, 2, &pose_payload());
        }
        assert_eq!(app.remote_profiles.len(), MAX_REMOTE_PLAYERS);
        assert_eq!(app.remote_players.len(), MAX_REMOTE_PLAYERS);
        assert_eq!(app.last_remote_reliable_sequences.len(), MAX_REMOTE_PLAYERS);
        assert_eq!(app.last_remote_pose_sequences.len(), MAX_REMOTE_PLAYERS);
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 1 });
        let replacement = (MAX_REMOTE_PLAYERS + 1) as u64;
        app.receive_message(replacement, 1, &profile_payload("Replacement"));
        app.receive_unreliable_pose(replacement, 1, &pose_payload());
        app.receive_message(1, 100, &profile_payload("Departed"));
        app.receive_unreliable_pose(1, 100, &pose_payload());
        assert_eq!(app.remote_profiles.len(), MAX_REMOTE_PLAYERS);
        assert_eq!(app.remote_players.len(), MAX_REMOTE_PLAYERS);
        assert!(!app.remote_players.contains_key(&1));
        assert!(app.remote_players.contains_key(&replacement));
    }

    #[test]
    fn malformed_profiles_and_poses_do_not_poison_either_watermark() {
        let mut app = FirstPersonLab::new();
        enter_peer(&mut app, 9);
        for payload in [
            r#"{"kind":"profile","version":2,"display_name":"Visitor"}"#,
            r#"{"kind":"profile","version":1,"display_name":""}"#,
            r#"{"kind":"profile","version":1,"display_name":"Visitor","extra":true}"#,
        ] {
            app.receive_message(9, 99, payload);
        }
        let invalid_pose = unchecked_pose_payload([1.0, EYE_HEIGHT, -2.0], [f32::MAX; 3]);
        app.receive_unreliable_pose(9, 99, &invalid_pose);
        assert!(app.last_remote_reliable_sequences.is_empty());
        assert!(app.last_remote_pose_sequences.is_empty());
        app.receive_message(9, 1, &profile_payload("Visitor"));
        app.receive_unreliable_pose(9, 1, &pose_payload());
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        assert_eq!(app.last_remote_pose_sequences[&9], 1);
    }

    #[test]
    fn disconnect_clears_peer_authorization_and_reconnect_keeps_local_sequence_monotonic() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &profile_payload("Visitor"));
        app.receive_unreliable_pose(9, 2, &pose_payload());
        app.queue_local_pose(0.0);
        let previous_sequence = app.next_sequence;
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        app.receive_unreliable_pose(9, 3, &pose_payload());
        assert!(app.remote_players.is_empty());
        assert!(app.remote_profiles.is_empty());
        assert!(app.last_remote_reliable_sequences.is_empty());
        assert!(app.last_remote_pose_sequences.is_empty());
        assert!(app.pending_publishes.is_empty());
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 8 });
        assert_eq!(
            app.pending_publishes.front().unwrap().sequence(),
            previous_sequence + 1
        );
    }

    #[test]
    fn unknown_peer_messages_are_validated_before_consuming_capacity() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        let mut invalid_legacy: serde_json::Value =
            serde_json::from_str(&legacy_pose_payload("Visitor")).unwrap();
        invalid_legacy["forward"] = serde_json::json!([f32::MAX, f32::MAX, f32::MAX]);
        let invalid_legacy = invalid_legacy.to_string();
        let too_large = "x".repeat(MAX_MESSAGE_BYTES + 1);
        for entity_id in 9..9 + (MAX_REMOTE_PLAYERS + 10) as u64 {
            for payload in [
                r#"{"kind":"profile","version":2,"display_name":"Visitor"}"#,
                r#"{"kind":"profile","version":1,"display_name":""}"#,
                r#"{"kind":"profile","version":1,"display_name":"Visitor","extra":true}"#,
                "not json",
                invalid_legacy.as_str(),
                too_large.as_str(),
            ] {
                app.receive_message(entity_id, 99, payload);
            }
            app.receive_message(entity_id, 100, &chat_payload("Visitor", "not a profile"));
            app.receive_unreliable_pose(entity_id, 101, &pose_payload());
        }
        assert!(app.remote_profiles.is_empty());
        assert!(app.remote_players.is_empty());
        assert!(app.last_remote_reliable_sequences.is_empty());
        assert!(app.last_remote_pose_sequences.is_empty());
        assert!(app.departed_entities.is_empty());
        assert!(app.chat_history.is_empty());
        app.receive_message(9, 1, &profile_payload("Validated"));
        app.receive_unreliable_pose(9, 1, &pose_payload());
        assert_eq!(app.remote_players[&9].display_name, "Validated");
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        assert_eq!(app.last_remote_pose_sequences[&9], 1);
    }

    #[test]
    fn bounded_departure_history_fails_closed_without_evicting_old_tombstones() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.receive_message(9, 1, &profile_payload("Still Present"));
        for entity_id in 100..100 + MAX_DEPARTED_ENTITIES as u64 {
            app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id });
        }
        assert_eq!(app.departed_entities.len(), MAX_DEPARTED_ENTITIES);
        assert!(!app.profile_only_admission_closed);
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 100 });
        assert!(!app.profile_only_admission_closed);
        let overflow_entity = 100 + MAX_DEPARTED_ENTITIES as u64;
        app.handle_realtime_event(RealtimeEvent::EntityLeft {
            entity_id: overflow_entity,
        });
        assert!(app.profile_only_admission_closed);
        assert_eq!(app.departed_entities.len(), MAX_DEPARTED_ENTITIES);
        for entity_id in [100, overflow_entity, 10_000] {
            app.receive_message(entity_id, 1, &profile_payload("Must Wait For Entry"));
            app.receive_message(entity_id, 2, &legacy_pose_payload("Must Wait For Entry"));
            app.receive_unreliable_pose(entity_id, 3, &pose_payload());
            assert!(!app.remote_profiles.contains_key(&entity_id));
            assert!(!app.remote_players.contains_key(&entity_id));
        }
        app.receive_message(9, 2, &profile_payload("Still Updating"));
        app.receive_unreliable_pose(9, 3, &pose_payload());
        assert_eq!(app.remote_players[&9].display_name, "Still Updating");
        enter_peer(&mut app, 100);
        assert!(!app.departed_entities.contains(&100));
        app.receive_message(100, 1, &profile_payload("Reentered"));
        app.receive_unreliable_pose(100, 2, &pose_payload());
        assert_eq!(app.remote_players[&100].display_name, "Reentered");
        enter_peer(&mut app, overflow_entity);
        app.receive_unreliable_pose(overflow_entity, 1, &pose_payload());
        assert!(app.remote_players.contains_key(&overflow_entity));
    }

    #[test]
    fn connection_lifecycle_resets_departures_and_profile_admission() {
        let mut app = FirstPersonLab::new();
        for entity_id in 100..=100 + MAX_DEPARTED_ENTITIES as u64 {
            app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id });
        }
        assert!(app.profile_only_admission_closed);
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        assert!(app.departed_entities.is_empty());
        assert!(!app.profile_only_admission_closed);
        app.receive_message(100, 1, &profile_payload("Fresh Connection"));
        assert_eq!(app.remote_profiles[&100], "Fresh Connection");
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 100 });
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        assert!(app.departed_entities.is_empty());
        assert!(!app.profile_only_admission_closed);
        app.receive_message(100, 2, &profile_payload("Late While Offline"));
        assert!(app.remote_profiles.is_empty());
        assert!(app.last_remote_reliable_sequences.is_empty());
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 8 });
        app.receive_message(100, 1, &profile_payload("Fresh Reconnection"));
        assert_eq!(app.remote_profiles[&100], "Fresh Reconnection");
    }

    #[test]
    fn exhausted_sequence_does_not_repeat_pose_or_fall_back_to_reliable() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        app.next_sequence = u64::MAX;
        app.queue_local_pose(1.0);
        assert!(app.pending_publishes.is_empty());
        assert_eq!(app.next_sequence, u64::MAX);
    }
}
