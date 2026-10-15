//! Shared first-person multiplayer room for desktop and browser shells.

mod avatars;
mod control_panels;
mod npcs;
mod seating;
mod theater;
mod view;

#[cfg(target_arch = "wasm32")]
pub use control_panels::{
    activate_seat_interaction, invalidate_control_panels, invalidate_scene_interactions,
    pick_control_panel, pick_scene_interaction, set_control_panel_state,
};
#[cfg(target_arch = "wasm32")]
pub use npcs::{
    clear_npc_dialogue, get_npc_definitions, get_npc_interaction_context, present_npc_response,
    set_npc_interaction_active, set_npc_show_cycle, submit_npc_question,
};

use glam::{Quat, UVec2, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use weaver_app_core::{
    AppAssets, FirstPersonController, FrameContext, InputFrame, PointerMode, RealtimeCommand,
    RealtimeEvent, TextInputEvent, TextInputMode, WeaverApp,
};
use weaver_render::{
    Camera, Projection, RenderTransform, SceneSnapshot, SpriteHandle, SpriteInstance, SpriteSpace,
    TextAnchor, TextRun, TextWeight, UiElement, UiRect,
};

const EYE_HEIGHT: f32 = 1.7;
const PLAYER_BILLBOARD_HEIGHT: f32 = EYE_HEIGHT * 1.3;
const NAMEPLATE_FONT_SIZE: f32 = 15.0;
const CHAT_HOLD_SECONDS: f64 = 4.0;
const CHAT_FADE_SECONDS: f64 = 2.0;
const CHAT_FONT_SIZE: f32 = 25.5;
const CHAT_RANGE_METERS: f32 = 10.0;
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
#[cfg(any(target_arch = "wasm32", test))]
const MAX_HOST_CHAT_PENDING: usize = 8;
#[cfg(any(target_arch = "wasm32", test))]
const HOST_CHAT_INTERVAL_MS: f64 = 250.0;
const MAX_CHAT_HISTORY: usize = 30;

const DEFAULT_DISPLAY_NAME: &str = "Guest";

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static HOST_CHAT_COMMANDS: Rc<RefCell<HostChatCommands>> = Rc::new(RefCell::new(HostChatCommands::default()));
    static PENDING_DISPLAY_NAME: std::cell::RefCell<Option<String>> = const {
        std::cell::RefCell::new(None)
    };
    static PENDING_MINIMAL_UI: std::cell::Cell<Option<bool>> = const {
        std::cell::Cell::new(None)
    };
    static PENDING_ROOM_LIGHTING: std::cell::Cell<Option<(f32, f32)>> = const {
        std::cell::Cell::new(None)
    };
    static PENDING_VIDEO_ASPECT: std::cell::Cell<Option<f32>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = weaverDisplayNameChanged, catch)]
    fn browser_display_name_changed(value: &str) -> Result<(), wasm_bindgen::JsValue>;
    #[wasm_bindgen(js_namespace = globalThis, js_name = weaverChatMessage, catch)]
    fn browser_chat_message(
        entity_id: &str,
        display_name: &str,
        text: &str,
        is_local: bool,
    ) -> Result<(), wasm_bindgen::JsValue>;
    #[wasm_bindgen(js_namespace = globalThis, js_name = weaverChatSendCapacity)]
    fn browser_chat_send_capacity() -> usize;
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

/// Select browser presentation before `start`, or queue a change for the next frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_minimal_ui(value: bool) {
    PENDING_MINIMAL_UI.with(|pending| pending.set(Some(value)));
}

/// Attach one metadata-ready browser video after the scene-ready notification.
/// No GPU resource or per-frame queue is exposed to JavaScript.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn attach_theater_video(video: web_sys::HtmlVideoElement) -> bool {
    let Some(aspect) = video_aspect(video.video_width(), video.video_height()) else {
        return false;
    };
    if !weaver_platform_web::attach_external_video(video) {
        return false;
    }
    PENDING_VIDEO_ASPECT.with(|pending| pending.set(Some(aspect)));
    true
}

/// Coalesce a new decoded frame into the next scene render.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn mark_theater_video_frame() -> bool {
    weaver_platform_web::mark_external_video_frame()
}

/// Release the browser source immediately and restore black on the next render.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn detach_theater_video() {
    weaver_platform_web::detach_external_video();
    PENDING_VIDEO_ASPECT.with(|pending| pending.set(Some(theater::DEFAULT_VIDEO_ASPECT)));
}

/// Queue independent lobby/auditorium levels for the next frame; no playback is started.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_room_lighting(lobby: f32, auditorium: f32) -> bool {
    let Some(levels) = validate_light_levels(lobby, auditorium) else {
        return false;
    };
    PENDING_ROOM_LIGHTING.with(|pending| pending.set(Some(levels)));
    true
}

/// Accept a bounded, validated host command, not an acknowledgement of delivery.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn send_host_chat(text: &str) -> bool {
    let Some(now) = web_sys::window()
        .and_then(|window| window.performance())
        .map(|p| p.now())
    else {
        return false;
    };
    HOST_CHAT_COMMANDS.with(|commands| {
        commands
            .borrow_mut()
            .submit(text, now, browser_chat_send_capacity())
    })
}

/// Pause local input independently of networking, including in the offline room.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_host_chat_focus(active: bool) {
    HOST_CHAT_COMMANDS.with(|commands| commands.borrow_mut().set_focus(active));
}

/// Internal shell lifecycle seam: invalidate submissions before asynchronous realtime events.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn reset_host_chat() {
    HOST_CHAT_COMMANDS.with(|commands| commands.borrow_mut().reset_connection());
}

#[derive(Default)]
struct HostChatCommands {
    pending: VecDeque<String>,
    // Zero until Rust has consumed a connection event, and immediately on shell reset.
    capacity: usize,
    last_submission_at: Option<f64>,
    focused: bool,
    reset_native_input: bool,
    wait_for_neutral_input: bool,
}

