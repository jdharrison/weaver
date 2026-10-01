//! Shared first-person multiplayer room for desktop and browser shells.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use weaver_app_core::{
    AppAssets, CameraBounds, FirstPersonController, FrameContext, InputFrame, MeshAsset,
    PointerMode, RealtimeCommand, RealtimeEvent, WeaverApp,
};
use weaver_render::{Camera, MeshHandle, MeshInstance, Projection, RenderTransform, SceneSnapshot};
use weaver_render_wgpu::Vertex;

const ROOM_HALF_WIDTH: f32 = 6.0;
const ROOM_HALF_DEPTH: f32 = 8.0;
const ROOM_HEIGHT: f32 = 4.0;
const EYE_HEIGHT: f32 = 1.7;
const WALL_CLEARANCE: f32 = 0.3;
const PLAYER_BODY_OFFSET: f32 = 0.82;
const PLAYER_SCALE: f32 = 1.0;
const PLAYER_STATE_VERSION: u8 = 1;
const PUBLISH_INTERVAL_SECONDS: f64 = 0.1;
const MAX_REMOTE_PLAYERS: usize = 128;
const MAX_PENDING_PUBLISHES: usize = 4;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PlayerState {
    version: u8,
    position: [f32; 3],
    forward: [f32; 3],
}

impl PlayerState {
    fn from_camera(camera: &Camera) -> Self {
        let forward = (camera.target - camera.eye).normalize_or_zero();
        Self {
            version: PLAYER_STATE_VERSION,
            position: camera.eye.to_array(),
            forward: forward.to_array(),
        }
    }

    fn validated_pose(&self) -> Option<(Vec3, Vec3)> {
        let position = Vec3::from_array(self.position);
        let forward = Vec3::from_array(self.forward);
        let values_are_finite = position.is_finite() && forward.is_finite();
        let position_is_inside_room = position.x.abs() <= ROOM_HALF_WIDTH
            && position.z.abs() <= ROOM_HALF_DEPTH
            && (position.y - EYE_HEIGHT).abs() <= 0.25;
        if self.version != PLAYER_STATE_VERSION
            || !values_are_finite
            || !position_is_inside_room
            || forward.length_squared() < 0.001
        {
            return None;
        }
        Some((position, forward.normalize()))
    }
}

#[derive(Clone, Copy, Debug)]
struct RemotePlayer {
    displayed_position: Vec3,
    target_position: Vec3,
    forward: Vec3,
    sequence: u64,
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
    pending_publishes: VecDeque<RealtimeCommand>,
    next_sequence: u64,
    next_publish_at: f64,
    network_status: String,
}

impl FirstPersonLab {
    /// Create the enclosed room and local first-person camera.
    #[must_use]
    pub fn new() -> Self {
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
            pending_publishes: VecDeque::with_capacity(MAX_PENDING_PUBLISHES),
            next_sequence: 0,
            next_publish_at: 0.0,
            network_status: "offline · 0 remote".to_owned(),
        }
    }

    fn queue_local_pose(&mut self, elapsed_seconds: f64) {
        if self.local_entity_id.is_none() || elapsed_seconds < self.next_publish_at {
            return;
        }
        self.next_publish_at = elapsed_seconds + PUBLISH_INTERVAL_SECONDS;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let Ok(payload) = serde_json::to_string(&PlayerState::from_camera(&self.scene.camera))
        else {
            "online · local pose serialization failed".clone_into(&mut self.network_status);
            return;
        };
        if self.pending_publishes.len() == MAX_PENDING_PUBLISHES {
            self.pending_publishes.pop_front();
        }
        self.pending_publishes.push_back(RealtimeCommand::Publish {
            sequence: self.next_sequence,
            payload,
        });
    }

    fn receive_player(&mut self, entity_id: u64, sequence: u64, payload: &str) {
        if Some(entity_id) == self.local_entity_id {
            return;
        }
        let Ok(state) = serde_json::from_str::<PlayerState>(payload) else {
            return;
        };
        let Some((position, forward)) = state.validated_pose() else {
            return;
        };
        if let Some(player) = self.remote_players.get_mut(&entity_id) {
            if sequence <= player.sequence {
                return;
            }
            player.target_position = position;
            player.forward = forward;
            player.sequence = sequence;
        } else if self.remote_players.len() < MAX_REMOTE_PLAYERS {
            self.remote_players.insert(
                entity_id,
                RemotePlayer {
                    displayed_position: position,
                    target_position: position,
                    forward,
                    sequence,
                },
            );
        }
        self.refresh_status();
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

    fn refresh_status(&mut self) {
        let connection = self
            .local_entity_id
            .map_or("offline".to_owned(), |entity_id| {
                format!("online as entity {entity_id}")
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
        PointerMode::LockedLook
    }

    fn handle_realtime_event(&mut self, event: RealtimeEvent) {
        match event {
            RealtimeEvent::Connected { entity_id } => {
                self.local_entity_id = Some(entity_id);
                self.next_sequence = 0;
                self.next_publish_at = 0.0;
                self.refresh_status();
            }
            RealtimeEvent::EntityLeft { entity_id } => {
                self.remote_players.remove(&entity_id);
                self.refresh_status();
            }
            RealtimeEvent::Payload {
                entity_id,
                sequence,
                payload,
            } => self.receive_player(entity_id, sequence, &payload),
            RealtimeEvent::Disconnected { reason } => {
                self.local_entity_id = None;
                self.remote_players.clear();
                self.pending_publishes.clear();
                self.network_status = format!("offline · {reason}");
                self.scene.status_lines = vec![self.network_status.clone()];
            }
        }
    }

    fn drain_realtime_commands(&mut self, commands: &mut Vec<RealtimeCommand>) {
        commands.extend(self.pending_publishes.drain(..));
    }

    fn update(&mut self, frame: FrameContext, input: &InputFrame) {
        self.controller
            .update(&mut self.scene.camera, input, frame.delta_seconds);
        self.update_remote_players(frame.delta_seconds);
        self.queue_local_pose(frame.elapsed_seconds);
    }
}

/// Start First-Person Lab in its browser shell.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    std::panic::set_hook(Box::new(|panic| {
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&panic.to_string()));
    }));
    weaver_platform_web::run(
        Box::new(FirstPersonLab::new()),
        weaver_platform_web::WebConfig::default(),
    )
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
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

    #[test]
    fn peer_payloads_are_validated_ordered_and_rendered() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        let payload = serde_json::to_string(&PlayerState {
            version: PLAYER_STATE_VERSION,
            position: [1.0, EYE_HEIGHT, -2.0],
            forward: [0.0, 0.0, -1.0],
        })
        .unwrap();
        app.handle_realtime_event(RealtimeEvent::Payload {
            entity_id: 9,
            sequence: 2,
            payload: payload.clone(),
        });
        app.handle_realtime_event(RealtimeEvent::Payload {
            entity_id: 9,
            sequence: 1,
            payload,
        });
        assert_eq!(app.remote_players.len(), 1);
        assert_eq!(app.remote_players[&9].sequence, 2);
        app.update_remote_players(1.0 / 60.0);
        assert_eq!(app.scene.meshes.len(), app.base_mesh_count + 1);
    }

    #[test]
    fn connected_player_publishes_bounded_pose_updates() {
        let mut app = FirstPersonLab::new();
        app.handle_realtime_event(RealtimeEvent::Connected { entity_id: 7 });
        app.update(
            FrameContext {
                delta_seconds: 1.0 / 60.0,
                elapsed_seconds: 1.0,
            },
            &InputFrame::default(),
        );
        let mut commands = Vec::new();
        app.drain_realtime_commands(&mut commands);
        assert_eq!(commands.len(), 1);
        let RealtimeCommand::Publish { payload, .. } = &commands[0];
        assert!(serde_json::from_str::<PlayerState>(payload).is_ok());
    }
}