impl HostChatCommands {
    #[cfg(any(target_arch = "wasm32", test))]
    fn submit(&mut self, text: &str, now: f64, transport_capacity: usize) -> bool {
        if !now.is_finite()
            || text.len() > MAX_CHAT_BYTES
            || self.pending.len() >= MAX_HOST_CHAT_PENDING
            || self.pending.len() >= self.capacity.min(transport_capacity)
            || self
                .last_submission_at
                .is_some_and(|last| now - last < HOST_CHAT_INTERVAL_MS)
        {
            return false;
        }
        let Some(text) = validate_chat_message(text) else {
            return false;
        };
        self.pending.push_back(text);
        self.last_submission_at = Some(now);
        true
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn set_focus(&mut self, active: bool) {
        self.focused = active;
        self.reset_native_input = true;
        self.wait_for_neutral_input = true;
    }

    fn reset_connection(&mut self) {
        self.pending.clear();
        self.capacity = 0;
        self.last_submission_at = None;
    }
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
    seat: Option<u8>,
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

#[derive(Clone, Debug)]
struct AvatarChat {
    text: String,
    received_at: Option<f64>,
}

impl AvatarChat {
    fn opacity(&self, now: f64) -> f32 {
        let age = self
            .received_at
            .map_or(0.0, |received_at| (now - received_at).max(0.0));
        ((CHAT_HOLD_SECONDS + CHAT_FADE_SECONDS - age) / CHAT_FADE_SECONDS).clamp(0.0, 1.0) as f32
    }
}

/// Enclosed room, local camera, and server-backed remote-player state.
pub struct FirstPersonLab {
    scene: SceneSnapshot,
    assets: AppAssets,
    controller: FirstPersonController,
    view: view::ViewRig,
    seating: seating::Seating,
    seat_wait_for_neutral: bool,
    interaction_hint: u32,
    colliders: Vec<theater::Collider>,
    avatar_textures: [SpriteHandle; avatars::AVATAR_COUNT],
    npcs: npcs::NpcRuntime,
    base_mesh_count: usize,
    control_panels: control_panels::ControlPanels,
    lobby_light_level: f32,
    auditorium_light_level: f32,
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
    minimal_ui: bool,
    host_chat: Rc<RefCell<HostChatCommands>>,
    #[cfg(test)]
    chat_notifications: Vec<(String, String, String, bool)>,
    chat_open: bool,
    chat_draft: String,
    chat_history: VecDeque<ChatEntry>,
    avatar_chat: BTreeMap<u64, AvatarChat>,
    elapsed_seconds: f64,
    chat_feedback: Option<String>,
    viewport_size: UVec2,
    network_status: String,
}

impl FirstPersonLab {
    /// Create the enclosed room with a modest third-person camera offset.
    #[must_use]
    pub fn new() -> Self {
        Self::with_display_name("")
    }

    /// Create the room with a validated temporary display name.
    #[must_use]
    pub fn with_display_name(display_name: &str) -> Self {
        let display_name =
            validate_display_name(display_name).unwrap_or_else(weaver_core::generate_guest_name);
        let theater::Theater {
            mut assets,
            instances: mut meshes,
            colliders,
        } = theater::Theater::new();
        let textures = avatars::texture_assets();
        let avatar_textures = std::array::from_fn(|index| textures[index].handle);
        assets.textures.extend(textures);
        let npcs = npcs::NpcRuntime::new(&colliders);
        let control_panels = control_panels::ControlPanels::new(&mut assets, &mut meshes);
        let base_mesh_count = meshes.len();
        let mut scene = SceneSnapshot {
            background_color: [0.01, 0.01, 0.015, 1.0],
            camera: Camera {
                eye: theater::SPAWN,
                target: theater::SPAWN + Vec3::NEG_Z,
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
        controller.sprint_multiplier = 2.0;
        controller.look_sensitivity = 0.0025;
        controller.bounds = Some(theater::Theater::camera_bounds());
        let mut view = view::ViewRig::new(scene.camera.clone());
        scene.camera = view.camera(0.0, &colliders);

        Self {
            seating: seating::Seating::Standing,
            seat_wait_for_neutral: false,
            interaction_hint: 0,
            view,
            scene,
            assets,
            controller,
            colliders,
            avatar_textures,
            npcs,
            base_mesh_count,
            control_panels,
            lobby_light_level: theater::LOBBY_LIGHT_LEVEL,
            auditorium_light_level: theater::AUDITORIUM_LIGHT_LEVEL,
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
            minimal_ui: false,
            #[cfg(target_arch = "wasm32")]
            host_chat: HOST_CHAT_COMMANDS.with(Rc::clone),
            #[cfg(not(target_arch = "wasm32"))]
            host_chat: Rc::new(RefCell::new(HostChatCommands::default())),
            #[cfg(test)]
            chat_notifications: Vec::new(),
            chat_open: false,
            chat_draft: String::with_capacity(MAX_CHAT_BYTES),
            chat_history: VecDeque::with_capacity(MAX_CHAT_HISTORY),
            avatar_chat: BTreeMap::new(),
            elapsed_seconds: 0.0,
            chat_feedback: None,
            viewport_size: UVec2::ZERO,
            network_status: "offline · 0 remote".to_owned(),
        }
    }

    /// Set independent room lighting. Finite levels are clamped to 0..=1.
    /// This local presentation seam does not start playback or publish network state.
    pub fn set_room_lighting(&mut self, lobby: f32, auditorium: f32) -> bool {
        let Some((lobby, auditorium)) = validate_light_levels(lobby, auditorium) else {
            return false;
        };
        self.lobby_light_level = lobby;
        self.auditorium_light_level = auditorium;
        theater::apply_room_lighting(
            &mut self.scene.meshes[..self.base_mesh_count],
            lobby,
            auditorium,
            self.assets.textures[4].handle,
        );
        self.control_panels.refresh_meshes(&mut self.scene.meshes);
        true
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn apply_video_aspect(&mut self, aspect: f32) -> bool {
        let changed = theater::fit_screen(
            &mut self.scene.meshes,
            self.assets.textures[4].handle,
            aspect,
        );
        if changed {
            self.scene.revision = self.scene.revision.next();
        }
        changed
    }

    /// Suppress engine chrome and text entry, retaining avatar names and transient chat.
    pub fn set_minimal_ui(&mut self, minimal_ui: bool) {
        if self.minimal_ui == minimal_ui {
            return;
        }
        self.minimal_ui = minimal_ui;
        if minimal_ui {
            self.chat_open = false;
            self.chat_draft.clear();
            self.chat_feedback = None;
        }
        self.sync_status_lines();
        self.rebuild_overlay();
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
        let camera = if self.view.third_person() {
            &self.view.player
        } else {
            &self.scene.camera
        };
        let payload = match self.seating.index() {
            Some(index) => encode_seated_pose(camera.eye, camera.target - camera.eye, Some(index))
                .map(|bytes| bytes.to_vec()),
            None => encode_pose(camera.eye, camera.target - camera.eye).map(|bytes| bytes.to_vec()),
        };
        let Some(payload) = payload else {
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
                payload,
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
            if player.seat != pose.seat || pose.seat.is_some() {
                player.displayed_position = pose.position;
            }
            player.seat = pose.seat;
            player.target_position = pose.position;
            player.forward = pose.forward;
        } else if self.remote_players.len() < MAX_REMOTE_PLAYERS {
            self.remote_players.insert(
                entity_id,
                RemotePlayer {
                    seat: pose.seat,
                    displayed_position: pose.position,
                    target_position: pose.position,
                    forward: pose.forward,
                    display_name: display_name.clone(),
                },
            );
        } else {
            return false;
        }
        self.resolve_seat_conflict();
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
        let is_local = Some(entry.entity_id) == self.local_entity_id;
        // One replaceable transient message per participant, independent of view mode.
        self.avatar_chat.insert(
            entry.entity_id,
            AvatarChat {
                text: entry.text.clone(),
                received_at: None,
            },
        );
        #[cfg(target_arch = "wasm32")]
        let _ = browser_chat_message(
            &entry.entity_id.to_string(),
            &entry.display_name,
            &entry.text,
            is_local,
        );
        #[cfg(test)]
        self.chat_notifications.push((
            entry.entity_id.to_string(),
            entry.display_name.clone(),
            entry.text.clone(),
            is_local,
        ));
        #[cfg(not(any(target_arch = "wasm32", test)))]
        let _ = is_local;
        if self.chat_history.len() == MAX_CHAT_HISTORY {
            self.chat_history.pop_front();
        }
        self.chat_history.push_back(entry);
    }

    fn handle_text_input(&mut self, events: &[TextInputEvent]) {
        if self.minimal_ui || self.host_chat.borrow().focused {
            return;
        }
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
        if !self.queue_validated_chat(text) {
            return;
        }
        self.chat_draft.clear();
        self.chat_feedback = None;
        self.chat_open = false;
    }

    // Both native and host entry points validate before reaching this shared reliable path.
    fn queue_validated_chat(&mut self, text: String) -> bool {
        let Some(entity_id) = self.local_entity_id else {
            return false;
        };
        let message = FirstPersonMessage::Chat {
            version: MESSAGE_VERSION,
            display_name: self.display_name.clone(),
            text: text.clone(),
        };
        if !self.queue_message(&message) {
            return false;
        }
        self.push_chat(ChatEntry {
            entity_id,
            display_name: self.display_name.clone(),
            text,
        });
        true
    }

    fn sync_host_chat_capacity(&mut self) {
        let mut commands = self.host_chat.borrow_mut();
        commands.capacity = if self.local_entity_id.is_some() && self.next_sequence < u64::MAX {
            MAX_PENDING_PUBLISHES
                .saturating_sub(self.pending_publishes.len())
                .min((u64::MAX - self.next_sequence).min(MAX_PENDING_PUBLISHES as u64) as usize)
        } else {
            0
        };
    }

    fn process_host_chat(&mut self, transport_capacity: usize) {
        while self.pending_publishes.len() < transport_capacity.min(MAX_PENDING_PUBLISHES) {
            let Some(text) = self.host_chat.borrow_mut().pending.pop_front() else {
                break;
            };
            if !self.queue_validated_chat(text.clone()) {
                self.host_chat.borrow_mut().pending.push_front(text);
                break;
            }
        }
        self.sync_host_chat_capacity();
    }

    fn update_remote_players(&mut self, delta_seconds: f32) {
        let blend = 1.0 - (-12.0 * delta_seconds.max(0.0)).exp();
        for player in self.remote_players.values_mut() {
            player.displayed_position = player
                .displayed_position
                .lerp(player.target_position, blend);
        }
        self.scene.meshes.truncate(self.base_mesh_count);
        self.scene.sprites.clear();
        self.scene
            .sprites
            .extend(self.remote_players.iter().map(|(entity_id, player)| {
                let mut avatar = posed_avatar_instance(
                    self.avatar_textures[avatars::avatar_index(*entity_id)],
                    player.displayed_position,
                    self.scene.camera.eye,
                    player.seat.is_some(),
                );
                let level = if theater::in_auditorium(player.displayed_position) {
                    self.auditorium_light_level
                } else {
                    self.lobby_light_level
                };
                avatar.tint = [level, level, level, 1.0];
                avatar
            }));
        if self.view.third_person() {
            // Entity zero is a deterministic offline preview, not a network identity.
            let entity_id = self.local_entity_id.unwrap_or(0);
            let mut avatar = avatar_instance(
                self.avatar_textures[avatars::avatar_index(entity_id)],
                self.view.player.eye,
                self.scene.camera.eye,
            );
            let level = if theater::in_auditorium(self.view.player.eye) {
                self.auditorium_light_level
            } else {
                self.lobby_light_level
            };
            avatar.tint = [level, level, level, 1.0];
            self.scene.sprites.push(avatar);
        }
    }

    fn rebuild_avatar_chat(&mut self) {
        let width = self.viewport_size.x as f32;
        let height = self.viewport_size.y as f32;
        if width <= 1.0 || height <= 1.0 {
            return;
        }
        for (entity_id, message) in &self.avatar_chat {
            let is_local = Some(*entity_id) == self.local_entity_id;
            let opacity = message.opacity(self.elapsed_seconds);
            if opacity <= 0.0 {
                continue;
            }
            let (position, anchor) = if is_local && !self.view.third_person() {
                (Vec2::new(width * 0.5, 52.0), TextAnchor::TopCenter)
            } else {
                let eye = if is_local {
                    self.view.player.eye
                } else if let Some(player) = self.remote_players.get(entity_id) {
                    // Presentation-only proximity uses the physical player, never the zoomed camera.
                    if !remote_chat_in_range(self.view.player.eye, player.displayed_position) {
                        continue;
                    }
                    player.displayed_position
                } else {
                    continue;
                };
                let Some(position) = avatar_label_position(
                    &self.scene.camera,
                    self.viewport_size,
                    &self.colliders,
                    eye,
                ) else {
                    continue;
                };
                // A screen-space gap keeps speech above the name at every distance.
                (
                    position - Vec2::Y * (NAMEPLATE_FONT_SIZE + 6.0),
                    TextAnchor::BottomCenter,
                )
            };
            let text = wrap_avatar_chat(&message.text);
            self.scene.text.push(screen_text(
                text.clone(),
                position + Vec2::new(1.0, 1.0),
                CHAT_FONT_SIZE,
                [0.0, 0.0, 0.0, opacity],
                anchor,
            ));
            self.scene.text.push(screen_text(
                text,
                position,
                CHAT_FONT_SIZE,
                [1.0, 1.0, 1.0, opacity],
                anchor,
            ));
        }
    }

    fn rebuild_nameplates(&mut self) {
        let mut append = |display_name: &str, eye: Vec3| {
            if let Some(position) =
                avatar_label_position(&self.scene.camera, self.viewport_size, &self.colliders, eye)
            {
                let mut nameplate = screen_text(
                    display_name.to_owned(),
                    position,
                    NAMEPLATE_FONT_SIZE,
                    [1.0; 4],
                    TextAnchor::BottomCenter,
                );
                nameplate.weight = TextWeight::Bold;
                self.scene.text.push(nameplate);
            }
        };
        for player in self.remote_players.values() {
            append(&player.display_name, player.displayed_position);
        }
        if self.view.third_person() {
            append(&self.display_name, self.view.player.eye);
        }
        for (display_name, eye) in self.npcs.nameplates() {
            append(display_name, eye);
        }
    }

    fn rebuild_overlay(&mut self) {
        self.scene.text.clear();
        self.scene.ui.clear();
        self.rebuild_avatar_chat();
        self.rebuild_nameplates();
        let hint = match self.interaction_hint {
            target if (100..100 + theater::SEAT_COUNT as u32).contains(&target) => {
                Some("Click / tap to sit")
            }

            _ => None,
        };
        if let Some(hint) = hint.filter(|_| self.viewport_size.x > 1 && self.viewport_size.y > 1) {
            self.scene.text.push(screen_text(
                hint.to_owned(),
                Vec2::new(
                    self.viewport_size.x as f32 * 0.5,
                    self.viewport_size.y as f32 * 0.5 + 32.0,
                ),
                14.0,
                [0.9, 0.93, 1.0, 1.0],
                TextAnchor::TopCenter,
            ));
        }
        if self.minimal_ui {
            return;
        }
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

        if !self.chat_open && self.chat_feedback.is_none() {
            return;
        }
        let line_count = usize::from(self.chat_open) + usize::from(self.chat_feedback.is_some());
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

        let mut line = 0_usize;
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
                weight: TextWeight::Regular,
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
        self.sync_status_lines();
    }

    fn sync_status_lines(&mut self) {
        self.scene.status_lines.clear();
        if !self.minimal_ui {
            self.scene.status_lines.push(self.network_status.clone());
        }
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

    fn external_image_texture(&self) -> Option<weaver_render::SpriteHandle> {
        Some(self.assets.textures[4].handle)
    }

    fn pointer_mode(&self) -> PointerMode {
        if self.chat_open || self.host_chat.borrow().focused {
            PointerMode::None
        } else {
            PointerMode::LockedLook
        }
    }

    fn primary_activation(&mut self, ndc: Vec2) -> bool {
        let host = self.host_chat.borrow();
        if self.pointer_mode() != PointerMode::LockedLook
            || self.chat_open
            || host.focused
            || host.reset_native_input
            || host.wait_for_neutral_input
            || self.seat_wait_for_neutral
        {
            return false;
        }
        drop(host);
        #[cfg(not(target_arch = "wasm32"))]
        let now = self.elapsed_seconds * 1000.0;
        #[cfg(target_arch = "wasm32")]
        let now = web_sys::window()
            .and_then(|window| window.performance())
            .map_or(f64::NAN, |performance| performance.now());
        match control_panels::pick_current_interaction(ndc, now) {
            target
                if target == 200 || (100..100 + theater::SEAT_COUNT as u32).contains(&target) =>
            {
                control_panels::queue_seat_request(target, now)
            }
            1..=3 => true,
            _ => false,
        }
    }

    fn text_input_mode(&self) -> TextInputMode {
        if self.minimal_ui || self.host_chat.borrow().focused {
            TextInputMode::Disabled
        } else if self.chat_open {
            TextInputMode::Focused
        } else {
            TextInputMode::Available
        }
    }

    fn handle_realtime_event(&mut self, event: RealtimeEvent) {
        match event {
            RealtimeEvent::Connected { entity_id } => {
                self.host_chat.borrow_mut().reset_connection();
                self.local_entity_id = Some(entity_id);
                self.remote_players.clear();
                self.avatar_chat.clear();
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
                self.avatar_chat.remove(&entity_id);
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
                self.host_chat.borrow_mut().reset_connection();
                self.local_entity_id = None;
                self.remote_players.clear();
                self.avatar_chat.clear();
                self.remote_profiles.clear();
                self.departed_entities.clear();
                self.profile_only_admission_closed = false;
                self.last_remote_reliable_sequences.clear();
                self.last_remote_pose_sequences.clear();
                self.pending_publishes.clear();
                self.profile_pending = false;
                self.network_status = format!("offline · {reason}");
                self.sync_status_lines();
            }
        }
        self.sync_host_chat_capacity();
    }

    fn drain_realtime_commands(&mut self, commands: &mut Vec<RealtimeCommand>) {
        self.queue_pending_profile();
        commands.extend(self.pending_publishes.drain(..));
        self.sync_host_chat_capacity();
    }

    fn update(&mut self, frame: FrameContext, input: &InputFrame) {
        #[cfg(target_arch = "wasm32")]
        if let Some(aspect) = PENDING_VIDEO_ASPECT.with(std::cell::Cell::take) {
            self.apply_video_aspect(aspect);
        }
        let interaction_context_fresh = frame.elapsed_seconds.is_finite()
            && frame.elapsed_seconds >= self.elapsed_seconds
            && frame.elapsed_seconds - self.elapsed_seconds <= 0.25;
        self.elapsed_seconds = self.elapsed_seconds.max(frame.elapsed_seconds);
        for message in self.avatar_chat.values_mut() {
            // Events arrive before update; start fresh messages on this frame's
            // clock, including the first frame after a hidden-tab pause.
            message.received_at.get_or_insert(self.elapsed_seconds);
        }
        self.avatar_chat
            .retain(|_, message| message.opacity(self.elapsed_seconds) > 0.0);
        #[cfg(target_arch = "wasm32")]
        if let Some((lobby, auditorium)) = PENDING_ROOM_LIGHTING.with(std::cell::Cell::take) {
            self.set_room_lighting(lobby, auditorium);
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(minimal_ui) = PENDING_MINIMAL_UI.with(std::cell::Cell::take) {
            self.set_minimal_ui(minimal_ui);
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(display_name) = PENDING_DISPLAY_NAME.with(std::cell::RefCell::take) {
            self.apply_display_name(display_name);
        }
        let (host_focused, wait_for_neutral, reset_native_input) = {
            let mut commands = self.host_chat.borrow_mut();
            let reset_native_input = commands.reset_native_input;
            if reset_native_input {
                self.chat_open = false;
                self.chat_draft.clear();
                self.chat_feedback = None;
                commands.reset_native_input = false;
            }
            if !commands.focused
                && input.movement == Vec2::ZERO
                && input.look_delta == Vec2::ZERO
                && !input.sprint
                && !input.toggle_perspective
                && input.zoom_delta == 0.0
            {
                commands.wait_for_neutral_input = false;
            }
            (
                commands.focused,
                commands.wait_for_neutral_input,
                reset_native_input,
            )
        };
        let was_chat_open = self.chat_open;
        if !reset_native_input && !wait_for_neutral {
            self.handle_text_input(&input.text_events);
        }
        if !self.view.third_person() {
            self.view.player = self.scene.camera.clone();
        }
        let input_allowed = !was_chat_open
            && !self.chat_open
            && !host_focused
            && !wait_for_neutral
            && !reset_native_input;
        if input.movement == Vec2::ZERO && !input.sprint {
            self.seat_wait_for_neutral = false;
        }
        let npc_utc = npcs::unix_seconds(frame.elapsed_seconds);
        self.npcs
            .set_player_occupied_seats(self.player_occupied_seats());
        self.npcs
            .update(npc_utc, self.view.player.eye, 0.0, &self.colliders);
        // Always drain intent: focus changes must discard, never defer, an old click.
        let request = control_panels::take_seat_request();
        let transitioned = input_allowed
            && interaction_context_fresh
            && request.is_some_and(|target| self.apply_seat_request(target));
        if input_allowed && !transitioned {
            let previous_eye = self.view.player.eye;
            let mut filtered = input.clone();
            if self.seating.index().is_some() || self.seat_wait_for_neutral {
                filtered.movement = Vec2::ZERO;
                filtered.sprint = false;
            }
            if self.seating.index().is_some() {
                filtered.toggle_perspective = false;
                filtered.zoom_delta = 0.0;
            }
            self.controller
                .update(&mut self.view.player, &filtered, frame.delta_seconds);
            if self.seating.index().is_none() {
                let resolved =
                    theater::resolve_movement(previous_eye, self.view.player.eye, &self.colliders);
                let correction = resolved - self.view.player.eye;
                self.view.player.eye = resolved;
                self.view.player.target += correction;
            }
            self.view.apply_input(&filtered);
        }
        self.scene.camera = self.view.camera(frame.delta_seconds, &self.colliders);
        self.control_panels.refresh_meshes(&mut self.scene.meshes);
        #[cfg(target_arch = "wasm32")]
        let panel_now = web_sys::window()
            .and_then(|window| window.performance())
            .map_or(f64::NAN, |performance| performance.now());
        #[cfg(not(target_arch = "wasm32"))]
        let panel_now = frame.elapsed_seconds * 1000.0;
        self.npcs
            .set_player_occupied_seats(self.player_occupied_seats());
        control_panels::set_seating_context(
            self.view.player.eye,
            self.seating.index(),
            self.occupied_seats(),
        );
        self.control_panels.publish(
            &self.scene.camera,
            frame.viewport_size,
            &self.colliders,
            panel_now,
            !self.chat_open && !host_focused && !wait_for_neutral && !reset_native_input,
        );
        self.interaction_hint = if input_allowed {
            control_panels::pick_current_interaction(Vec2::ZERO, panel_now)
        } else {
            0
        };
        #[cfg(target_arch = "wasm32")]
        self.process_host_chat(browser_chat_send_capacity());
        #[cfg(not(target_arch = "wasm32"))]
        self.process_host_chat(MAX_PENDING_PUBLISHES);
        self.viewport_size = frame.viewport_size;
        self.update_remote_players(frame.delta_seconds);
        self.npcs.append_sprites(
            &mut self.scene,
            &self.avatar_textures,
            self.lobby_light_level,
            self.auditorium_light_level,
        );
        self.queue_pending_profile();
        self.queue_local_pose(frame.elapsed_seconds);
        self.sync_host_chat_capacity();
        self.rebuild_overlay();
    }
}

/// Start First-Person Lab after browser initialization and presentation setup.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
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
    let mut app = FirstPersonLab::new();
    app.set_minimal_ui(
        PENDING_MINIMAL_UI
            .with(std::cell::Cell::take)
            .unwrap_or(false),
    );
    weaver_platform_web::run(Box::new(app), weaver_platform_web::WebConfig::default())
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayerPose {
    seat: Option<u8>,
    position: Vec3,
    forward: Vec3,
}

fn validate_light_levels(lobby: f32, auditorium: f32) -> Option<(f32, f32)> {
    if !lobby.is_finite() || !auditorium.is_finite() {
        return None;
    }
    Some((lobby.clamp(0.0, 1.0), auditorium.clamp(0.0, 1.0)))
}

#[cfg(any(target_arch = "wasm32", test))]
fn video_aspect(width: u32, height: u32) -> Option<f32> {
    if width == 0 || height == 0 {
        return None;
    }
    Some(width as f32 / height as f32)
}

// Client pose bounds follow the physical layout; Woven's server bounds remain authoritative.
fn validate_pose(position: Vec3, forward: Vec3) -> Option<PlayerPose> {
    let bounds = theater::Theater::camera_bounds();
    let radius = theater::PLAYER_RADIUS;
    if !position.is_finite()
        || !forward.is_finite()
        || position.x < bounds.min.x - radius
        || position.x > bounds.max.x + radius
        || position.z < bounds.min.z - radius
        || position.z > bounds.max.z + radius
        || (position.z >= 0.0 && position.x < -theater::HALF_WIDTH)
        || (position.y - (EYE_HEIGHT + theater::floor_height(position))).abs() > 0.25
    {
        return None;
    }
    let length_squared = forward.length_squared();
    // Finite components can still overflow the norm and normalize into a zero direction.
    if !length_squared.is_finite() || length_squared < 0.001 {
        return None;
    }
    Some(PlayerPose {
        seat: None,
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

fn validate_seated_pose(position: Vec3, forward: Vec3, seat: Option<u8>) -> Option<PlayerPose> {
    let Some(index) = seat else {
        return validate_pose(position, forward);
    };
    let anchor = theater::SEATS.get(usize::from(index))?.seated_eye();
    let norm = forward.length_squared();
    if !position.is_finite()
        || position.distance(anchor) > 0.01
        || !forward.is_finite()
        || !norm.is_finite()
        || norm < 0.001
    {
        return None;
    }
    Some(PlayerPose {
        seat,
        position: anchor,
        forward: forward / norm.sqrt(),
    })
}

fn encode_seated_pose(position: Vec3, forward: Vec3, seat: Option<u8>) -> Option<[u8; 26]> {
    let pose = validate_seated_pose(position, forward, seat)?;
    let mut payload = [0; 26];
    payload[0] = 2;
    payload[1] = seat.map_or(0, |index| index + 1);
    for (index, value) in pose
        .position
        .to_array()
        .into_iter()
        .chain(pose.forward.to_array())
        .enumerate()
    {
        let offset = 2 + index * 4;
        payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    Some(payload)
}

fn decode_pose(payload: &[u8]) -> Option<PlayerPose> {
    let (offset, seat) = match (payload.first(), payload.len()) {
        (Some(&POSE_VERSION), POSE_PAYLOAD_BYTES) => (1, None),
        (Some(&2), 26) if usize::from(payload[1]) <= theater::SEAT_COUNT => {
            (2, payload[1].checked_sub(1))
        }
        _ => return None,
    };
    let mut values = [0.0; 6];
    for (value, bytes) in values.iter_mut().zip(payload[offset..].as_chunks::<4>().0) {
        *value = f32::from_le_bytes(*bytes);
    }
    validate_seated_pose(
        Vec3::new(values[0], values[1], values[2]),
        Vec3::new(values[3], values[4], values[5]),
        seat,
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

fn wrap_avatar_chat(text: &str) -> String {
    // Bound lines without losing Unicode scalars or long unbroken words.
    let mut wrapped = String::new();
    let mut line_length = 0;
    for word in text.split_whitespace() {
        let word_length = word.chars().count();
        if line_length > 0 {
            if line_length + 1 + word_length > 40 {
                wrapped.push('\n');
                line_length = 0;
            } else {
                wrapped.push(' ');
                line_length += 1;
            }
        }
        for character in word.chars() {
            if line_length == 40 {
                wrapped.push('\n');
                line_length = 0;
            }
            wrapped.push(character);
            line_length += 1;
        }
    }
    wrapped
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
        weight: TextWeight::Regular,
        color,
        anchor,
        bounds: None,
        world_space: false,
        world_height: 1.0,
    }
}

fn remote_chat_in_range(local_eye: Vec3, remote_eye: Vec3) -> bool {
    let distance_squared = local_eye.distance_squared(remote_eye);
    distance_squared.is_finite() && distance_squared <= CHAT_RANGE_METERS * CHAT_RANGE_METERS
}

fn avatar_label_position(
    camera: &Camera,
    viewport_size: UVec2,
    colliders: &[theater::Collider],
    eye: Vec3,
) -> Option<Vec2> {
    if viewport_size.x <= 1 || viewport_size.y <= 1 {
        return None;
    }
    let head = eye + Vec3::Y * (PLAYER_BILLBOARD_HEIGHT - EYE_HEIGHT + 0.05);
    if !theater::has_line_of_sight(camera.eye, head, colliders) {
        return None;
    }
    project_to_screen(
        camera,
        viewport_size.x as f32 / viewport_size.y as f32,
        viewport_size,
        head,
    )
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

fn posed_avatar_instance(
    sprite: SpriteHandle,
    eye: Vec3,
    viewer: Vec3,
    seated: bool,
) -> SpriteInstance {
    let mut avatar = avatar_instance(sprite, eye, viewer);
    if seated {
        // Share the remote-player seated crop, keeping the head/name anchor fixed.
        let height = PLAYER_BILLBOARD_HEIGHT * 0.58;
        avatar.uv_rect = [0.0, 0.0, 1.0, 0.58];
        avatar.transform.scale = height;
        avatar.transform.translation =
            eye + Vec3::Y * (PLAYER_BILLBOARD_HEIGHT - EYE_HEIGHT - height * 0.5);
    }
    avatar
}

fn avatar_instance(
    sprite: SpriteHandle,
    eye_position: Vec3,
    viewer_position: Vec3,
) -> SpriteInstance {
    let to_viewer = viewer_position - eye_position;
    let yaw = to_viewer.x.atan2(to_viewer.z);
    SpriteInstance {
        sprite,
        transform: RenderTransform {
            translation: eye_position - Vec3::Y * (EYE_HEIGHT - PLAYER_BILLBOARD_HEIGHT * 0.5),
            rotation: Quat::from_rotation_y(yaw),
            scale: PLAYER_BILLBOARD_HEIGHT,
        },
        space: SpriteSpace::World,
        ..SpriteInstance::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seating_keeps_sit_hint_but_stands_without_an_exit_overlay() {
        let mut app = FirstPersonLab::new();
        app.set_minimal_ui(true);
        app.viewport_size = UVec2::new(960, 540);
        app.interaction_hint = 100;
        app.rebuild_overlay();
        assert!(
            app.scene
                .text
                .iter()
                .any(|run| run.text == "Click / tap to sit")
        );

        app.view.player.eye = theater::SEATS[0].approach();
        assert!(app.apply_seat_request(100));
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        app.update(frame(0.01), &InputFrame::default());
        assert_eq!(app.interaction_hint, 200);
        assert!(
            !app.scene
                .text
                .iter()
                .any(|run| run.text.starts_with("Click / tap"))
        );
        assert!(app.primary_activation(Vec2::ZERO));
        app.update(frame(0.02), &InputFrame::default());
        assert_eq!(app.seating.index(), None);
    }

    #[test]
    fn seated_codec_is_strict_and_legacy_standing_still_round_trips() {
        for (index, seat) in theater::SEATS.iter().enumerate() {
            let eye = seat.seated_eye();
            let bytes = encode_seated_pose(eye, Vec3::NEG_Z, Some(index as u8)).unwrap();
            assert_eq!(&bytes[..2], &[2, index as u8 + 1]);
            let pose = decode_pose(&bytes).unwrap();
            assert_eq!(pose.seat, Some(index as u8));
            assert_eq!(pose.position, eye);
            assert!(encode_pose(eye, Vec3::NEG_Z).is_none());
            assert!(
                encode_seated_pose(eye + Vec3::X * 0.02, Vec3::NEG_Z, Some(index as u8)).is_none()
            );
            for direction in [Vec3::ZERO, Vec3::splat(f32::NAN), Vec3::splat(f32::MAX)] {
                assert!(encode_seated_pose(eye, direction, Some(index as u8)).is_none());
            }
            let mut malformed = bytes;
            malformed[1] = (theater::SEAT_COUNT + 1) as u8;
            assert!(decode_pose(&malformed).is_none());
            malformed[1] = 0;
            assert!(decode_pose(&malformed).is_none());
            assert!(decode_pose(&bytes[..25]).is_none());
            malformed = bytes;
            malformed[2..6].copy_from_slice(&f32::NAN.to_le_bytes());
            assert!(decode_pose(&malformed).is_none());
        }
        let standing = encode_seated_pose(theater::SPAWN, Vec3::NEG_Z, None).unwrap();
        assert_eq!(
            decode_pose(&standing),
            decode_pose(&encode_pose(theater::SPAWN, Vec3::NEG_Z).unwrap())
        );
    }

    #[test]
    fn seat_intents_use_current_npc_sample_on_arrival_and_departure() {
        let geometry = theater::Theater::new();
        let mut runtime = npcs::NpcRuntime::new(&geometry.colliders);
        let mut arrival = None;
        let mut departure = None;
        for step in 1..6000 {
            let time = f64::from(step) * 0.05;
            runtime.update(time, theater::SPAWN, 0.0, &geometry.colliders);
            if runtime.occupied_seats()[23] {
                arrival.get_or_insert(time);
            } else if arrival.is_some() {
                departure = Some(time);
                break;
            }
        }
        for (time, should_enter) in [(arrival.unwrap(), false), (departure.unwrap(), true)] {
            let mut app = FirstPersonLab::new();
            let seat = theater::SEATS[23];
            app.view.force_first_person();
            app.view.player.eye = seat.approach();
            app.view.player.target = seat.position + Vec3::Y * 0.5;
            app.scene.camera = app.view.camera(0.0, &app.colliders);
            app.controller = FirstPersonController::from_camera(&app.view.player);
            app.update(frame(time - 0.05), &InputFrame::default());
            if !should_enter {
                assert!(control_panels::queue_seat_request(
                    123,
                    (time - 0.05) * 1000.0
                ));
            }
            app.update(frame(time), &InputFrame::default());
            if should_enter {
                // Departure must refresh the published picker in this frame too.
                assert!(app.primary_activation(Vec2::ZERO));
                app.update(frame(time + 0.01), &InputFrame::default());
            }
            assert_eq!(app.seating.index(), should_enter.then_some(23));
            assert_eq!(app.npcs.occupied_seats()[23], !should_enter);
        }
    }

    #[test]
    fn npc_seat_claims_refuse_entry_yield_to_players_and_retry_after_exit() {
        let mut app = FirstPersonLab::new();
        app.update(frame(0.0), &InputFrame::default());
        for index in [5, 18] {
            app.view.player.eye = theater::SEATS[index].approach();
            assert!(app.occupied_seats()[index]);
            assert!(!app.player_occupied_seats()[index]);
            assert!(!app.apply_seat_request(100 + index as u32));
        }
        // Actual remote occupancy, not NPC claims, drives the yield input.
        enter_peer(&mut app, 42);
        let bytes =
            encode_seated_pose(theater::SEATS[18].seated_eye(), Vec3::NEG_Z, Some(18)).unwrap();
        app.receive_unreliable_pose(42, 1, &bytes);
        app.update(frame(0.01), &InputFrame::default());
        assert!(!app.npcs.occupied_seats()[18]);
        assert!(app.occupied_seats()[18]);
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 42 });
        app.update(frame(0.02), &InputFrame::default());
        assert!(app.npcs.occupied_seats()[18]);

        let mut app = FirstPersonLab::new();
        app.view.player.eye = theater::SEATS[18].approach();
        assert!(app.apply_seat_request(118));
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        app.update(frame(0.0), &InputFrame::default());
        assert_eq!(app.seating.index(), Some(18));
        assert!(!app.npcs.occupied_seats()[18]);
        assert!(app.exit_seat());
        app.update(frame(0.01), &InputFrame::default());
        assert!(app.npcs.occupied_seats()[18]);
        assert!(!app.apply_seat_request(118));
    }

    #[test]
    fn seating_forces_first_person_restores_view_and_prevents_held_exit_lurch() {
        let mut app = FirstPersonLab::new();
        app.view.player.eye = theater::SEATS[0].approach();
        app.view.player.target = app.view.player.eye + Vec3::X;
        app.view.apply_input(&InputFrame {
            zoom_delta: -4.0,
            ..InputFrame::default()
        });
        let saved = app.view.preferences();
        app.controller.move_speed = 3.3;
        app.controller.look_sensitivity = 0.004;
        assert!(app.apply_seat_request(100));
        let expected = (theater::SCREEN_TARGET - app.view.player.eye).normalize();
        app.controller
            .update(&mut app.view.player, &InputFrame::default(), 0.0);
        assert!((app.view.player.target - app.view.player.eye).abs_diff_eq(expected, 0.00001));
        assert_eq!(app.controller.move_speed, 3.3);
        assert_eq!(app.controller.look_sensitivity, 0.004);
        assert!(app.controller.bounds.is_some());
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        assert_eq!(app.scene.camera.eye, theater::SEATS[0].seated_eye());
        app.update(
            frame(1.0),
            &InputFrame {
                movement: Vec2::Y,
                sprint: true,
                toggle_perspective: true,
                zoom_delta: -3.0,
                look_delta: Vec2::new(20.0, 0.0),
                ..InputFrame::default()
            },
        );
        assert_eq!(app.view.player.eye, theater::SEATS[0].seated_eye());
        assert!(!app.view.third_person());
        assert!(!(app.view.player.target - app.view.player.eye).abs_diff_eq(expected, 0.001));
        let seated_direction = (app.view.player.target - app.view.player.eye).normalize();
        assert!(app.exit_seat());
        assert_eq!(app.view.player.eye, theater::SEATS[0].approach());
        assert_eq!(app.view.preferences(), saved);
        assert!(
            (app.view.player.target - app.view.player.eye).abs_diff_eq(seated_direction, 0.00001)
        );
        let exit = app.view.player.eye;
        app.update(
            frame(2.0),
            &InputFrame {
                movement: Vec2::Y,
                sprint: true,
                ..InputFrame::default()
            },
        );
        assert_eq!(app.view.player.eye, exit);
        app.update(frame(3.0), &InputFrame::default());
        assert!(!app.seat_wait_for_neutral);
    }

    #[test]
    fn native_activation_all_seats_preserves_height_view_and_pose_roundtrip() {
        for (index, seat) in theater::SEATS.iter().enumerate() {
            for first_person in [true, false] {
                let mut app = FirstPersonLab::new();
                app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 20 });
                app.view.player.eye = seat.approach();
                app.view.player.target = seat.position + Vec3::Y * 0.5;
                if first_person {
                    app.view.force_first_person();
                }
                app.controller = FirstPersonController::from_camera(&app.view.player);
                // Resident chairs use a player-first entry before the first NPC
                // occupancy sample. Production refusal remains enabled throughout.
                let preferences = app.view.preferences();
                let player_first = matches!(index, 5 | 18);
                if player_first {
                    assert!(app.apply_seat_request(100 + index as u32));
                }
                app.scene.camera = app.view.camera(0.0, &app.colliders);
                app.update(frame(0.99), &InputFrame::default());
                app.update(frame(1.0), &InputFrame::default());
                if !player_first {
                    assert!(
                        app.primary_activation(Vec2::ZERO),
                        "seat {index}, first person {first_person}"
                    );
                    assert_eq!(app.seating.index(), None);
                }
                app.update(frame(1.01), &InputFrame::default());
                assert_eq!(app.seating.index(), Some(index as u8));
                for time in [1.02, 1.13, 1.24] {
                    app.update(
                        frame(time),
                        &InputFrame {
                            movement: Vec2::Y,
                            sprint: true,
                            toggle_perspective: true,
                            zoom_delta: -1.0,
                            look_delta: Vec2::new(4.0, 2.0),
                            ..InputFrame::default()
                        },
                    );
                    assert_eq!(app.scene.camera.eye, seat.seated_eye());
                    assert_eq!(app.view.player.eye, seat.seated_eye());
                }
                let (position, payload) = app
                    .pending_publishes
                    .iter()
                    .find_map(|command| {
                        if let RealtimeCommand::PublishPositionedUnreliable {
                            position,
                            payload,
                            ..
                        } = command
                        {
                            Some((*position, payload.clone()))
                        } else {
                            None
                        }
                    })
                    .unwrap();
                assert_eq!(position, seat.seated_eye().as_dvec3().to_array());
                assert_eq!(decode_pose(&payload).unwrap().seat, Some(index as u8));
                let direction = (app.view.player.target - app.view.player.eye).normalize();
                assert!(app.primary_activation(Vec2::ZERO));
                app.update(
                    FrameContext {
                        delta_seconds: 0.0,
                        ..frame(1.25)
                    },
                    &InputFrame::default(),
                );
                assert_eq!(app.seating.index(), None);
                assert_eq!(app.view.player.eye, seat.approach());
                assert_eq!(app.view.preferences(), preferences);
                assert!(
                    (app.view.player.target - app.view.player.eye).abs_diff_eq(direction, 0.00001)
                );
                app.update(frame(1.26), &InputFrame::default());
                assert!(
                    (app.view.player.target - app.view.player.eye).abs_diff_eq(direction, 0.00001)
                );

                let mut receiver = FirstPersonLab::new();
                enter_peer(&mut receiver, 20);
                receiver.receive_unreliable_pose(20, 1, &payload);
                assert_eq!(receiver.remote_players[&20].seat, Some(index as u8));
                assert_eq!(
                    receiver.remote_players[&20].displayed_position,
                    seat.seated_eye()
                );
                receiver.update_remote_players(0.1);
                let avatar = &receiver.scene.sprites[0];
                let height = avatar.transform.scale;
                let top = avatar.transform.translation.y + height * 0.5;
                let bottom = avatar.transform.translation.y - height * 0.5;
                assert!(bottom >= seat.position.y);
                assert!((bottom - (seat.position.y + 0.48)).abs() < 0.01);
                assert!(
                    (top - (seat.seated_eye().y + PLAYER_BILLBOARD_HEIGHT - EYE_HEIGHT)).abs()
                        < 0.00001
                );
                // The renderer uses sampled-region aspect, so height / UV height
                // preserves the standing art's pixel scale and full width.
                assert!((height / avatar.uv_rect[3] - PLAYER_BILLBOARD_HEIGHT).abs() < 0.00001);
                assert_eq!(avatar.uv_rect[2], 1.0);
            }
        }
    }

    #[test]
    fn native_panel_activation_is_consumed_without_queueing_a_seat_action() {
        let mut app = first_person_app();
        app.scene.camera.eye = Vec3::new(-1.5, EYE_HEIGHT, 3.0);
        app.scene.camera.target = app.scene.camera.eye + Vec3::NEG_Z;
        app.controller = FirstPersonController::from_camera(&app.scene.camera);
        app.update(frame(1.0), &InputFrame::default());
        assert_eq!(
            control_panels::pick_current_interaction(Vec2::ZERO, 1000.0),
            1
        );
        assert!(app.primary_activation(Vec2::ZERO));
        assert!(control_panels::take_seat_request().is_none());
        assert_eq!(app.seating.index(), None);
    }

    #[test]
    fn seat_entry_revalidates_physical_line_of_sight() {
        let mut app = FirstPersonLab::new();
        let seat = theater::SEATS[0];
        // A stale/external camera placed inside the chair is in range but occluded.
        app.view.player.eye = seat.position + Vec3::Y * 0.5;
        assert!(app.view.player.eye.distance(seat.seated_eye()) < 2.5);
        assert!(!theater::has_line_of_sight(
            app.view.player.eye,
            seat.seated_eye(),
            &app.colliders
        ));
        assert!(!app.apply_seat_request(100));
        app.view.player.eye = seat.approach();
        assert!(app.apply_seat_request(100));
    }

    #[test]
    fn native_activation_rejects_focus_reset_and_held_input_gates() {
        let mut app = FirstPersonLab::new();
        app.view.player.eye = theater::SEATS[0].approach();
        app.view.player.target = theater::SEATS[0].position + Vec3::Y * 0.5;
        app.view.force_first_person();
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        app.controller = FirstPersonController::from_camera(&app.view.player);
        app.update(frame(1.0), &InputFrame::default());
        app.chat_open = true;
        assert!(!app.primary_activation(Vec2::ZERO));
        app.chat_open = false;
        app.host_chat.borrow_mut().focused = true;
        assert!(!app.primary_activation(Vec2::ZERO));
        app.host_chat.borrow_mut().focused = false;
        app.host_chat.borrow_mut().reset_native_input = true;
        assert!(!app.primary_activation(Vec2::ZERO));
        app.host_chat.borrow_mut().reset_native_input = false;
        app.host_chat.borrow_mut().wait_for_neutral_input = true;
        assert!(!app.primary_activation(Vec2::ZERO));
        app.host_chat.borrow_mut().wait_for_neutral_input = false;
        app.seat_wait_for_neutral = true;
        assert!(!app.primary_activation(Vec2::ZERO));
        app.seat_wait_for_neutral = false;
        assert!(control_panels::take_seat_request().is_none());
        assert!(!app.primary_activation(Vec2::splat(f32::NAN)));
        assert!(app.primary_activation(Vec2::ZERO));
    }

    #[test]
    fn seat_intents_are_discarded_during_chat_and_revalidated_at_consumption() {
        let mut app = FirstPersonLab::new();
        app.view.player.eye = theater::SEATS[0].approach();
        app.view.player.target = theater::SEATS[0].position + Vec3::Y * 0.5;
        app.view.force_first_person();
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        app.controller = FirstPersonController::from_camera(&app.view.player);
        app.update(frame(1.0), &InputFrame::default());
        assert!(control_panels::queue_seat_request(100, 1000.0));
        app.chat_open = true;
        app.update(frame(1.01), &InputFrame::default());
        assert_eq!(app.seating.index(), None);
        app.chat_open = false;
        app.update(frame(1.02), &InputFrame::default());
        assert_eq!(app.seating.index(), None);
        assert!(control_panels::queue_seat_request(100, 1020.0));
        app.scene.camera.eye = theater::SPAWN;
        app.scene.camera.target = theater::SPAWN + Vec3::NEG_Z;
        app.update(frame(1.03), &InputFrame::default());
        assert_eq!(app.seating.index(), None);
        assert!(!control_panels::queue_seat_request(100, 1030.0));
    }

    #[test]
    fn occupancy_conflicts_choose_lowest_entity_and_disconnect_clears_occupancy() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 20 });
        app.view.player.eye = theater::SEATS[0].approach();
        assert!(app.apply_seat_request(100));
        enter_peer(&mut app, 30);
        let bytes =
            encode_seated_pose(theater::SEATS[0].seated_eye(), Vec3::NEG_Z, Some(0)).unwrap();
        app.receive_unreliable_pose(30, 1, &bytes);
        assert_eq!(app.seating.index(), Some(0));
        enter_peer(&mut app, 10);
        app.receive_unreliable_pose(10, 1, &bytes);
        assert_eq!(app.seating.index(), None);
        assert_eq!(app.view.player.eye, theater::SEATS[0].approach());
        assert!(!app.apply_seat_request(100));
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 10 });
        assert!(app.occupied_seats()[0]);
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        assert_eq!(app.occupied_seats(), [false; theater::SEAT_COUNT]);
        assert!(app.apply_seat_request(100));
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 40 });
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        app.queue_local_pose(5.0);
        assert!(app.pending_publishes.iter().any(|command| matches!(command,
            RealtimeCommand::PublishPositionedUnreliable { payload, .. }
            if decode_pose(payload).unwrap().seat == Some(0))));
    }

    fn grounded_eye(mut position: Vec3) -> Vec3 {
        position.y = EYE_HEIGHT + theater::floor_height(position);
        position
    }

    fn waypoint(name: &str) -> Vec3 {
        theater::npc_waypoint(name).unwrap()
    }

    fn horizontal_distance(from: Vec3, to: Vec3) -> f32 {
        Vec2::new(to.x - from.x, to.z - from.z).length()
    }

    fn first_person_app() -> FirstPersonLab {
        let mut app = FirstPersonLab::new();
        app.view.apply_input(&InputFrame {
            toggle_perspective: true,
            ..InputFrame::default()
        });
        app.scene.camera = app.view.camera(0.0, &app.colliders);
        app
    }

    fn chat_runs(app: &FirstPersonLab) -> Vec<&TextRun> {
        app.scene
            .text
            .iter()
            .filter(|run| run.size == CHAT_FONT_SIZE)
            .collect()
    }

    fn assert_identity_only_overlay(app: &FirstPersonLab) {
        let mut names: Vec<_> = app
            .remote_players
            .values()
            .map(|player| player.display_name.as_str())
            .collect();
        names.extend(app.npcs.nameplates().map(|(name, _)| name));
        if app.view.third_person() {
            names.push(&app.display_name);
        }
        assert!(app.scene.text.iter().all(|run| {
            run.size == NAMEPLATE_FONT_SIZE
                && run.anchor == TextAnchor::BottomCenter
                && names.contains(&run.text.as_str())
        }));
        assert!(app.scene.ui.is_empty());
        assert!(app.scene.status_lines.is_empty());
    }

    #[test]
    fn ambient_npcs_are_local_noninteractive_room_population_with_only_identity_labels() {
        let definitions: serde_json::Value =
            serde_json::from_str(&npcs::get_npc_definitions()).unwrap();
        let definitions = definitions.as_array().unwrap();
        assert_eq!(definitions.len(), 5);
        for definition in definitions {
            assert_eq!(definition["behavior"], "ambient");
            assert_eq!(definition["interaction"]["enabled"], false);
            let id = definition["id"].as_str().unwrap();
            assert!(!npcs::submit_npc_question(id, "forged", "hello"));
            assert!(!npcs::present_npc_response(
                id, "forged", "answer", "explain"
            ));
        }
        for third_person in [false, true] {
            let mut app = FirstPersonLab::new();
            app.set_minimal_ui(true);
            assert_eq!(app.npcs.room_counts(), (2, 2));
            app.update(
                FrameContext {
                    delta_seconds: 0.0,
                    ..frame(0.0)
                },
                &InputFrame {
                    toggle_perspective: !third_person,
                    ..InputFrame::default()
                },
            );
            npcs::set_npc_interaction_active(true);
            for step in 0..=1200 {
                let time = f64::from(step) * 0.5;
                app.update(frame(time), &InputFrame::default());
                let (lobby, auditorium) = app.npcs.room_counts();
                assert!((1..=2).contains(&lobby));
                assert!((2..=3).contains(&auditorium));
                assert_eq!(
                    app.scene.sprites.len(),
                    app.npcs.visible_count() + usize::from(third_person)
                );
                assert!(npcs::get_npc_interaction_context().is_none());
                assert_identity_only_overlay(&app);
                assert!(app.scene.text.iter().any(|run| run.text == "Curious Otter"));
                assert_eq!(
                    app.scene
                        .text
                        .iter()
                        .any(|run| run.text == app.display_name),
                    third_person
                );
                assert!(app.local_entity_id.is_none());
                assert!(app.remote_players.is_empty());
                assert!(app.remote_profiles.is_empty());
                assert!(app.departed_entities.is_empty());
                assert!(app.last_remote_reliable_sequences.is_empty());
                assert!(app.last_remote_pose_sequences.is_empty());
                assert!(app.chat_history.is_empty());
                assert!(app.avatar_chat.is_empty());
                assert!(app.chat_notifications.is_empty());
                let mut commands = Vec::new();
                app.drain_realtime_commands(&mut commands);
                assert!(commands.is_empty());
                assert_eq!(app.next_sequence, 0);
            }
            npcs::clear_npc_dialogue();
            npcs::set_npc_interaction_active(false);
            app.set_minimal_ui(false);
            app.update(frame(600.1), &InputFrame::default());
            assert!(app.scene.text.iter().all(|run| !run.text.contains("TALK")));
        }
    }

    #[test]
    fn sprint_is_twice_walking_and_release_restores_speed() {
        let mut walk = FirstPersonLab::new();
        let mut sprint = FirstPersonLab::new();
        let start = theater::SPAWN;
        let input = InputFrame {
            movement: Vec2::Y,
            ..InputFrame::default()
        };
        walk.update(frame(0.0), &input);
        sprint.update(
            frame(0.0),
            &InputFrame {
                sprint: true,
                ..input.clone()
            },
        );
        assert!(
            (sprint.view.player.eye.distance(start) - 2.0 * walk.view.player.eye.distance(start))
                .abs()
                < 0.0001
        );
        let previous = sprint.view.player.eye;
        sprint.update(frame(0.1), &input);
        assert!(
            (sprint.view.player.eye.distance(previous) - walk.view.player.eye.distance(start))
                .abs()
                < 0.0001
        );
    }

    #[test]
    fn third_person_is_local_presentation_not_a_transported_camera_pose() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
        app.pending_publishes.clear();
        app.update(frame(0.0), &InputFrame::default());
        assert!(app.view.third_person());
        assert_eq!(app.view.player.eye, theater::SPAWN);
        assert!(app.scene.camera.eye.distance(app.view.player.eye) > 0.0);
        assert_eq!(app.scene.sprites.len(), 1 + app.npcs.visible_count());
        assert_eq!(
            app.scene.sprites[0].sprite,
            app.avatar_textures[avatars::avatar_index(42)]
        );
        for command in &app.pending_publishes {
            if let RealtimeCommand::PublishPositionedUnreliable {
                position, payload, ..
            } = command
            {
                let pose = decode_pose(payload).unwrap();
                assert_eq!(*position, theater::SPAWN.as_dvec3().to_array());
                assert_eq!(pose.position, theater::SPAWN);
                assert_eq!(pose.forward, Vec3::NEG_Z);
            }
        }
        let first_pose = app.view.player.clone();
        app.update(
            frame(0.05),
            &InputFrame {
                zoom_delta: -100.0,
                ..InputFrame::default()
            },
        );
        assert_eq!(app.view.player.eye, first_pose.eye);
        assert_eq!(app.pending_publishes.len(), 1); // Original 10 Hz cadence.
        app.update(
            frame(0.1),
            &InputFrame {
                zoom_delta: 100.0,
                ..InputFrame::default()
            },
        );
        assert!(!app.view.third_person());
        assert_eq!(app.scene.camera.eye, first_pose.eye);
        assert_eq!(app.scene.sprites.len(), app.npcs.visible_count());
        let mut offline = FirstPersonLab::new();
        offline.update(frame(0.0), &InputFrame::default());
        assert_eq!(
            offline.scene.sprites[0].sprite,
            offline.avatar_textures[avatars::avatar_index(0)]
        );
    }

    #[test]
    fn third_person_sprint_and_look_keep_the_same_collided_player_pose_as_first_person() {
        let mut first = first_person_app();
        let mut third = FirstPersonLab::new();
        first.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
        third.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
        third.update(frame(0.0), &InputFrame::default());
        for elapsed in [0.1, 0.25, 0.4] {
            let input = InputFrame {
                movement: Vec2::new(1.0, -1.0),
                look_delta: Vec2::new(30.0, 80.0),
                sprint: true,
                ..InputFrame::default()
            };
            first.update(frame(elapsed), &input);
            third.update(frame(elapsed), &input);
            assert!(
                third
                    .view
                    .player
                    .eye
                    .abs_diff_eq(first.view.player.eye, 0.0001)
            );
            assert!(
                (third.view.player.target - third.view.player.eye)
                    .abs_diff_eq(first.view.player.target - first.view.player.eye, 0.0001)
            );
            let transported = third
                .pending_publishes
                .iter()
                .find_map(|command| {
                    if let RealtimeCommand::PublishPositionedUnreliable { payload, .. } = command {
                        decode_pose(payload)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(transported.position, third.view.player.eye);
            assert!(transported.forward.abs_diff_eq(
                (third.view.player.target - third.view.player.eye).normalize(),
                0.0001
            ));
            assert!(theater::has_line_of_sight(
                third.view.player.eye,
                third.scene.camera.eye,
                &third.colliders
            ));
        }
        assert!(third.scene.camera.eye.y >= view::CAMERA_CLEARANCE);
        assert!(third.scene.camera.eye.y <= theater::HEIGHT - view::CAMERA_CLEARANCE);
    }

    #[test]
    fn chat_focus_suppresses_sprint_toggle_zoom_and_waits_for_neutral_host_input() {
        let held = InputFrame {
            movement: Vec2::Y,
            sprint: true,
            toggle_perspective: true,
            zoom_delta: -3.0,
            ..InputFrame::default()
        };
        for host in [false, true] {
            let mut app = first_person_app();
            if host {
                app.host_chat.borrow_mut().set_focus(true);
            } else {
                app.chat_open = true;
            }
            app.update(frame(0.0), &held);
            assert_eq!(app.view.player.eye, theater::SPAWN);
            assert!(!app.view.third_person());
            if host {
                app.host_chat.borrow_mut().set_focus(false);
                app.update(frame(0.1), &held);
                // Sprint alone is also non-neutral, even without movement.
                app.update(
                    frame(0.2),
                    &InputFrame {
                        sprint: true,
                        ..InputFrame::default()
                    },
                );
                assert!(app.host_chat.borrow().wait_for_neutral_input);
            } else {
                app.update(
                    frame(0.1),
                    &InputFrame {
                        text_events: vec![TextInputEvent::Cancel],
                        ..held.clone()
                    },
                );
            }
            assert_eq!(app.view.player.eye, theater::SPAWN);
            assert!(!app.view.third_person());
            app.update(frame(0.3), &InputFrame::default());
            app.update(frame(0.4), &held);
            assert!(app.view.third_person());
            assert_ne!(app.view.player.eye, theater::SPAWN);
        }
    }

    #[test]
    fn own_chat_switches_between_top_center_and_overhead_with_one_expiry_clock() {
        let mut app = first_person_app();
        app.set_minimal_ui(true);
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
        let entry = |text: &str| ChatEntry {
            entity_id: 42,
            display_name: "Local".to_owned(),
            text: text.to_owned(),
        };
        app.push_chat(entry("first"));
        app.update(frame(1.0), &InputFrame::default());
        assert_eq!(chat_runs(&app).len(), 2);
        assert!(
            chat_runs(&app)
                .iter()
                .all(|run| run.weight == TextWeight::Regular)
        );
        assert!(
            !app.scene
                .text
                .iter()
                .any(|run| run.text == app.display_name)
        );
        assert_eq!(app.scene.text[1].anchor, TextAnchor::TopCenter);
        assert_eq!(
            app.scene.text[1].position,
            Vec2::new(app.viewport_size.x as f32 * 0.5, 52.0)
        );
        app.push_chat(entry("replacement"));
        app.update(frame(2.0), &InputFrame::default());
        assert_eq!(app.avatar_chat.len(), 1);
        assert!(
            chat_runs(&app)
                .iter()
                .all(|text| text.text == "replacement")
        );
        // Move before switching without restarting the local echo's expiry.
        app.update(
            frame(2.1),
            &InputFrame {
                movement: Vec2::Y,
                ..InputFrame::default()
            },
        );
        app.update(
            frame(2.2),
            &InputFrame {
                toggle_perspective: true,
                ..InputFrame::default()
            },
        );
        app.update(
            FrameContext {
                delta_seconds: 0.8,
                ..frame(3.0)
            },
            &InputFrame::default(),
        );
        assert_eq!(chat_runs(&app).len(), 2);
        assert!(
            chat_runs(&app)
                .iter()
                .all(|run| run.weight == TextWeight::Regular)
        );
        assert_eq!(app.scene.text[1].anchor, TextAnchor::BottomCenter);
        let name = app
            .scene
            .text
            .iter()
            .find(|run| run.text == app.display_name)
            .unwrap();
        assert_eq!(name.position.x, app.scene.text[1].position.x);
        assert!(
            (name.position.y - app.scene.text[1].position.y - NAMEPLATE_FONT_SIZE - 6.0).abs()
                < 0.0001
        );
        assert_eq!(app.avatar_chat[&42].received_at, Some(2.0));
        app.update(
            frame(7.0),
            &InputFrame {
                toggle_perspective: true,
                ..InputFrame::default()
            },
        );
        assert_eq!(app.scene.text[1].anchor, TextAnchor::TopCenter);
        assert_eq!(app.scene.text[1].color[3], 0.5);
        app.update(frame(8.0), &InputFrame::default());
        assert!(app.avatar_chat.is_empty());
        assert_identity_only_overlay(&app);
        app.push_chat(entry("disconnect"));
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        assert!(app.avatar_chat.is_empty());
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
        app.push_chat(entry("reconnect"));
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 43 });
        assert!(app.avatar_chat.is_empty());
    }

    #[test]
    fn scene_starts_with_the_theater_clipped_third_person_camera_and_avatar_assets() {
        let mut app = FirstPersonLab::new();
        assert_eq!(
            app.scene.meshes.len(),
            47 + theater::CHAIR_MESH_COUNT + theater::PROJECTOR_MESH_COUNT
        );
        assert_eq!(app.colliders.len(), 10 + theater::SEAT_COUNT);
        assert_eq!(app.base_mesh_count, app.scene.meshes.len());
        assert_eq!(app.assets.meshes.len(), app.base_mesh_count);
        assert_eq!(
            app.assets.textures.len(),
            5 + theater::POSTER_COUNT + avatars::AVATAR_COUNT + 3 * 7
        );
        assert!(app.scene.sprites.is_empty());
        assert!(app.view.third_person());
        assert_eq!(app.view.player.eye, theater::SPAWN);
        assert_eq!(app.view.player.target - app.view.player.eye, Vec3::NEG_Z);
        let clipped_eye = theater::SPAWN + Vec3::Z * 1.8;
        assert!(app.scene.camera.eye.abs_diff_eq(clipped_eye, 0.0001));
        assert_eq!(app.scene.camera.target, app.view.player.target);
        app.update(frame(0.0), &InputFrame::default());
        assert!(app.scene.camera.eye.abs_diff_eq(clipped_eye, 0.0001));
        for texture in app.avatar_textures {
            assert!(
                app.assets
                    .textures
                    .iter()
                    .any(|asset| asset.handle == texture)
            );
        }
    }

    #[test]
    fn theater_collision_preserves_controller_look_and_stops_large_frame_motion() {
        let mut app = first_person_app();
        app.scene.camera.eye.x = 0.5;
        app.scene.camera.target.x = 0.5;
        let input = InputFrame {
            movement: Vec2::Y,
            look_delta: Vec2::new(0.0, 100.0),
            ..InputFrame::default()
        };
        let mut expected_camera = app.scene.camera.clone();
        let mut controller = app.controller;
        controller.update(&mut expected_camera, &input, 10.0);
        let expected_direction = expected_camera.target - expected_camera.eye;
        app.update(
            FrameContext {
                delta_seconds: 10.0,
                ..frame(10.0)
            },
            &input,
        );
        assert!(
            app.scene
                .camera
                .eye
                .abs_diff_eq(Vec3::new(0.5, EYE_HEIGHT, 0.3), 0.0001)
        );
        assert!(
            (app.scene.camera.target - app.scene.camera.eye)
                .abs_diff_eq(expected_direction, 0.0001)
        );
        app.update(frame(10.1), &InputFrame::default());
        assert!(
            (app.scene.camera.target - app.scene.camera.eye)
                .abs_diff_eq(expected_direction, 0.0001)
        );
    }

    #[test]
    fn theater_passage_is_walkable_through_the_app_in_both_directions() {
        let mut app = first_person_app();
        let start = waypoint("lobby_aisle");
        let end = waypoint("corridor_turn");
        app.scene.camera.eye = start;
        app.scene.camera.target = start + Vec3::NEG_Z;
        let movement_frame = FrameContext {
            delta_seconds: horizontal_distance(start, end) / app.controller.move_speed,
            ..frame(2.0)
        };
        app.update(
            movement_frame,
            &InputFrame {
                movement: Vec2::Y,
                ..InputFrame::default()
            },
        );
        assert!(app.scene.camera.eye.abs_diff_eq(end, 0.0001));
        app.update(
            movement_frame,
            &InputFrame {
                movement: Vec2::NEG_Y,
                ..InputFrame::default()
            },
        );
        assert!(app.scene.camera.eye.abs_diff_eq(start, 0.0001));
    }

    #[test]
    fn theater_hallway_left_turn_and_standing_paths_work_through_the_app() {
        let mut app = first_person_app();
        let route = [
            waypoint("lobby_aisle"),
            waypoint("passage_lobby"),
            waypoint("corridor_turn"),
            waypoint("auditorium_entry"),
            waypoint("row_a"),
            waypoint("row_b"),
            waypoint("auditorium_left"),
            waypoint("auditorium_center"),
            grounded_eye(Vec3::new(-6.75, 0.0, -10.6)),
            theater::SEATS[0].approach(),
            theater::SEATS[3].approach(),
        ];
        app.scene.camera.eye = route[0];
        for destination in route[1..]
            .iter()
            .chain(route[..route.len() - 1].iter().rev())
        {
            let mut direction = *destination - app.scene.camera.eye;
            direction.y = 0.0;
            app.scene.camera.target = app.scene.camera.eye + direction.normalize();
            app.controller = FirstPersonController::from_camera(&app.scene.camera);
            app.update(
                FrameContext {
                    delta_seconds: horizontal_distance(app.scene.camera.eye, *destination) / 4.0,
                    ..frame(2.0)
                },
                &InputFrame {
                    movement: Vec2::Y,
                    ..InputFrame::default()
                },
            );
            assert!(
                app.scene.camera.eye.abs_diff_eq(*destination, 0.0001),
                "actual {:?}, expected {destination:?}",
                app.scene.camera.eye
            );
        }
        app.scene.camera.eye = Vec3::new(0.0, EYE_HEIGHT, 3.0);
        app.scene.camera.target = app.scene.camera.eye + Vec3::NEG_Z;
        app.controller = FirstPersonController::from_camera(&app.scene.camera);
        app.update(
            FrameContext {
                delta_seconds: 3.0,
                ..frame(3.0)
            },
            &InputFrame {
                movement: Vec2::Y,
                ..InputFrame::default()
            },
        );
        assert!(
            app.scene
                .camera
                .eye
                .abs_diff_eq(Vec3::new(0.0, EYE_HEIGHT, 0.3), 0.0001)
        );
    }

    #[test]
    fn full_app_ascent_descent_and_third_person_preserve_grounded_player_poses() {
        let route = [
            waypoint("lobby_aisle"),
            waypoint("passage_lobby"),
            waypoint("corridor_turn"),
            waypoint("auditorium_entry"),
            waypoint("row_a"),
            waypoint("row_b"),
            grounded_eye(Vec3::new(waypoint("row_b").x, 0.0, theater::MIN_Z + 2.0)),
        ];
        let mut first_person_positions = Vec::new();
        for third_person in [false, true] {
            let mut app = first_person_app();
            app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
            app.pending_publishes.clear();
            app.scene.camera.eye = route[0];
            app.scene.camera.target = route[0] + Vec3::NEG_Z;
            app.controller = FirstPersonController::from_camera(&app.scene.camera);
            app.controller.bounds = Some(theater::Theater::camera_bounds());
            app.update(
                FrameContext {
                    delta_seconds: 0.0,
                    ..frame(0.0)
                },
                &InputFrame {
                    toggle_perspective: third_person,
                    ..InputFrame::default()
                },
            );
            let mut initial_commands = Vec::new();
            app.drain_realtime_commands(&mut initial_commands);
            for command in initial_commands {
                if let RealtimeCommand::PublishPositionedUnreliable { payload, .. } = command {
                    assert_eq!(decode_pose(&payload).unwrap().position, route[0]);
                }
            }
            let mut elapsed = 0.0;
            let mut sample_index = 0;
            let mut saw_levels = [false; 3];
            for (leg, destination) in route[1..]
                .iter()
                .chain(route[..route.len() - 1].iter().rev())
                .enumerate()
            {
                let mut direction = *destination - app.view.player.eye;
                direction.y = 0.0;
                let look = direction.normalize() + Vec3::Y * if leg % 2 == 0 { 0.8 } else { -0.8 };
                app.view.player.target = app.view.player.eye + look.normalize();
                if !third_person {
                    app.scene.camera = app.view.player.clone();
                }
                app.controller = FirstPersonController::from_camera(&app.view.player);
                app.controller.move_speed = 4.0;
                app.controller.bounds = Some(theater::Theater::camera_bounds());
                let mut remaining = horizontal_distance(app.view.player.eye, *destination) / 4.0;
                while remaining > 0.000_001 {
                    let delta_seconds = remaining.min(0.05);
                    remaining -= delta_seconds;
                    elapsed += f64::from(delta_seconds);
                    let before = app.view.player.eye;
                    app.update(
                        FrameContext {
                            delta_seconds,
                            ..frame(elapsed)
                        },
                        &InputFrame {
                            movement: Vec2::Y,
                            ..InputFrame::default()
                        },
                    );
                    let eye = app.view.player.eye;
                    assert!(
                        (horizontal_distance(before, eye) - 4.0 * delta_seconds).abs() < 0.0001
                    );
                    assert_eq!(eye.y, EYE_HEIGHT + theater::floor_height(eye));
                    for (index, level) in [0.0, 1.2, 2.4].into_iter().enumerate() {
                        saw_levels[index] |= (theater::floor_height(eye) - level).abs() < 0.0001;
                    }
                    if third_person {
                        assert!(eye.abs_diff_eq(first_person_positions[sample_index], 0.0001));
                        assert!(
                            app.scene.camera.eye.y
                                >= theater::floor_height(app.scene.camera.eye)
                                    + view::CAMERA_CLEARANCE
                                    - 0.0001
                        );
                        assert!(theater::has_line_of_sight(
                            eye,
                            app.scene.camera.eye,
                            &app.colliders
                        ));
                    } else {
                        first_person_positions.push(eye);
                        assert_eq!(app.scene.camera.eye, eye);
                    }
                    sample_index += 1;
                    let mut commands = Vec::new();
                    app.drain_realtime_commands(&mut commands);
                    for command in commands {
                        if let RealtimeCommand::PublishPositionedUnreliable {
                            position,
                            payload,
                            ..
                        } = command
                        {
                            assert_eq!(position, eye.as_dvec3().to_array());
                            let pose = decode_pose(&payload).unwrap();
                            assert_eq!(pose.position, eye);
                            assert!(
                                pose.forward.abs_diff_eq(
                                    (app.view.player.target - eye).normalize(),
                                    0.0001
                                )
                            );
                        }
                    }
                }
                assert!(app.view.player.eye.abs_diff_eq(*destination, 0.0001));
            }
            assert_eq!(saw_levels, [true; 3]);
            assert_eq!(sample_index, first_person_positions.len());
            assert!(app.view.player.eye.abs_diff_eq(route[0], 0.0001));
        }
    }

    #[test]
    fn remote_chat_range_requires_finite_three_dimensional_distance() {
        assert_eq!(CHAT_RANGE_METERS, 10.0);
        let local = Vec3::new(1.0, 2.0, 3.0);
        for offset in [
            Vec3::ZERO,
            Vec3::X * 10.0,
            Vec3::Y * 10.0,
            Vec3::NEG_Z * 10.0,
            Vec3::new(6.0, 0.0, 8.0),
            Vec3::new(0.0, 6.0, 8.0),
        ] {
            assert!(remote_chat_in_range(local, local + offset), "{offset:?}");
        }
        for offset in [
            Vec3::X * 10.001,
            Vec3::Y * 10.001,
            Vec3::NEG_Z * 10.001,
            Vec3::new(0.0, 6.001, 8.0),
            Vec3::new(0.0, 0.1, 10.0),
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
            Vec3::splat(f32::NEG_INFINITY),
            Vec3::splat(f32::MAX),
        ] {
            let remote = local + offset;
            assert!(!remote_chat_in_range(local, remote), "{offset:?}");
            assert!(!remote_chat_in_range(remote, local), "{offset:?}");
        }
    }

    #[test]
    fn remote_chat_uses_displayed_eye_at_the_boundary_and_keeps_distant_names() {
        for minimal_ui in [false, true] {
            let mut app = first_person_app();
            app.set_minimal_ui(minimal_ui);
            app.scene.camera.eye = Vec3::new(-5.0, EYE_HEIGHT, 5.0);
            app.scene.camera.target = app.scene.camera.eye + Vec3::X;
            app.controller = FirstPersonController::from_camera(&app.scene.camera);
            let peer = Vec3::new(5.0, EYE_HEIGHT, 5.0);
            enter_peer(&mut app, 9);
            app.receive_unreliable_pose(9, 1, &pose_payload_at(peer));
            app.receive_message(9, 1, &chat_payload("Visitor", "at the boundary"));
            app.update(frame(1.0), &InputFrame::default());
            assert_eq!(app.view.player.eye.distance(peer), CHAT_RANGE_METERS);
            assert_eq!(chat_runs(&app).len(), 2);

            // A new target does not move the displayed speaker until interpolation runs.
            app.receive_unreliable_pose(9, 2, &pose_payload_at(peer + Vec3::X * 0.01));
            app.rebuild_overlay();
            assert_eq!(app.remote_players[&9].displayed_position, peer);
            assert_eq!(chat_runs(&app).len(), 2);
            app.update(frame(1.1), &InputFrame::default());
            assert!(chat_runs(&app).is_empty());
            let name = app
                .scene
                .text
                .iter()
                .find(|run| run.text == "Visitor")
                .unwrap();
            assert_eq!(name.size, NAMEPLATE_FONT_SIZE);
            assert_eq!(name.anchor, TextAnchor::BottomCenter);
            if minimal_ui {
                assert_identity_only_overlay(&app);
            }

            app.receive_unreliable_pose(9, 3, &pose_payload_at(peer - Vec3::X * 0.1));
            app.rebuild_overlay();
            assert!(chat_runs(&app).is_empty());
            app.update(frame(1.2), &InputFrame::default());
            assert_eq!(chat_runs(&app).len(), 2);
            assert!(
                chat_runs(&app)
                    .iter()
                    .all(|run| run.text == "at the boundary")
            );
            assert_eq!(app.avatar_chat[&9].received_at, Some(1.0));
        }
    }

    #[test]
    fn remote_chat_range_is_independent_of_zoom_and_perspective() {
        for distance in [9.5, 10.5] {
            let mut app = first_person_app();
            app.set_minimal_ui(true);
            let local = Vec3::new(-5.0, EYE_HEIGHT, 5.0);
            let peer = local + Vec3::X * distance;
            app.scene.camera.eye = local;
            app.scene.camera.target = local + Vec3::X;
            app.controller = FirstPersonController::from_camera(&app.scene.camera);
            enter_peer(&mut app, 9);
            app.receive_unreliable_pose(9, 1, &pose_payload_at(peer));
            app.receive_message(9, 1, &chat_payload("Visitor", "physical proximity"));
            for (step, (toggle_perspective, zoom_delta, camera_distance)) in [
                (false, 0.0, 0.0),
                (true, 0.0, 2.0),
                (false, -100.0, 6.0),
                (false, 100.0, 0.0),
            ]
            .into_iter()
            .enumerate()
            {
                app.update(
                    FrameContext {
                        delta_seconds: 1.0,
                        ..frame(1.0 + step as f64 * 0.1)
                    },
                    &InputFrame {
                        toggle_perspective,
                        zoom_delta,
                        ..InputFrame::default()
                    },
                );
                assert_eq!(app.view.player.eye, local);
                assert_eq!(app.view.third_person(), camera_distance > 0.0);
                assert!((app.scene.camera.eye.distance(local) - camera_distance).abs() < 0.001);
                if app.view.third_person() {
                    assert!(app.scene.camera.eye.distance(peer) > CHAT_RANGE_METERS);
                }
                assert_eq!(
                    chat_runs(&app).len(),
                    if distance < CHAT_RANGE_METERS { 2 } else { 0 }
                );
                assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
                assert_eq!(app.avatar_chat[&9].received_at, Some(1.0));
            }
        }
    }

    #[test]
    fn out_of_range_chat_replaces_fades_and_expires_without_restarting_on_return() {
        let mut app = first_person_app();
        app.set_minimal_ui(true);
        app.scene.camera.eye = Vec3::new(-5.0, EYE_HEIGHT, 5.0);
        app.scene.camera.target = app.scene.camera.eye + Vec3::X;
        app.controller = FirstPersonController::from_camera(&app.scene.camera);
        enter_peer(&mut app, 9);
        app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(4.0, EYE_HEIGHT, 5.0)));
        app.receive_message(9, 1, &chat_payload("Visitor", "first"));
        app.update(frame(1.0), &InputFrame::default());
        assert_eq!(chat_runs(&app).len(), 2);

        app.scene.camera.eye.x = -7.0;
        app.scene.camera.target = app.scene.camera.eye + Vec3::X;
        app.update(frame(2.0), &InputFrame::default());
        assert_identity_only_overlay(&app);
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
        assert_eq!(app.avatar_chat[&9].received_at, Some(1.0));

        app.receive_message(9, 2, &chat_payload("Visitor", "replacement while distant"));
        app.update(frame(3.0), &InputFrame::default());
        assert!(chat_runs(&app).is_empty());
        assert_eq!(app.avatar_chat.len(), 1);
        assert_eq!(app.avatar_chat[&9].text, "replacement while distant");
        assert_eq!(app.avatar_chat[&9].received_at, Some(3.0));
        assert_eq!(app.chat_history.len(), 2);
        assert_eq!(app.chat_notifications.len(), 2);
        assert_eq!(app.last_remote_reliable_sequences[&9], 2);
        app.update(frame(8.0), &InputFrame::default());
        assert!(chat_runs(&app).is_empty());
        assert_eq!(app.avatar_chat[&9].opacity(8.0), 0.5);

        app.scene.camera.eye.x = -5.0;
        app.scene.camera.target = app.scene.camera.eye + Vec3::X;
        app.update(frame(8.0), &InputFrame::default());
        let runs = chat_runs(&app);
        assert_eq!(runs.len(), 2);
        assert!(
            runs.iter()
                .all(|run| run.text == "replacement while distant" && run.color[3] == 0.5)
        );
        assert_eq!(app.avatar_chat[&9].received_at, Some(3.0));

        app.scene.camera.eye.x = -7.0;
        app.scene.camera.target = app.scene.camera.eye + Vec3::X;
        app.update(frame(9.0), &InputFrame::default());
        assert!(app.avatar_chat.is_empty());
        app.scene.camera.eye.x = -5.0;
        app.scene.camera.target = app.scene.camera.eye + Vec3::X;
        app.update(frame(10.0), &InputFrame::default());
        assert_identity_only_overlay(&app);
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
    }

    #[test]
    fn own_chat_has_no_range_limit_but_overhead_still_requires_visibility() {
        for third_person in [false, true] {
            let mut app = if third_person {
                FirstPersonLab::new()
            } else {
                first_person_app()
            };
            app.set_minimal_ui(true);
            app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
            assert!(app.queue_validated_chat("own echo".to_owned()));
            app.update(frame(1.0), &InputFrame::default());
            let local = Vec3::new(5.0, EYE_HEIGHT, 5.0);
            app.view.player.eye = local;
            let distant_camera = Vec3::new(-6.0, EYE_HEIGHT, 5.0);
            assert!(distant_camera.distance(local) > CHAT_RANGE_METERS);
            // Isolate presentation: distant clear view, a wall, then off-screen.
            for (eye, target, visible) in [
                (distant_camera, local, true),
                (Vec3::new(0.0, EYE_HEIGHT, -2.0), local, false),
                (distant_camera, distant_camera - Vec3::X, false),
            ] {
                app.scene.camera.eye = eye;
                app.scene.camera.target = target;
                app.rebuild_overlay();
                let runs = chat_runs(&app);
                assert_eq!(runs.len(), if third_person && !visible { 0 } else { 2 });
                if !runs.is_empty() {
                    assert!(runs.iter().all(|run| run.text == "own echo"));
                    if third_person {
                        let name = app
                            .scene
                            .text
                            .iter()
                            .find(|run| run.text == app.display_name)
                            .unwrap();
                        assert_eq!(runs[1].anchor, TextAnchor::BottomCenter);
                        assert_eq!(
                            runs[1].position,
                            name.position - Vec2::Y * (NAMEPLATE_FONT_SIZE + 6.0)
                        );
                    } else {
                        assert_eq!(runs[1].anchor, TextAnchor::TopCenter);
                        assert_eq!(runs[1].position, Vec2::new(640.0, 52.0));
                    }
                }
                assert_eq!(app.avatar_chat[&42].received_at, Some(1.0));
            }
        }
    }

    #[test]
    fn avatar_overlays_follow_physical_line_of_sight() {
        for (viewer, peer, visible) in [
            (
                waypoint("corridor_turn"),
                waypoint("auditorium_entry"),
                true,
            ),
            (
                grounded_eye(waypoint("corridor_turn") - Vec3::Z * 5.0),
                waypoint("row_b"),
                false,
            ),
            (waypoint("lobby_center"), waypoint("auditorium_back"), false),
            (
                waypoint("passage_lobby"),
                grounded_eye(waypoint("passage_lobby").lerp(waypoint("corridor_turn"), 0.15)),
                true,
            ),
            (waypoint("lobby_west"), waypoint("lobby_center"), true),
        ] {
            for minimal_ui in [false, true] {
                let mut app = first_person_app();
                app.set_minimal_ui(minimal_ui);
                app.scene.camera.eye = viewer;
                app.scene.camera.target = peer;
                app.controller = FirstPersonController::from_camera(&app.scene.camera);
                enter_peer(&mut app, 9);
                app.receive_unreliable_pose(9, 1, &pose_payload_at(peer));
                app.receive_message(9, 1, &chat_payload("Visitor", "occlusion test"));
                app.update(frame(1.0), &InputFrame::default());
                assert_eq!(
                    app.scene
                        .text
                        .iter()
                        .filter(|run| run.text == "occlusion test")
                        .count(),
                    if visible { 2 } else { 0 }
                );
                assert_eq!(
                    app.scene.text.iter().any(|run| run.text == "Visitor"),
                    visible
                );
                assert_eq!(app.scene.sprites.len(), 1 + app.npcs.visible_count());
                assert_eq!(app.avatar_chat[&9].received_at, Some(1.0));
            }
        }
    }

    #[test]
    fn npc_nameplates_track_visible_grounded_sprites_without_look_at_activation() {
        for minimal_ui in [false, true] {
            let mut app = first_person_app();
            app.set_minimal_ui(minimal_ui);
            app.scene.camera.eye.x = 2.0;
            app.scene.camera.target.x = 2.0;
            let mut previous_position = None;
            for time in [0.0, 0.125] {
                app.update(frame(time), &InputFrame::default());
                let labels: Vec<_> = app.npcs.nameplates().collect();
                assert_eq!(labels.len(), app.scene.sprites.len());
                for ((name, _), sprite) in labels.iter().zip(&app.scene.sprites) {
                    let top = sprite.transform.translation
                        + Vec3::Y * (sprite.transform.scale * 0.5 + 0.05);
                    let expected =
                        if theater::has_line_of_sight(app.scene.camera.eye, top, &app.colliders) {
                            project_to_screen(
                                &app.scene.camera,
                                1280.0 / 720.0,
                                app.viewport_size,
                                top,
                            )
                        } else {
                            None
                        };
                    let actual = app.scene.text.iter().find(|run| run.text == *name);
                    assert_eq!(actual.is_some(), expected.is_some(), "{name}");
                    if let (Some(run), Some(position)) = (actual, expected) {
                        assert!(run.position.abs_diff_eq(position, 0.0001));
                    }
                }
                let name = app
                    .scene
                    .text
                    .iter()
                    .find(|run| run.text == "Curious Otter")
                    .unwrap();
                assert!(name.position.x < app.viewport_size.x as f32 * 0.5);
                if let Some(previous) = previous_position {
                    assert_eq!(name.position, previous);
                }
                previous_position = Some(name.position);
                assert!(npcs::get_npc_interaction_context().is_none());
                if minimal_ui {
                    assert_identity_only_overlay(&app);
                }
            }
            // Aimed at an authored NPC, but the lobby divider still hides its label.
            app.scene.camera.target = waypoint("auditorium_left");
            app.controller = FirstPersonController::from_camera(&app.scene.camera);
            app.update(frame(1.0), &InputFrame::default());
            assert!(!app.scene.text.iter().any(|run| run.text == "Gentle Falcon"));
            assert!(npcs::get_npc_interaction_context().is_none());
        }
    }

    #[test]
    fn player_and_npc_nameplates_are_white_and_bold() {
        for local_entity_id in [None, Some(7), Some(42)] {
            let mut app = FirstPersonLab::with_display_name("Local");
            app.set_minimal_ui(true);
            if let Some(entity_id) = local_entity_id {
                app.handle_realtime_event(RealtimeEvent::Connected { entity_id });
            }
            enter_peer(&mut app, 9);
            assert!(app.receive_profile(9, MESSAGE_VERSION, "Visitor"));
            app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, 1.0)));
            app.update(frame(1.0), &InputFrame::default());

            let local = app
                .scene
                .text
                .iter()
                .find(|run| run.text == "Local")
                .unwrap();
            assert_eq!(local.color, [1.0; 4]);
            assert_eq!(local.weight, TextWeight::Bold);
            let remote = app
                .scene
                .text
                .iter()
                .find(|run| run.text == "Visitor")
                .unwrap();
            assert_eq!(remote.color, [1.0; 4]);
            assert_eq!(remote.weight, TextWeight::Bold);
            let mut visible_npc_names = 0;
            for (name, _) in app.npcs.nameplates() {
                if let Some(run) = app.scene.text.iter().find(|run| run.text == name) {
                    assert_eq!(run.color, [1.0; 4]);
                    assert_eq!(run.weight, TextWeight::Bold);
                    assert_eq!(run.size, local.size);
                    assert_eq!(run.anchor, local.anchor);
                    assert_eq!(run.bounds, local.bounds);
                    assert_eq!(run.world_space, local.world_space);
                    visible_npc_names += 1;
                }
            }
            assert!(visible_npc_names > 0);
        }
    }

    #[test]
    fn avatar_label_projection_respects_walls_floor_ceiling_and_screen_bounds() {
        let app = first_person_app();
        let viewport = UVec2::new(1280, 720);
        for (viewer, peer) in [
            (waypoint("lobby_center"), waypoint("auditorium_back")),
            (
                Vec3::new(0.0, 1.0, -5.0),
                grounded_eye(Vec3::new(0.0, 0.0, -7.0)),
            ),
            (Vec3::new(0.0, 5.0, 8.0), waypoint("lobby_center")),
            (Vec3::new(8.0, 8.0, -10.5), Vec3::new(11.0, 7.44, -10.5)),
        ] {
            let head = peer + Vec3::Y * (PLAYER_BILLBOARD_HEIGHT - EYE_HEIGHT + 0.05);
            let camera = Camera {
                eye: viewer,
                target: head,
                ..app.scene.camera.clone()
            };
            assert!(project_to_screen(&camera, 1280.0 / 720.0, viewport, head).is_some());
            assert!(avatar_label_position(&camera, viewport, &app.colliders, peer).is_none());
        }
        let peer = waypoint("lobby_center");
        assert!(avatar_label_position(&app.scene.camera, viewport, &app.colliders, peer).is_some());
        for peer in [theater::SPAWN + Vec3::Z, Vec3::new(11.0, EYE_HEIGHT, 7.0)] {
            assert!(
                avatar_label_position(&app.scene.camera, viewport, &app.colliders, peer).is_none()
            );
        }
        for viewport in [UVec2::ZERO, UVec2::new(1, 720), UVec2::new(1280, 1)] {
            assert!(
                avatar_label_position(&app.scene.camera, viewport, &app.colliders, peer).is_none()
            );
        }
    }

    #[test]
    fn hidden_avatar_chat_keeps_its_original_fade_and_expiry_clock() {
        let mut app = first_person_app();
        app.set_minimal_ui(true);
        app.scene.camera.eye.x = 9.5;
        app.scene.camera.target.x = 9.5;
        enter_peer(&mut app, 9);
        app.receive_unreliable_pose(
            9,
            1,
            &pose_payload_at(grounded_eye(Vec3::new(9.5, 0.0, -2.0))),
        );
        app.receive_message(9, 1, &chat_payload("Visitor", "do not reset"));
        app.update(frame(1.0), &InputFrame::default());
        // The corridor ramp puts this peer just beyond 10 m in 3D despite a clear view.
        assert!(
            app.view
                .player
                .eye
                .distance(app.remote_players[&9].displayed_position)
                > CHAT_RANGE_METERS
        );
        assert!(chat_runs(&app).is_empty());
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));

        // Moving left puts the divider between the viewer and the head anchor.
        app.scene.camera.eye.x = 0.0;
        app.scene.camera.target.x = 0.0;
        app.update(frame(6.0), &InputFrame::default());
        assert_identity_only_overlay(&app);
        assert!(!app.scene.text.iter().any(|run| run.text == "Visitor"));
        assert_eq!(app.avatar_chat[&9].received_at, Some(1.0));
        assert_eq!(app.avatar_chat[&9].opacity(6.0), 0.5);

        app.scene.camera.eye.x = 9.5;
        app.scene.camera.target.x = 9.5;
        app.update(frame(6.0), &InputFrame::default());
        assert!(chat_runs(&app).is_empty());
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
        assert_eq!(app.avatar_chat[&9].opacity(6.0), 0.5);
        assert_eq!(app.avatar_chat[&9].received_at, Some(1.0));

        app.scene.camera.eye.x = 0.0;
        app.scene.camera.target.x = 0.0;
        app.update(frame(7.0), &InputFrame::default());
        assert!(app.avatar_chat.is_empty());
        app.scene.camera.eye.x = 9.5;
        app.scene.camera.target.x = 9.5;
        app.update(frame(8.0), &InputFrame::default());
        assert_identity_only_overlay(&app);
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
    }

    #[test]
    fn avatar_chat_replaces_resets_fades_and_expires_without_a_feed() {
        let mut app = FirstPersonLab::new();
        app.set_minimal_ui(true);
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, 1.0)));
        app.update(frame(1.0), &InputFrame::default());
        app.receive_message(9, 1, &chat_payload("Visitor", "first"));
        app.update(frame(1.0), &InputFrame::default());
        app.update(frame(5.0), &InputFrame::default());
        assert_eq!(chat_runs(&app).len(), 2);
        assert_eq!(app.scene.text[1].color[3], 1.0);
        app.update(frame(6.0), &InputFrame::default());
        assert_eq!(app.scene.text[1].color[3], 0.5);
        app.receive_message(9, 2, &chat_payload("Visitor", "replacement"));
        app.update(frame(6.1), &InputFrame::default());
        assert_eq!(app.avatar_chat.len(), 1);
        assert!(chat_runs(&app).iter().all(|run| run.text == "replacement"));
        assert_eq!(app.scene.text[1].color[3], 1.0);
        // Replayed/invalid messages cannot restart the timer.
        app.receive_message(9, 2, &chat_payload("Visitor", "replayed"));
        app.receive_message(9, 3, &chat_payload("Visitor", ""));
        app.update(frame(11.1), &InputFrame::default());
        assert_eq!(app.scene.text[1].color[3], 0.5);
        app.update(frame(12.1), &InputFrame::default());
        assert!(app.avatar_chat.is_empty());
        assert_identity_only_overlay(&app);
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
    }

    #[test]
    fn avatar_chat_tracks_pose_and_cleans_up_on_leave_and_reconnect() {
        let mut app = FirstPersonLab::new();
        app.set_minimal_ui(true);
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &chat_payload("Visitor", "before pose"));
        app.update(frame(1.0), &InputFrame::default());
        assert_identity_only_overlay(&app);
        assert!(!app.scene.text.iter().any(|run| run.text == "Visitor"));
        app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, 1.0)));
        app.update(frame(1.1), &InputFrame::default());
        let position = app.scene.text[1].position;
        app.receive_unreliable_pose(9, 2, &pose_payload_at(Vec3::new(1.5, EYE_HEIGHT, 1.0)));
        app.update(frame(1.2), &InputFrame::default());
        assert_ne!(position, app.scene.text[1].position);
        app.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 9 });
        assert!(app.avatar_chat.is_empty());
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &chat_payload("Visitor", "again"));
        assert_eq!(app.avatar_chat.len(), 1);
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 8 });
        assert!(app.avatar_chat.is_empty());
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &chat_payload("Visitor", "last"));
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        assert!(app.avatar_chat.is_empty());
    }

    #[test]
    fn long_avatar_chat_wraps_unicode_without_truncating() {
        let text = "界".repeat(MAX_CHAT_CHARS);
        let wrapped = wrap_avatar_chat(&text);
        assert!(wrapped.lines().all(|line| line.chars().count() <= 40));
        assert_eq!(wrapped.replace('\n', ""), text);
        assert_eq!(wrap_avatar_chat("short message"), "short message");
    }

    #[test]
    fn natural_video_dimensions_produce_valid_aspects_and_reject_unready_metadata() {
        for (width, height) in [(0, 0), (0, 1080), (1920, 0)] {
            assert!(video_aspect(width, height).is_none());
        }
        assert_eq!(
            video_aspect(1920, 1080),
            Some(theater::DEFAULT_VIDEO_ASPECT)
        );
        assert_eq!(video_aspect(1440, 1080), Some(4.0 / 3.0));
        assert_eq!(video_aspect(720, 1280), Some(9.0 / 16.0));
        assert!(video_aspect(u32::MAX, 1).unwrap().is_finite());
        assert!(video_aspect(1, u32::MAX).unwrap() > 0.0);
    }

    #[test]
    fn native_video_aspect_changes_are_idempotent_and_preserve_geometry_and_lighting() {
        let mut app = FirstPersonLab::new();
        assert!(app.set_room_lighting(0.7, 0.12));
        let screen = app.assets.textures[4].handle;
        let matching: Vec<_> = app
            .scene
            .meshes
            .iter()
            .enumerate()
            .filter_map(|(index, mesh)| {
                (mesh.texture == Some(screen) && mesh.texture_fit.is_some()).then_some(index)
            })
            .collect();
        assert_eq!(matching.len(), 2); // Screen and fitted projection beam; not the lens.
        assert!(!app.apply_video_aspect(theater::DEFAULT_VIDEO_ASPECT));
        for aspect in [4.0 / 3.0, 9.0 / 16.0, 2.39, theater::DEFAULT_VIDEO_ASPECT] {
            let before = app.scene.clone();
            assert!(app.apply_video_aspect(aspect));
            let mut expected = before;
            for &index in &matching {
                expected.meshes[index].texture_fit = app.scene.meshes[index].texture_fit;
                assert_eq!(
                    app.scene.meshes[index].texture_fit.unwrap().content_aspect,
                    aspect
                );
            }
            expected.revision = expected.revision.next();
            assert_eq!(app.scene, expected);

            assert!(!app.apply_video_aspect(aspect));
            assert_eq!(app.scene, expected);
        }
        let before = app.scene.clone();
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0] {
            assert!(!app.apply_video_aspect(invalid));
            assert_eq!(app.scene, before);
        }
        app.update(frame(1.0), &InputFrame::default());
        assert_eq!(app.scene.meshes, before.meshes);
        assert_eq!(app.scene.revision, before.revision);
    }

    #[test]
    fn runtime_lighting_changes_only_the_selected_room_and_tints_its_avatars() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        enter_peer(&mut app, 10);
        app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, 2.0)));
        app.receive_unreliable_pose(
            10,
            1,
            &pose_payload_at(grounded_eye(Vec3::new(1.0, 0.0, -2.0))),
        );
        app.update_remote_players(0.0);
        assert_eq!(app.scene.sprites[0].tint, [1.0; 4]);
        assert_eq!(app.scene.sprites[1].tint, [0.20, 0.20, 0.20, 1.0]);
        let lobby_light = app.scene.meshes[0].lighting;
        assert!(app.set_room_lighting(1.0, 0.0));
        assert_eq!(app.scene.meshes[0].lighting, lobby_light);
        let dark = app.scene.meshes[1].lighting.unwrap();
        assert_eq!(dark.ambient, 0.0);
        assert_eq!(dark.intensity, 0.0);
        app.update_remote_players(0.0);
        assert_eq!(app.scene.sprites[1].tint, [0.0, 0.0, 0.0, 1.0]);
        assert!(!app.set_room_lighting(f32::NAN, 1.0));
        assert_eq!(app.scene.meshes[0].lighting, lobby_light);
        assert!(app.set_room_lighting(2.0, -1.0));
        assert_eq!(app.lobby_light_level, 1.0);
        assert_eq!(app.auditorium_light_level, 0.0);
    }

    #[test]
    fn billboard_is_upright_grounded_and_faces_each_viewer() {
        let sprite = SpriteHandle::new();
        for eye in [
            waypoint("lobby_center"),
            waypoint("row_b"),
            waypoint("auditorium_back"),
            grounded_eye(Vec3::new(waypoint("row_b").x, 0.0, theater::MIN_Z + 2.0)),
        ] {
            for viewer in [
                eye + Vec3::X,
                eye - Vec3::X,
                eye + Vec3::Z,
                eye - Vec3::Z,
                eye,
            ] {
                let instance = avatar_instance(sprite, eye, viewer);
                assert_eq!(instance.sprite, sprite);
                assert_eq!(instance.space, SpriteSpace::World);
                assert_eq!(instance.tint, [1.0; 4]);
                assert!((instance.transform.scale - 2.21).abs() < 1e-5);
                assert!(
                    (instance.transform.translation.y
                        - instance.transform.scale * 0.5
                        - theater::floor_height(eye))
                    .abs()
                        < 1e-5
                );
                assert!((instance.transform.rotation * Vec3::Y - Vec3::Y).length() < 1e-5);
                let direction = (viewer - eye).normalize_or_zero();
                if direction != Vec3::ZERO {
                    assert!((instance.transform.rotation * Vec3::Z - direction).length() < 1e-5);
                }
            }
        }
    }

    #[test]
    fn independent_viewers_select_the_same_avatar_and_remove_it_on_leave() {
        let mut a = first_person_app();
        let mut b = first_person_app();
        for app in [&mut a, &mut b] {
            enter_peer(app, 42);
            app.receive_pose(
                42,
                validate_pose(grounded_eye(Vec3::new(1.0, 0.0, -2.0)), -Vec3::Z).unwrap(),
            );
            app.update_remote_players(0.1);
            assert_eq!(app.scene.meshes.len(), app.base_mesh_count);
            assert_eq!(app.scene.sprites.len(), 1);
            assert_eq!(
                app.scene.sprites[0].sprite,
                app.avatar_textures[avatars::avatar_index(42)]
            );
        }
        assert_eq!(a.scene.sprites[0].sprite, b.scene.sprites[0].sprite);
        a.handle_realtime_event(RealtimeEvent::EntityLeft { entity_id: 42 });
        a.update_remote_players(0.1);
        assert!(a.scene.sprites.is_empty());
    }

    fn frame(elapsed_seconds: f64) -> FrameContext {
        FrameContext {
            delta_seconds: 1.0 / 60.0,
            elapsed_seconds,
            viewport_size: UVec2::new(1280, 720),
        }
    }

    fn pose_payload() -> Vec<u8> {
        pose_payload_at(grounded_eye(Vec3::new(1.0, 0.0, -2.0)))
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
            "position": grounded_eye(Vec3::new(1.0, 0.0, -2.0)).to_array(),
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

    fn assert_guest_name_format(name: &str) {
        let words: Vec<_> = name.split(' ').collect();
        assert_eq!(words.len(), 2);
        for word in words {
            assert!(word.as_bytes()[0].is_ascii_uppercase());
            assert!(word.as_bytes()[1..].iter().all(u8::is_ascii_lowercase));
        }
        assert!(validate_display_name(name).is_some());
    }

    #[test]
    fn scripted_npc_names_are_unique_and_stable_across_apps() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../assets/npcs.json")).unwrap();
        let names: Vec<_> = catalog["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|definition| definition["displayName"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "Curious Otter",
                "Gentle Falcon",
                "Sunny Panda",
                "Brave Robin",
                "Merry Fox"
            ]
        );
        assert_eq!(
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            5
        );
        for name in &names {
            assert_guest_name_format(name);
        }
        let mut first = FirstPersonLab::new();
        let mut second = FirstPersonLab::new();
        let mut seen = std::collections::HashSet::new();
        for seconds in 0..600 {
            first
                .npcs
                .update(f64::from(seconds), Vec3::ZERO, 0.0, &first.colliders);
            second
                .npcs
                .update(f64::from(seconds), Vec3::ZERO, 0.0, &second.colliders);
            let a: Vec<_> = first
                .npcs
                .nameplates()
                .map(|(name, _)| name.to_owned())
                .collect();
            let b: Vec<_> = second
                .npcs
                .nameplates()
                .map(|(name, _)| name.to_owned())
                .collect();
            assert_eq!(a, b);
            for name in a {
                assert!(names.contains(&name.as_str()));
                seen.insert(name);
            }
        }
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn default_name_uses_adjective_noun_and_survives_reconnection() {
        let mut app = FirstPersonLab::new();
        let name = app.display_name.clone();
        assert_guest_name_format(&name);
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
        assert_guest_name_format(&invalid.display_name);
        for custom in ["BrightCalm", "SuperCoder", "Curious Otter"] {
            assert_eq!(
                FirstPersonLab::with_display_name(custom).display_name,
                custom
            );
        }
        assert!(validate_display_name(&invalid.display_name).is_some());
    }

    fn host_submit(app: &mut FirstPersonLab, text: &str, now: f64) -> bool {
        app.host_chat
            .borrow_mut()
            .submit(text, now, MAX_PENDING_PUBLISHES)
    }

    #[test]
    fn host_chat_local_and_remote_notifications_are_validated_once_with_decimal_ids() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.set_minimal_ui(true);
        app.update(
            frame(0.0),
            &InputFrame {
                toggle_perspective: true,
                ..InputFrame::default()
            },
        );
        app.handle_realtime_event(RealtimeEvent::Connected {
            entity_id: u64::MAX,
        });
        assert!(host_submit(&mut app, "  hello 🌍  ", 0.0));
        assert!(app.chat_notifications.is_empty()); // Command acceptance is not a local echo.
        app.update(frame(0.0), &InputFrame::default());
        app.update(frame(0.01), &InputFrame::default());
        let remote_id = u64::MAX - 1;
        enter_peer(&mut app, remote_id);
        app.receive_message(remote_id, 1, &chat_payload("Visitor", " remote "));
        app.receive_message(remote_id, 1, &chat_payload("Duplicate", "ignored"));
        app.receive_message(u64::MAX, 100, &chat_payload("Self", "ignored"));
        app.receive_message(123, 100, &chat_payload("Unknown", "ignored"));
        app.receive_message(remote_id, 2, &chat_payload("Visitor", "bad\ttext"));
        for (name, text) in [
            ("bad\nname".to_owned(), "invalid name".to_owned()),
            (
                "x".repeat(MAX_DISPLAY_NAME_CHARS + 1),
                "long name".to_owned(),
            ),
            ("Visitor".to_owned(), "x".repeat(MAX_CHAT_CHARS + 1)),
            ("Visitor".to_owned(), "🌍".repeat(MAX_CHAT_CHARS + 1)),
            ("Visitor".to_owned(), "x".repeat(MAX_MESSAGE_BYTES + 1)),
        ] {
            app.receive_message(remote_id, 2, &chat_payload(&name, &text));
        }
        app.receive_message(
            remote_id,
            2,
            r#"{"kind":"chat","version":1,"display_name":"Visitor","text":"\ud800"}"#,
        );
        app.receive_message(
            remote_id,
            2,
            r#"{"kind":"chat","version":2,"display_name":"Visitor","text":"bad version"}"#,
        );
        app.receive_message(
            remote_id,
            2,
            r#"{"kind":"chat","version":1,"display_name":"Visitor","text":"extra","extra":true}"#,
        );
        assert_eq!(
            app.chat_notifications,
            vec![
                (
                    "18446744073709551615".to_owned(),
                    "Ada".to_owned(),
                    "hello 🌍".to_owned(),
                    true
                ),
                (
                    "18446744073709551614".to_owned(),
                    "Visitor".to_owned(),
                    "remote".to_owned(),
                    false
                ),
            ]
        );
        assert_eq!(app.last_remote_reliable_sequences[&remote_id], 1);
        assert_eq!(chat_runs(&app).len(), 2);
        assert!(chat_runs(&app).iter().all(|run| run.text == "hello 🌍"));
        assert_eq!(app.scene.text[1].anchor, TextAnchor::TopCenter);
        assert!(app.scene.ui.is_empty());
        assert!(app.scene.status_lines.is_empty());
        let chats = app.pending_publishes.iter().filter(|command| matches!(command,
            RealtimeCommand::PublishReliable { payload, .. }
            if matches!(serde_json::from_str::<FirstPersonMessage>(payload), Ok(FirstPersonMessage::Chat { .. }))
        )).count();
        assert_eq!(chats, 1);
    }

    #[test]
    fn host_command_queue_is_validated_rate_limited_and_capacity_bounded() {
        let mut commands = HostChatCommands::default();
        assert!(!commands.submit("offline", 0.0, MAX_PENDING_PUBLISHES));
        commands.capacity = MAX_PENDING_PUBLISHES;
        for text in [
            "",
            " ",
            "bad\0text",
            "bad\ntext",
            "bad\ttext",
            &"x".repeat(MAX_CHAT_CHARS + 1),
            &"🌍".repeat(MAX_CHAT_CHARS + 1),
            &" ".repeat(MAX_CHAT_BYTES + 1),
        ] {
            assert!(!commands.submit(text, 0.0, MAX_PENDING_PUBLISHES));
        }
        for now in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!commands.submit("invalid clock", now, MAX_PENDING_PUBLISHES));
        }
        assert!(!commands.submit("no transport slot", 0.0, 0));
        assert!(commands.submit(&"🌍".repeat(MAX_CHAT_CHARS), 0.0, MAX_PENDING_PUBLISHES));
        assert_eq!(commands.pending.front().unwrap().len(), MAX_CHAT_BYTES);
        assert!(!commands.submit("too fast", 249.0, MAX_PENDING_PUBLISHES));
        assert!(!commands.submit("clock went backwards", -1.0, MAX_PENDING_PUBLISHES));
        for i in 1..MAX_HOST_CHAT_PENDING {
            assert!(commands.submit(
                "bounded",
                i as f64 * HOST_CHAT_INTERVAL_MS,
                MAX_PENDING_PUBLISHES
            ));
        }
        assert!(!commands.submit("overflow", 10_000.0, MAX_PENDING_PUBLISHES));
        assert_eq!(commands.pending.len(), MAX_HOST_CHAT_PENDING);
        commands.reset_connection();
        commands.capacity = 1;
        assert!(commands.submit("one available Rust slot", 0.0, MAX_PENDING_PUBLISHES));
        assert!(!commands.submit("no available Rust slot", 250.0, MAX_PENDING_PUBLISHES));
    }

    #[test]
    fn host_echo_waits_for_reliable_queue_success_and_uses_current_profile_and_sequence() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        assert!(host_submit(&mut app, "queued", 0.0));
        app.apply_display_name("Grace".to_owned());
        let filler = FirstPersonMessage::Profile {
            version: MESSAGE_VERSION,
            display_name: "Grace".to_owned(),
        };
        while app.pending_publishes.len() < MAX_PENDING_PUBLISHES {
            assert!(app.queue_message(&filler));
        }
        app.process_host_chat(MAX_PENDING_PUBLISHES);
        assert!(app.chat_notifications.is_empty());
        assert_eq!(app.host_chat.borrow().pending.len(), 1);
        let last_sequence = app.next_sequence;
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        app.process_host_chat(0); // A full JS reliable outbox must defer without echo or a fatal write.
        assert!(app.chat_notifications.is_empty());
        app.process_host_chat(MAX_PENDING_PUBLISHES);
        assert_eq!(app.next_sequence, last_sequence + 1);
        assert_eq!(
            app.chat_notifications,
            vec![(
                "7".to_owned(),
                "Grace".to_owned(),
                "queued".to_owned(),
                true
            )]
        );
        assert!(app.host_chat.borrow().pending.is_empty());
        assert!(
            matches!(app.pending_publishes.front().unwrap(), RealtimeCommand::PublishReliable { payload, .. }
            if matches!(serde_json::from_str::<FirstPersonMessage>(payload), Ok(FirstPersonMessage::Chat { display_name, text, .. }) if display_name == "Grace" && text == "queued"))
        );
        app.next_sequence = u64::MAX - 1;
        app.sync_host_chat_capacity();
        assert!(host_submit(&mut app, "last sequence slot", 250.0));
        assert!(!host_submit(
            &mut app,
            "cannot reserve another sequence",
            500.0
        ));
        app.next_sequence = u64::MAX;
        app.sync_host_chat_capacity();
        assert!(!host_submit(&mut app, "exhausted", 250.0));
    }

    #[test]
    fn host_submissions_are_dropped_on_disconnect_and_even_direct_reconnect() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        assert!(host_submit(&mut app, "stale pending command", 0.0));
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        assert!(!host_submit(&mut app, "offline", 250.0));
        assert!(app.host_chat.borrow().pending.is_empty());
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 8 });
        assert!(host_submit(&mut app, "stale direct retry", 0.0));
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 9 });
        app.update(frame(0.0), &InputFrame::default());
        assert!(app.chat_notifications.is_empty());
        assert!(host_submit(&mut app, "fresh", 0.0));
        app.update(frame(0.01), &InputFrame::default());
        assert_eq!(
            app.chat_notifications,
            vec![("9".to_owned(), "Ada".to_owned(), "fresh".to_owned(), true)]
        );
        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 10 });
        assert!(app.pending_publishes.iter().all(|command| !matches!(command,
            RealtimeCommand::PublishReliable { payload, .. }
            if matches!(serde_json::from_str::<FirstPersonMessage>(payload), Ok(FirstPersonMessage::Chat { .. }))
        )));
    }

    #[test]
    fn host_focus_pauses_camera_and_native_chat_offline_and_cleans_up_on_cancel() {
        for minimal in [false, true] {
            let mut app = FirstPersonLab::with_display_name("Ada");
            app.set_minimal_ui(minimal);
            app.chat_open = true;
            app.chat_draft = "stale draft".to_owned();
            app.chat_feedback = Some("stale feedback".to_owned());
            let eye = app.scene.camera.eye;
            let target = app.scene.camera.target;
            let player_eye = app.view.player.eye;
            app.host_chat.borrow_mut().set_focus(true);
            assert_eq!(app.pointer_mode(), PointerMode::None);
            assert_eq!(app.text_input_mode(), TextInputMode::Disabled);
            let held = InputFrame {
                movement: Vec2::Y,
                look_delta: Vec2::new(30.0, 10.0),
                text_events: vec![
                    TextInputEvent::Open,
                    TextInputEvent::Insert("/name Wrong".to_owned()),
                    TextInputEvent::Submit,
                ],
                ..InputFrame::default()
            };
            for i in 0..3 {
                app.update(frame(i as f64 * 0.1), &held);
            }
            assert_eq!(app.scene.camera.eye, eye);
            assert_eq!(app.scene.camera.target, target);
            assert_eq!(app.display_name, "Ada");
            assert!(!app.chat_open);
            assert!(app.chat_draft.is_empty());
            assert!(app.chat_feedback.is_none());
            app.host_chat.borrow_mut().set_focus(false);
            app.update(frame(0.3), &held); // Never resume a stale held movement or text event.
            assert_eq!(app.scene.camera.eye, eye);
            assert!(!app.chat_open);
            app.update(frame(0.4), &InputFrame::default());
            app.update(
                frame(0.5),
                &InputFrame {
                    movement: Vec2::Y,
                    ..InputFrame::default()
                },
            );
            // The rear wall can hold the third-person camera still as the player moves.
            assert_ne!(app.view.player.eye, player_eye);
            assert_eq!(app.pointer_mode(), PointerMode::LockedLook);
            assert_eq!(
                app.text_input_mode(),
                if minimal {
                    TextInputMode::Disabled
                } else {
                    TextInputMode::Available
                }
            );
        }
    }

    #[test]
    fn host_focus_keeps_pose_generation_at_ten_hz_and_native_name_command_intact() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.chat_open = true;
        app.chat_draft = "/name Grace".to_owned();
        app.submit_chat();
        assert_eq!(app.display_name, "Grace");
        assert!(app.chat_notifications.is_empty());
        app.host_chat.borrow_mut().set_focus(true);
        assert!(host_submit(&mut app, "host chat", 0.0));
        let mut poses = 0;
        for elapsed in [0.0, 0.05, 0.1, 0.15, 0.2] {
            app.update(frame(elapsed), &InputFrame::default());
            let mut commands = Vec::new();
            app.drain_realtime_commands(&mut commands);
            poses += commands
                .iter()
                .filter(|command| {
                    matches!(command, RealtimeCommand::PublishPositionedUnreliable { .. })
                })
                .count();
        }
        assert_eq!(poses, 3);
        assert_eq!(app.chat_notifications.len(), 1);
        assert_eq!(app.chat_notifications[0].1, "Grace");
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
        let mut app = first_person_app();
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
            grounded_eye(Vec3::new(1.0, 0.0, -2.0))
        );
        assert_eq!(app.last_remote_pose_sequences[&9], 2);
        assert_eq!(app.last_remote_reliable_sequences[&9], 1);
        app.update_remote_players(1.0 / 60.0);
        assert_eq!(app.scene.meshes.len(), app.base_mesh_count);
        assert_eq!(app.scene.sprites.len(), 1);
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
        assert_eq!(*position, app.view.player.eye.as_dvec3().to_array());
        assert_eq!(decode_pose(payload).unwrap().position, app.view.player.eye);
        assert_ne!(app.view.player.eye, app.scene.camera.eye);
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
    fn enter_send_closes_compact_input_and_echoes_once_in_either_perspective() {
        for third_person in [false, true] {
            let mut app = if third_person {
                FirstPersonLab::new()
            } else {
                first_person_app()
            };
            app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 42 });
            app.update(
                frame(1.0),
                &InputFrame {
                    text_events: vec![
                        TextInputEvent::Open,
                        TextInputEvent::Insert("hello room".to_owned()),
                    ],
                    ..InputFrame::default()
                },
            );
            assert!(app.chat_open);
            assert_eq!(app.text_input_mode(), TextInputMode::Focused);
            assert_eq!(app.scene.ui.len(), 1);
            let UiElement::Rect(panel) = &app.scene.ui[0] else {
                panic!("expected compact input panel")
            };
            assert!(panel.size.x <= 540.0 && panel.size.y <= 66.0);
            assert!(app.chat_notifications.is_empty());
            app.update(
                frame(1.1),
                &InputFrame {
                    text_events: vec![TextInputEvent::Submit],
                    ..InputFrame::default()
                },
            );
            assert!(!app.chat_open);
            assert!(app.chat_draft.is_empty());
            assert!(app.scene.ui.is_empty());
            assert_eq!(app.text_input_mode(), TextInputMode::Available);
            assert_eq!(
                app.chat_notifications,
                vec![(
                    "42".to_owned(),
                    app.display_name.clone(),
                    "hello room".to_owned(),
                    true
                )]
            );
            assert_eq!(app.pending_publishes.iter().filter(|command| matches!(command,
                RealtimeCommand::PublishReliable { payload, .. }
                    if matches!(serde_json::from_str::<FirstPersonMessage>(payload), Ok(FirstPersonMessage::Chat { .. }))
            )).count(), 1);
            let runs = chat_runs(&app);
            assert_eq!(runs.len(), 2);
            assert!(runs.iter().all(|run| run.text == "hello room"));
            let name = app
                .scene
                .text
                .iter()
                .find(|run| run.text == app.display_name);
            if third_person {
                assert_eq!(runs[1].anchor, TextAnchor::BottomCenter);
                assert!(runs[1].position.y <= name.unwrap().position.y - NAMEPLATE_FONT_SIZE - 5.0);
            } else {
                assert!(name.is_none());
                assert_eq!(runs[1].anchor, TextAnchor::TopCenter);
                assert_eq!(runs[1].position, Vec2::new(640.0, 52.0));
            }
            app.update(frame(1.2), &InputFrame::default());
            let received_at = app.avatar_chat[&42].received_at.unwrap();
            app.update(
                frame(received_at + CHAT_HOLD_SECONDS + CHAT_FADE_SECONDS),
                &InputFrame::default(),
            );
            assert!(chat_runs(&app).is_empty());
            assert!(app.scene.ui.is_empty());
            assert_eq!(app.chat_notifications.len(), 1);
        }
    }

    #[test]
    fn full_ui_is_the_default_with_status_nameplates_and_chat() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &chat_payload("Visitor", "hello"));
        app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, 1.0)));
        app.update(frame(1.0), &InputFrame::default());

        assert!(!app.minimal_ui);
        assert_eq!(app.text_input_mode(), TextInputMode::Available);
        assert_eq!(app.pointer_mode(), PointerMode::LockedLook);
        assert_eq!(app.scene.status_lines, vec![app.network_status.clone()]);
        assert!(
            app.scene
                .text
                .iter()
                .any(|run| run.text.contains("Enter: chat"))
        );
        assert!(app.scene.text.iter().any(|run| run.text == "Visitor"));
        assert!(app.scene.text.iter().any(|run| run.text == "hello"));
        assert!(app.scene.ui.is_empty());
    }

    #[test]
    fn minimal_ui_keeps_identity_labels_chat_transport_and_poses_without_chrome() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.set_minimal_ui(true);
        assert!(app.scene.text.is_empty());
        assert!(app.scene.ui.is_empty());
        assert!(app.scene.status_lines.is_empty());
        assert_eq!(app.text_input_mode(), TextInputMode::Disabled);
        assert_eq!(app.pointer_mode(), PointerMode::LockedLook);

        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        enter_peer(&mut app, 9);
        app.receive_message(9, 1, &chat_payload("Visitor", "hello"));
        app.receive_unreliable_pose(9, 1, &pose_payload_at(Vec3::new(1.0, EYE_HEIGHT, 1.0)));
        app.apply_display_name("Renamed".to_owned());
        app.update(frame(1.0), &InputFrame::default());

        assert_eq!(chat_runs(&app).len(), 2); // Text plus a readability shadow.
        assert!(chat_runs(&app).iter().all(|run| run.text == "hello"));
        for name in ["Visitor", "Renamed", "Curious Otter"] {
            assert!(app.scene.text.iter().any(|run| run.text == name));
        }
        let name = app
            .scene
            .text
            .iter()
            .find(|run| run.text == "Visitor")
            .unwrap();
        let speech = chat_runs(&app)[1];
        assert_eq!(speech.position.x, name.position.x);
        assert!((name.position.y - speech.position.y - NAMEPLATE_FONT_SIZE - 6.0).abs() < 0.0001);
        assert!(app.scene.ui.is_empty());
        assert!(app.scene.status_lines.is_empty());
        assert!(
            app.scene
                .text
                .iter()
                .all(|run| run.size == NAMEPLATE_FONT_SIZE || run.text == "hello")
        );
        assert!(npcs::get_npc_interaction_context().is_none());
        assert_eq!(app.scene.meshes.len(), app.base_mesh_count);
        assert_eq!(app.scene.sprites.len(), 2 + app.npcs.visible_count());
        assert_eq!(app.remote_players[&9].display_name, "Visitor");
        assert_eq!(app.chat_history.len(), 1);
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        assert!(commands.iter().any(|command| matches!(
            command,
            RealtimeCommand::PublishReliable { payload, .. }
                if matches!(serde_json::from_str::<FirstPersonMessage>(payload).unwrap(),
                    FirstPersonMessage::Profile { display_name, .. } if display_name == "Renamed")
        )));
        assert!(
            commands.iter().any(|command| matches!(
                command,
                RealtimeCommand::PublishPositionedUnreliable { .. }
            ))
        );

        app.handle_realtime_event(RealtimeEvent::Disconnected {
            reason: "test".to_owned(),
        });
        assert!(app.scene.status_lines.is_empty());
        app.update(frame(2.0), &InputFrame::default());
        assert_identity_only_overlay(&app);
        assert!(!app.scene.text.iter().any(|run| run.text == "Visitor"));
        assert!(app.scene.text.iter().any(|run| run.text == "Renamed"));
    }

    #[test]
    fn minimal_ui_ignores_text_events_without_affecting_movement_or_look() {
        let mut full = FirstPersonLab::with_display_name("Ada");
        let mut minimal = FirstPersonLab::with_display_name("Ada");
        minimal.set_minimal_ui(true);
        let starting_eye = minimal.scene.camera.eye;
        let starting_target = minimal.scene.camera.target;
        let input = InputFrame {
            movement: Vec2::new(0.5, 1.0),
            look_delta: Vec2::new(12.0, -6.0),
            ..InputFrame::default()
        };
        full.update(frame(1.0), &input);
        minimal.update(
            frame(1.0),
            &InputFrame {
                text_events: vec![
                    TextInputEvent::Open,
                    TextInputEvent::Insert("/name Changed".to_owned()),
                    TextInputEvent::Submit,
                    TextInputEvent::Open,
                    TextInputEvent::Insert("hello".to_owned()),
                    TextInputEvent::Backspace,
                    TextInputEvent::Submit,
                    TextInputEvent::Cancel,
                ],
                ..input
            },
        );

        assert_eq!(minimal.scene.camera.eye, full.scene.camera.eye);
        assert_eq!(minimal.scene.camera.target, full.scene.camera.target);
        assert_ne!(minimal.scene.camera.eye, starting_eye);
        assert_ne!(minimal.scene.camera.target, starting_target);
        assert_eq!(minimal.pointer_mode(), PointerMode::LockedLook);
        assert_eq!(minimal.text_input_mode(), TextInputMode::Disabled);
        assert!(!minimal.chat_open);
        assert!(minimal.chat_draft.is_empty());
        assert!(minimal.chat_history.is_empty());
        assert!(minimal.pending_publishes.is_empty());
        assert_eq!(minimal.display_name, "Ada");
    }

    #[test]
    fn toggling_minimal_ui_clears_stale_overlay_and_text_focus() {
        let mut app = FirstPersonLab::new();
        app.update(
            frame(1.0),
            &InputFrame {
                text_events: vec![
                    TextInputEvent::Open,
                    TextInputEvent::Insert("draft".to_owned()),
                ],
                ..InputFrame::default()
            },
        );
        app.chat_feedback = Some("feedback".to_owned());
        app.rebuild_overlay();
        assert_eq!(app.text_input_mode(), TextInputMode::Focused);
        assert!(!app.scene.text.is_empty());
        assert!(!app.scene.ui.is_empty());

        app.set_minimal_ui(true);
        assert!(!app.chat_open);
        assert!(app.chat_draft.is_empty());
        assert!(app.chat_feedback.is_none());
        assert_identity_only_overlay(&app);
        assert!(
            app.scene
                .text
                .iter()
                .any(|run| run.text == app.display_name)
        );
        assert!(app.scene.text.iter().any(|run| run.text == "Curious Otter"));
        assert_eq!(app.text_input_mode(), TextInputMode::Disabled);
        assert_eq!(app.pointer_mode(), PointerMode::LockedLook);

        app.set_minimal_ui(false);
        assert_eq!(app.text_input_mode(), TextInputMode::Available);
        assert!(!app.scene.text.is_empty());
        assert!(!app.scene.status_lines.is_empty());
        assert!(app.scene.ui.is_empty());
        app.handle_text_input(&[TextInputEvent::Open]);
        assert_eq!(app.text_input_mode(), TextInputMode::Focused);
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
        let position = grounded_eye(Vec3::new(1.0, 0.0, -2.0));
        let golden = [
            1, 0, 0, 128, 63, 52, 51, 131, 64, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 191,
        ];
        assert_eq!(golden.len(), 25);
        assert_eq!(
            encode_pose(position, Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            golden
        );
        assert_eq!(
            decode_pose(&golden),
            Some(PlayerPose {
                seat: None,
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
        let position = grounded_eye(Vec3::new(1.0, 0.0, -2.0)).to_array();
        let forward = [0.0, 0.0, -1.0];
        assert!(decode_pose(&unchecked_pose_payload(position, forward)).is_some());
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
        let bounds = theater::Theater::camera_bounds();
        let radius = theater::PLAYER_RADIUS;
        for position in [
            [bounds.max.x + radius + 0.01, EYE_HEIGHT, 0.0],
            [-theater::HALF_WIDTH - 0.01, EYE_HEIGHT, 0.0],
            [bounds.min.x, EYE_HEIGHT, bounds.max.z],
            [
                bounds.min.x - radius - 0.01,
                waypoint("row_b").y,
                waypoint("row_b").z,
            ],
            [0.0, EYE_HEIGHT, theater::MAX_Z + 0.01],
            [0.0, EYE_HEIGHT, theater::MIN_Z - 0.01],
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
        let boundary = grounded_eye(Vec3::new(bounds.max.x + radius, 0.0, bounds.min.z - radius))
            + Vec3::Y * 0.25;
        let payload = encode_pose(boundary, Vec3::new(0.0, 0.0, -2.0)).unwrap();
        assert_eq!(decode_pose(&payload).unwrap().position, boundary);
        assert_eq!(decode_pose(&payload).unwrap().forward, -Vec3::Z);
    }

    #[test]
    fn pose_codec_height_tolerance_follows_lobby_corridor_and_each_viewing_tier() {
        let bounds = theater::Theater::camera_bounds();
        for eye in [
            waypoint("lobby_center"),
            waypoint("corridor_turn"),
            waypoint("auditorium_entry"),
            waypoint("row_b"),
            waypoint("auditorium_left"),
            grounded_eye(Vec3::new(bounds.min.x, 0.0, bounds.min.z)),
            grounded_eye(waypoint("passage_lobby").lerp(waypoint("corridor_turn"), 0.5)),
            grounded_eye(waypoint("row_a").lerp(waypoint("row_b"), 0.5)),
        ] {
            for offset in [-0.25, 0.0, 0.25] {
                let position = eye + Vec3::Y * offset;
                let payload = encode_pose(position, -Vec3::Z * 2.0).unwrap();
                let decoded = decode_pose(&payload).unwrap();
                assert_eq!(decoded.position, position);
                assert_eq!(decoded.forward, -Vec3::Z);
            }
            for offset in [-0.26, 0.26] {
                let position = eye + Vec3::Y * offset;
                assert!(encode_pose(position, -Vec3::Z).is_none());
                assert!(
                    decode_pose(&unchecked_pose_payload(
                        position.to_array(),
                        [0.0, 0.0, -1.0]
                    ))
                    .is_none()
                );
            }
        }
        let mut ungrounded = waypoint("auditorium_entry");
        ungrounded.y = EYE_HEIGHT;
        assert!(encode_pose(ungrounded, -Vec3::Z).is_none());
        let mut floating = waypoint("lobby_center");
        floating.y = waypoint("auditorium_entry").y;
        assert!(encode_pose(floating, -Vec3::Z).is_none());
    }

    #[test]
    fn expanded_layout_corners_still_publish_and_decode_positioned_poses() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        let bounds = theater::Theater::camera_bounds();
        for (index, position) in [
            grounded_eye(Vec3::new(bounds.min.x, 0.0, bounds.min.z)),
            grounded_eye(Vec3::new(bounds.min.x, 0.0, waypoint("auditorium_back").z)),
            grounded_eye(Vec3::new(
                -theater::HALF_WIDTH + theater::PLAYER_RADIUS,
                0.0,
                bounds.max.z,
            )),
            grounded_eye(Vec3::new(bounds.max.x, 0.0, bounds.min.z)),
            grounded_eye(Vec3::new(bounds.max.x, 0.0, bounds.max.z)),
            waypoint("auditorium_left"),
            waypoint("corridor_turn"),
        ]
        .into_iter()
        .enumerate()
        {
            app.view.player.eye = position;
            app.view.player.target = position + Vec3::NEG_Z;
            app.queue_local_pose(index as f64);
            let RealtimeCommand::PublishPositionedUnreliable {
                position: routed,
                payload,
                ..
            } = app.pending_publishes.back().unwrap()
            else {
                panic!("expanded theater position must publish");
            };
            assert_eq!(*routed, position.as_dvec3().to_array());
            assert_eq!(decode_pose(payload).unwrap().position, position);
        }
    }

    #[test]
    fn pose_publishing_is_10_hz_and_replaces_only_pending_pose() {
        let mut app = FirstPersonLab::with_display_name("Ada");
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.queue_local_pose(0.0);
        let first_sequence = app.next_sequence;
        app.queue_local_pose(0.099);
        assert_eq!(app.next_sequence, first_sequence);
        app.view.player.eye.x = 2.0;
        app.view.player.target.x = 2.0;
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
        assert_eq!(
            *position,
            [2.0, f64::from(EYE_HEIGHT), f64::from(theater::SPAWN.z)]
        );
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
            position: grounded_eye(Vec3::new(1.0, 0.0, -2.0)).to_array(),
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
