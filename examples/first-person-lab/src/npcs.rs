//! Local, authored ambient portraits. No interaction, AI, or network entities.
//!
//! Three room residents guarantee physical occupancy; two non-overlapping door
//! visits add traffic. Schedules are compiled once and sampled directly from
//! accepted, nondecreasing UTC, never advanced by frames, movies, player actions,
//! or a catch-up scheduler. Invalid or backward UTC freezes the last safe poses,
//! including bob, until UTC catches up to the last accepted sample.
//!
//! Authored routes enter and exit at the door. After suspension or a large forward
//! clock jump, direct resynchronization can skip intermediate route positions and
//! door transitions: observed visibility changes across missing frames need not
//! occur at the door. Skipped positions are never replayed, but physical room
//! counts remain within 1–2 lobby / 2–3 auditorium at every sample and freeze.

use crate::{avatars, theater};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use weaver_render::{SceneSnapshot, SpriteHandle};

const NPC_COUNT: usize = 5;
const MAX_ACTIONS: usize = 32;
const MAX_SAFE_SECONDS: f64 = 9_007_199_254_740_991.0;
const AMBIENT_BOB_HEIGHT: f32 = 0.18;
const DEFINITIONS_JSON: &str = include_str!("../assets/npcs.json");

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NpcConfig {
    version: u8,
    schedule_period_seconds: f64,
    definitions: Vec<NpcDefinition>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NpcDefinition {
    id: String,
    display_name: String,
    avatar_id: String,
    profile: String,
    behavior: Behavior,
    interaction: Interaction,
    schedule: Schedule,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Behavior {
    Ambient,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Interaction {
    enabled: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Schedule {
    Resident {
        point: String,
    },
    DoorVisit {
        offset_seconds: f64,
        actions: Vec<Action>,
    },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Move { point: String, speed: f64 },
    Wait { seconds: f64 },
}

fn config() -> &'static NpcConfig {
    static CONFIG: OnceLock<NpcConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let config =
            serde_json::from_str(DEFINITIONS_JSON).expect("bundled ambient NPC catalog must parse");
        validate_config(&config).expect("bundled ambient NPC catalog must validate");
        config
    })
}

fn known_waypoint(point: &str) -> bool {
    matches!(
        point,
        "lobby_door"
            | "lobby_entry"
            | "lobby_center"
            | "lobby_west"
            | "lobby_east"
            | "lobby_aisle"
            | "passage_lobby"
            | "corridor_turn"
            | "auditorium_entry"
            | "auditorium_back"
            | "auditorium_left"
            | "auditorium_right"
            | "auditorium_center"
            | "row_a"
            | "row_b"
    ) && theater::npc_waypoint(point).is_some()
}

fn validate_config(config: &NpcConfig) -> Result<(), &'static str> {
    if config.version != 3
        || config.definitions.len() != NPC_COUNT
        || !config.schedule_period_seconds.is_finite()
        || !(60.0..=3600.0).contains(&config.schedule_period_seconds)
    {
        return Err("invalid ambient NPC catalog envelope");
    }
    for (index, definition) in config.definitions.iter().enumerate() {
        if definition.id != format!("guest_{:02}", index + 1)
            || definition.avatar_id != format!("avatar_{:02}", index + 1)
            || definition.profile != "npc_casual_guest"
            || definition.behavior != Behavior::Ambient
            || definition.interaction.enabled
            || crate::validate_display_name(&definition.display_name).as_ref()
                != Some(&definition.display_name)
        {
            return Err("catalog must contain only the five noninteractive ambient guests");
        }
        match (&definition.schedule, index) {
            (Schedule::Resident { point }, 0..=2) => {
                if !known_waypoint(point) {
                    return Err("unknown resident anchor");
                }
            }
            (
                Schedule::DoorVisit {
                    offset_seconds,
                    actions,
                },
                3..=4,
            ) => {
                if !offset_seconds.is_finite()
                    || !(0.0..config.schedule_period_seconds).contains(offset_seconds)
                    || !(2..=MAX_ACTIONS).contains(&actions.len())
                {
                    return Err("invalid bounded door visit");
                }
                for action in actions {
                    match action {
                        Action::Move { point, speed } => {
                            if !known_waypoint(point)
                                || !speed.is_finite()
                                || !(1.0..=1.6).contains(speed)
                            {
                                return Err("invalid authored walking leg");
                            }
                        }
                        Action::Wait { seconds } => {
                            if !seconds.is_finite() || !(0.5..=60.0).contains(seconds) {
                                return Err("invalid authored pause");
                            }
                        }
                    }
                }
                let points: Vec<_> = actions
                    .iter()
                    .filter_map(|action| match action {
                        Action::Move { point, .. } => Some(point.as_str()),
                        Action::Wait { .. } => None,
                    })
                    .collect();
                if points.last().copied() != Some("lobby_door")
                    || !points.windows(3).any(|points| {
                        points == ["passage_lobby", "corridor_turn", "auditorium_entry"]
                    })
                    || !points.windows(3).any(|points| {
                        points == ["auditorium_entry", "corridor_turn", "passage_lobby"]
                    })
                {
                    return Err("visitors must return to the door through both explicit elbows");
                }
            }
            _ => return Err("three residents and two door visitors are required"),
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Lobby,
    Auditorium,
    Corridor,
}

fn zone(position: Vec3) -> Zone {
    if theater::in_auditorium(position) {
        Zone::Auditorium
    } else if position.z > 0.0 {
        Zone::Lobby
    } else {
        // The threshold itself is not inside either room. Corridor travelers
        // are counted only where their rendered, grounded feet actually are.
        Zone::Corridor
    }
}

fn grounded_eye(mut position: Vec3) -> Vec3 {
    position.y = theater::floor_height(position) + crate::EYE_HEIGHT;
    position
}

fn waypoint(point: &str) -> Vec3 {
    grounded_eye(theater::npc_waypoint(point).expect("validated ambient waypoint"))
}

#[derive(Debug)]
struct Segment {
    start_seconds: f64,
    duration_seconds: f64,
    from: Vec3,
    to: Vec3,
}

#[derive(Debug)]
enum Slot {
    Resident(Vec3),
    DoorVisit {
        offset_seconds: f64,
        visible_seconds: f64,
        timeline: Vec<Segment>,
    },
}

fn compile_slot(
    schedule: &Schedule,
    colliders: &[theater::Collider],
) -> Result<Slot, &'static str> {
    match schedule {
        Schedule::Resident { point } => {
            let eye = waypoint(point);
            if !theater::npc_path_clear(eye, eye, colliders) {
                return Err("resident anchor intersects physical geometry");
            }
            Ok(Slot::Resident(eye))
        }
        Schedule::DoorVisit {
            offset_seconds,
            actions,
        } => {
            let mut position = waypoint("lobby_door");
            if !theater::npc_path_clear(position, position, colliders) {
                return Err("entrance door intersects physical geometry");
            }
            let mut start_seconds = 0.0;
            let mut timeline = Vec::with_capacity(actions.len());
            for action in actions {
                let (to, duration_seconds) = match action {
                    Action::Move { point, speed } => {
                        let to = waypoint(point);
                        if !theater::npc_path_clear(position, to, colliders) {
                            return Err("authored NPC leg intersects physical geometry");
                        }
                        let dx = f64::from(to.x) - f64::from(position.x);
                        let dz = f64::from(to.z) - f64::from(position.z);
                        // Constant horizontal walking speed; floor height is a
                        // surface query, not a straight 3D line between markers.
                        (to, dx.hypot(dz) / speed)
                    }
                    Action::Wait { seconds } => (position, *seconds),
                };
                if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
                    return Err("authored NPC legs must have positive finite durations");
                }
                timeline.push(Segment {
                    start_seconds,
                    duration_seconds,
                    from: position,
                    to,
                });
                start_seconds += duration_seconds;
                position = to;
            }
            Ok(Slot::DoorVisit {
                offset_seconds: *offset_seconds,
                visible_seconds: start_seconds,
                timeline,
            })
        }
    }
}

fn compile_slots(
    config: &NpcConfig,
    colliders: &[theater::Collider],
) -> Result<[Slot; NPC_COUNT], &'static str> {
    validate_config(config)?;
    let slots: Vec<_> = config
        .definitions
        .iter()
        .map(|definition| compile_slot(&definition.schedule, colliders))
        .collect::<Result<_, _>>()?;
    let slots: [Slot; NPC_COUNT] = slots.try_into().map_err(|_| "invalid slot count")?;
    let mut residents = (0, 0);
    for slot in &slots {
        match slot {
            Slot::Resident(eye) => match zone(*eye) {
                Zone::Lobby => residents.0 += 1,
                Zone::Auditorium => residents.1 += 1,
                Zone::Corridor => return Err("corridor anchors cannot satisfy room occupancy"),
            },
            Slot::DoorVisit {
                offset_seconds,
                visible_seconds,
                ..
            } => {
                if offset_seconds + visible_seconds > config.schedule_period_seconds {
                    return Err("door visit must fit its fixed period without accelerating");
                }
            }
        }
    }
    if residents != (1, 2) {
        return Err("exactly one lobby and two auditorium residents are required");
    }
    for (index, left) in slots.iter().enumerate() {
        for right in slots.iter().skip(index + 1) {
            if let (
                Slot::DoorVisit {
                    offset_seconds: left_start,
                    visible_seconds: left_duration,
                    ..
                },
                Slot::DoorVisit {
                    offset_seconds: right_start,
                    visible_seconds: right_duration,
                    ..
                },
            ) = (left, right)
                && *left_start < *right_start + *right_duration
                && *right_start < *left_start + *left_duration
            {
                return Err("door visibility windows must not overlap");
            }
        }
    }
    // Residents never leave their physical rooms. At most one other portrait
    // exists, so the maxima (2 lobby / 3 auditorium) hold even at thresholds.
    Ok(slots)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NpcPose {
    eye: Vec3,
    bob_height: f32,
}

fn bob_height(unix_seconds: f64, index: usize) -> f32 {
    // Five seconds is exactly nine bob cycles. Reduce the UTC input before
    // trigonometry so large time jumps never degrade the visible animation.
    let phase = unix_seconds.rem_euclid(5.0) + index as f64 * 0.17;
    let bob = (phase * std::f64::consts::TAU * 1.8).sin() as f32;
    (bob * 0.5 + 0.5) * AMBIENT_BOB_HEIGHT
}

fn pose_at(slot: &Slot, cycle_phase: f64, bob_height: f32) -> Option<NpcPose> {
    let eye = match slot {
        Slot::Resident(eye) => *eye,
        Slot::DoorVisit {
            offset_seconds,
            visible_seconds,
            timeline,
        } => {
            // Compilation forbids wrapped windows. Compare absolute cycle
            // boundaries before subtracting the offset, avoiding cancellation
            // that could render an extra frame at an exact departure boundary.
            if cycle_phase < *offset_seconds || cycle_phase >= offset_seconds + visible_seconds {
                return None;
            }
            let phase = cycle_phase - offset_seconds;
            let segment = timeline
                .iter()
                .find(|segment| phase < segment.start_seconds + segment.duration_seconds)
                .unwrap_or_else(|| timeline.last().expect("validated nonempty door route"));
            let blend =
                ((phase - segment.start_seconds) / segment.duration_seconds).clamp(0.0, 1.0) as f32;
            segment.from.lerp(segment.to, blend)
        }
    };
    // A single walking leg can cross several flats and ramps. Interpolating
    // endpoint heights would put portraits above or below the actual surface.
    Some(NpcPose {
        eye: grounded_eye(eye),
        bob_height,
    })
}

pub(crate) struct NpcRuntime {
    period_seconds: f64,
    slots: [Slot; NPC_COUNT],
    poses: [Option<NpcPose>; NPC_COUNT],
    last_accepted_unix_seconds: Option<f64>,
}

impl NpcRuntime {
    pub(crate) fn new(colliders: &[theater::Collider]) -> Self {
        let config = config();
        let slots = compile_slots(config, colliders)
            .expect("bundled ambient routes must preserve safe physical room occupancy");
        let poses =
            std::array::from_fn(|index| pose_at(&slots[index], 0.0, bob_height(0.0, index)));
        Self {
            period_seconds: config.schedule_period_seconds,
            slots,
            poses,
            last_accepted_unix_seconds: None,
        }
    }

    pub(crate) fn update(
        &mut self,
        unix_seconds: f64,
        _player_eye: Vec3,
        _now_ms: f64,
        _colliders: &[theater::Collider],
    ) {
        if !unix_seconds.is_finite()
            || !(0.0..=MAX_SAFE_SECONDS).contains(&unix_seconds)
            || self
                .last_accepted_unix_seconds
                .is_some_and(|last| unix_seconds < last)
        {
            // Rejected clocks cannot rewind across an arrival/departure or reset
            // the acceptance watermark. Before any valid sample, phase zero holds.
            return;
        }
        let cycle_phase = unix_seconds.rem_euclid(self.period_seconds);
        self.poses = std::array::from_fn(|index| {
            pose_at(
                &self.slots[index],
                cycle_phase,
                bob_height(unix_seconds, index),
            )
        });
        self.last_accepted_unix_seconds = Some(unix_seconds);
    }

    pub(crate) fn append_sprites(
        &self,
        scene: &mut SceneSnapshot,
        avatars: &[SpriteHandle; avatars::AVATAR_COUNT],
        lobby_light: f32,
        auditorium_light: f32,
    ) {
        for (index, pose) in self.poses.iter().enumerate() {
            let Some(pose) = pose else { continue };
            // Catalog validation fixes guest_01..05 to avatar_01..05.
            let mut sprite = crate::avatar_instance(avatars[index], pose.eye, scene.camera.eye);
            sprite.transform.translation.y += pose.bob_height;
            let light = if zone(pose.eye) == Zone::Auditorium {
                auditorium_light
            } else {
                lobby_light
            };
            sprite.tint = [light, light, light, 1.0];
            scene.sprites.push(sprite);
        }
    }

    /// Only live portraits supply names, at their current bobbed eye positions.
    pub(crate) fn nameplates(&self) -> impl Iterator<Item = (&str, Vec3)> {
        config()
            .definitions
            .iter()
            .zip(&self.poses)
            .filter_map(|(definition, pose)| {
                pose.map(|pose| {
                    (
                        definition.display_name.as_str(),
                        pose.eye + Vec3::Y * pose.bob_height,
                    )
                })
            })
    }

    #[cfg(test)]
    pub(crate) fn visible_count(&self) -> usize {
        self.poses.iter().flatten().count()
    }

    /// Counts rendered locations only; neither a destination nor a reservation
    /// nor a corridor traveler contributes to either room's occupancy.
    #[cfg(test)]
    pub(crate) fn room_counts(&self) -> (usize, usize) {
        self.poses
            .iter()
            .flatten()
            .fold((0, 0), |counts, pose| match zone(pose.eye) {
                Zone::Lobby => (counts.0 + 1, counts.1),
                Zone::Auditorium => (counts.0, counts.1 + 1),
                Zone::Corridor => counts,
            })
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Date, js_name = now)]
    fn browser_unix_millis() -> f64;
}

pub(crate) fn unix_seconds(elapsed_seconds: f64) -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = elapsed_seconds;
        browser_unix_millis() / 1000.0
    }
    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    {
        let _ = elapsed_seconds;
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(f64::NAN, |duration| duration.as_secs_f64())
    }
    #[cfg(all(not(target_arch = "wasm32"), test))]
    {
        elapsed_seconds
    }
}

// Preserve the browser export signatures without retaining a dialogue bridge,
// context parsing, callbacks, requests, subtitles, gestures, or mutable authority.
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn get_npc_interaction_context() -> Option<String> {
    None
}

#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn submit_npc_question(_npc_id: &str, _request_id: &str, _text: &str) -> bool {
    false
}

#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn present_npc_response(
    _npc_id: &str,
    _request_id: &str,
    _answer: &str,
    _gesture: &str,
) -> bool {
    false
}

#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn clear_npc_dialogue() {}

#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn set_npc_interaction_active(_active: bool) {}

/// Movies do not install or rephase the independent ambient UTC schedule.
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn set_npc_show_cycle(_anchor_unix_seconds: f64, _cycle_seconds: f64) -> bool {
    false
}

/// Inert catalog metadata only; these IDs are not network entities or AI profiles.
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn get_npc_definitions() -> String {
    serde_json::to_string(&config().definitions).expect("static ambient catalog serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> (NpcRuntime, Vec<theater::Collider>) {
        let colliders = theater::Theater::new().colliders;
        (NpcRuntime::new(&colliders), colliders)
    }

    fn sample(runtime: &mut NpcRuntime, time: f64, colliders: &[theater::Collider]) {
        runtime.update(time, theater::SPAWN, f64::NAN, colliders);
    }

    fn assert_occupancy(runtime: &NpcRuntime, time: f64) {
        let (lobby, auditorium) = runtime.room_counts();
        assert!((1..=2).contains(&lobby), "lobby={lobby} at UTC {time}");
        assert!(
            (2..=3).contains(&auditorium),
            "auditorium={auditorium} at UTC {time}"
        );
        let corridor = runtime
            .poses
            .iter()
            .flatten()
            .filter(|pose| zone(pose.eye) == Zone::Corridor)
            .count();
        assert_eq!(lobby + auditorium + corridor, runtime.visible_count());
        assert!((3..=4).contains(&runtime.visible_count()));
        assert!(runtime.poses[..3].iter().all(Option::is_some));
        assert!(runtime.poses[3..].iter().flatten().count() <= 1);
    }

    fn visit_windows(runtime: &NpcRuntime) -> Vec<(usize, f64, f64)> {
        runtime
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| match slot {
                Slot::DoorVisit {
                    offset_seconds,
                    visible_seconds,
                    ..
                } => Some((index, *offset_seconds, *visible_seconds)),
                Slot::Resident(_) => None,
            })
            .collect()
    }

    #[test]
    fn version_three_catalog_has_only_five_inert_ambient_slots() {
        assert_eq!(config().version, 3);
        assert_eq!(config().schedule_period_seconds, 600.0);
        let definitions: Vec<NpcDefinition> = serde_json::from_str(&get_npc_definitions()).unwrap();
        assert_eq!(definitions.len(), NPC_COUNT);
        for (index, definition) in definitions.iter().enumerate() {
            assert_eq!(definition.id, format!("guest_{:02}", index + 1));
            assert_eq!(definition.avatar_id, format!("avatar_{:02}", index + 1));
            assert_eq!(definition.behavior, Behavior::Ambient);
            assert_eq!(definition.profile, "npc_casual_guest");
            assert!(!definition.interaction.enabled);
            assert_eq!(
                avatars::index_from_avatar_id(&definition.avatar_id),
                Some(index)
            );
        }
        for removed in [
            "concession",
            "support",
            "player_like",
            "TALK",
            "queue_",
            "seed",
            "Jitter",
            "ShowCycle",
            "knowledgeStyle",
            "absence",
        ] {
            assert!(
                !DEFINITIONS_JSON.contains(removed),
                "legacy field {removed}"
            );
        }
    }

    #[test]
    fn catalog_rejects_legacy_roles_interaction_unbounded_or_invalid_scripts() {
        let mut bad = config().clone();
        bad.version = 2;
        assert!(validate_config(&bad).is_err());
        bad = config().clone();
        bad.definitions.pop();
        assert!(validate_config(&bad).is_err());
        bad = config().clone();
        bad.definitions[1].id = "guest_01".into();
        assert!(validate_config(&bad).is_err());
        bad = config().clone();
        bad.definitions[2].profile = "npc_engineer_guest".into();
        assert!(validate_config(&bad).is_err());
        bad = config().clone();
        bad.definitions[0].interaction.enabled = true;
        assert!(validate_config(&bad).is_err());
        for period in [f64::NAN, f64::INFINITY, -1.0, 0.0, 3601.0] {
            bad = config().clone();
            bad.schedule_period_seconds = period;
            assert!(validate_config(&bad).is_err());
        }
        for speed in [f64::NAN, f64::INFINITY, -1.0, 0.0, 4.0, 8.0] {
            bad = config().clone();
            let Schedule::DoorVisit { actions, .. } = &mut bad.definitions[3].schedule else {
                panic!("visitor slot");
            };
            actions[0] = Action::Move {
                point: "lobby_entry".into(),
                speed,
            };
            assert!(validate_config(&bad).is_err());
        }
        for point in ["concession_support", "queue_west", "missing"] {
            bad = config().clone();
            bad.definitions[0].schedule = Schedule::Resident {
                point: point.into(),
            };
            assert!(validate_config(&bad).is_err());
        }
        bad = config().clone();
        let Schedule::DoorVisit { actions, .. } = &mut bad.definitions[3].schedule else {
            panic!("visitor slot");
        };
        actions.extend((0..MAX_ACTIONS).map(|_| Action::Wait { seconds: 1.0 }));
        assert!(validate_config(&bad).is_err());
        for role in ["support", "player_like"] {
            let legacy = DEFINITIONS_JSON.replacen(
                "\"behavior\": \"ambient\"",
                &format!("\"behavior\": \"{role}\""),
                1,
            );
            assert!(serde_json::from_str::<NpcConfig>(&legacy).is_err());
        }
        let unknown = DEFINITIONS_JSON.replacen("\"version\": 3", "\"version\": 3, \"seed\": 1", 1);
        assert!(serde_json::from_str::<NpcConfig>(&unknown).is_err());
    }

    #[test]
    fn compiled_windows_require_real_room_anchors_and_never_overlap_or_accelerate() {
        let (_, colliders) = runtime();
        let mut bad = config().clone();
        bad.definitions[1].schedule = Schedule::Resident {
            point: "corridor_turn".into(),
        };
        assert_eq!(
            compile_slots(&bad, &colliders).unwrap_err(),
            "corridor anchors cannot satisfy room occupancy"
        );
        bad = config().clone();
        bad.definitions[1].schedule = Schedule::Resident {
            point: "lobby_west".into(),
        };
        assert!(compile_slots(&bad, &colliders).is_err());
        bad = config().clone();
        let Schedule::DoorVisit { offset_seconds, .. } = &mut bad.definitions[4].schedule else {
            panic!("visitor slot");
        };
        *offset_seconds = 0.0;
        assert_eq!(
            compile_slots(&bad, &colliders).unwrap_err(),
            "door visibility windows must not overlap"
        );
        bad = config().clone();
        bad.schedule_period_seconds = 60.0;
        let Schedule::DoorVisit { offset_seconds, .. } = &mut bad.definitions[4].schedule else {
            panic!("visitor slot");
        };
        *offset_seconds = 30.0;
        assert_eq!(
            compile_slots(&bad, &colliders).unwrap_err(),
            "door visit must fit its fixed period without accelerating"
        );
    }

    #[test]
    fn physical_zone_counts_hold_from_construction_through_every_schedule_phase() {
        let (mut runtime, colliders) = runtime();
        assert_eq!(runtime.room_counts(), (2, 2));
        assert_occupancy(&runtime, 0.0);
        let mut lobby_extremes = (usize::MAX, 0);
        let mut auditorium_extremes = (usize::MAX, 0);
        let mut saw_corridor = false;
        let mut saw_hidden = [false; 2];
        let textures = [SpriteHandle::new(); avatars::AVATAR_COUNT];
        for step in 0..=30_000 {
            let time = runtime.period_seconds * f64::from(step) / 30_000.0;
            sample(&mut runtime, time, &colliders);
            assert_occupancy(&runtime, time);
            let (lobby, auditorium) = runtime.room_counts();
            lobby_extremes = (lobby_extremes.0.min(lobby), lobby_extremes.1.max(lobby));
            auditorium_extremes = (
                auditorium_extremes.0.min(auditorium),
                auditorium_extremes.1.max(auditorium),
            );
            for (index, pose) in runtime.poses[3..].iter().enumerate() {
                saw_hidden[index] |= pose.is_none();
                if let Some(pose) = pose {
                    if zone(pose.eye) == Zone::Corridor {
                        saw_corridor = true;
                        // Neither minimum relies on the traveler being en route.
                        assert_eq!(runtime.room_counts(), (1, 2));
                    }
                }
            }
            if step % 100 == 0 {
                let mut scene = SceneSnapshot::default();
                runtime.append_sprites(&mut scene, &textures, 1.0, 0.18);
                assert_eq!(scene.sprites.len(), runtime.visible_count());
                assert!(scene.text.is_empty());
                assert!(scene.ui.is_empty());
            }
        }
        assert_eq!(lobby_extremes, (1, 2));
        assert_eq!(auditorium_extremes, (2, 3));
        assert!(saw_corridor);
        assert_eq!(saw_hidden, [true; 2]);
    }

    #[test]
    fn junction_boundaries_cycle_seams_and_arbitrary_first_frames_keep_room_counts() {
        let (mut runtime, colliders) = runtime();
        let mut boundaries = vec![0.0, runtime.period_seconds];
        for slot in &runtime.slots {
            if let Slot::DoorVisit {
                offset_seconds,
                visible_seconds,
                timeline,
            } = slot
            {
                boundaries.extend([*offset_seconds, offset_seconds + visible_seconds]);
                for segment in timeline {
                    boundaries.extend([
                        offset_seconds + segment.start_seconds,
                        offset_seconds + segment.start_seconds + segment.duration_seconds,
                    ]);
                }
            }
        }
        let mut times = Vec::new();
        for cycle in [0.0, 1.0, 10_000.0, 3_000_000.0] {
            for boundary in &boundaries {
                for delta in [-0.000_001, 0.0, 0.000_001] {
                    let time = cycle * runtime.period_seconds + boundary + delta;
                    if time >= 0.0 {
                        times.push(time);
                    }
                }
            }
        }
        times.sort_by(f64::total_cmp);
        times.dedup();
        for time in times {
            sample(&mut runtime, time, &colliders);
            assert_eq!(runtime.last_accepted_unix_seconds, Some(time));
            assert_occupancy(&runtime, time);
        }
        let mut runtime = NpcRuntime::new(&colliders);
        for time in [
            0.0,
            1.0 / 60.0,
            93.25,
            300.0,
            599.999,
            600.0,
            86_400.0,
            1_800_000_071.125,
            1_000_000_000_000.0,
            MAX_SAFE_SECONDS,
        ] {
            let mut fresh = NpcRuntime::new(&colliders);
            sample(&mut fresh, time, &colliders);
            sample(&mut runtime, time, &colliders);
            assert_occupancy(&fresh, time);
            assert_eq!(fresh.poses, runtime.poses, "first UTC sample {time}");
        }
    }

    #[test]
    fn authored_visitor_routes_enter_and_exit_at_the_door_and_are_continuous() {
        let (runtime, colliders) = runtime();
        let door = waypoint("lobby_door");
        for (index, offset, visible) in visit_windows(&runtime) {
            let mut runtime = NpcRuntime::new(&colliders);
            sample(&mut runtime, offset, &colliders);
            assert_eq!(runtime.poses[index].unwrap().eye, door);
            sample(&mut runtime, offset + 0.01, &colliders);
            assert!(runtime.poses[index].unwrap().eye.distance(door) > 0.0);
            sample(&mut runtime, offset + visible - 0.000_001, &colliders);
            assert_eq!(runtime.poses[index].unwrap().eye, door);
            sample(&mut runtime, offset + visible, &colliders);
            assert!(runtime.poses[index].is_none());
            sample(&mut runtime, offset + visible + 0.01, &colliders);
            assert!(runtime.poses[index].is_none());
            let next_arrival = offset + runtime.period_seconds;
            sample(&mut runtime, next_arrival, &colliders);
            assert_eq!(runtime.poses[index].unwrap().eye, door);
        }
        for slot in &runtime.slots {
            let Slot::DoorVisit {
                offset_seconds,
                timeline,
                ..
            } = slot
            else {
                continue;
            };
            assert_eq!(timeline.first().unwrap().from, door);
            assert_eq!(timeline.last().unwrap().to, door);
            for pair in timeline.windows(2) {
                assert_eq!(pair[0].to, pair[1].from);
                let time = offset_seconds + pair[1].start_seconds;
                let before = pose_at(slot, time - 0.000_01, 0.0).unwrap();
                let after = pose_at(slot, time + 0.000_01, 0.0).unwrap();
                assert!(before.eye.distance(after.eye) < 0.000_1);
            }
        }
    }

    #[test]
    fn both_visitors_use_the_explicit_corridor_elbow_in_both_directions() {
        let (runtime, _) = runtime();
        let passage = waypoint("passage_lobby");
        let turn = waypoint("corridor_turn");
        let entry = waypoint("auditorium_entry");
        for slot in &runtime.slots[3..] {
            let Slot::DoorVisit { timeline, .. } = slot else {
                panic!("visitor slot");
            };
            for (from, corner, to) in [(passage, turn, entry), (entry, turn, passage)] {
                assert!(timeline.windows(2).any(|pair| {
                    pair[0].from == from
                        && pair[0].to == corner
                        && pair[1].from == corner
                        && pair[1].to == to
                }));
            }
            assert!(!timeline.iter().any(|segment| {
                (segment.from == passage && segment.to == entry)
                    || (segment.from == entry && segment.to == passage)
            }));
        }
    }

    #[test]
    fn every_authored_leg_is_collider_safe_and_every_analytic_sample_is_surface_grounded() {
        let (runtime, colliders) = runtime();
        let mut nonlinear_height_error = 0.0_f32;
        let mut maximum_floor_height = 0.0_f32;
        for slot in &runtime.slots {
            match slot {
                Slot::Resident(eye) => {
                    assert!(theater::npc_path_clear(*eye, *eye, &colliders));
                    assert_eq!(eye.y, theater::floor_height(*eye) + crate::EYE_HEIGHT);
                }
                Slot::DoorVisit {
                    offset_seconds,
                    timeline,
                    ..
                } => {
                    for segment in timeline {
                        assert!(theater::npc_path_clear(
                            segment.from,
                            segment.to,
                            &colliders
                        ));
                        for step in 0..256 {
                            let blend = f64::from(step) / 256.0;
                            let time = offset_seconds
                                + segment.start_seconds
                                + segment.duration_seconds * blend;
                            let pose = pose_at(slot, time, 0.0).unwrap();
                            assert!(theater::npc_path_clear(pose.eye, pose.eye, &colliders));
                            let floor = theater::floor_height(pose.eye);
                            assert_eq!(pose.eye.y, floor + crate::EYE_HEIGHT);
                            maximum_floor_height = maximum_floor_height.max(floor);
                            let linear = segment.from.lerp(segment.to, blend as f32);
                            nonlinear_height_error =
                                nonlinear_height_error.max((pose.eye.y - linear.y).abs());
                        }
                    }
                }
            }
        }
        assert!(
            maximum_floor_height > 1.0,
            "visitors must reach the raised tiers"
        );
        assert!(
            nonlinear_height_error > 0.05,
            "test must exercise legs crossing ramps and flats, not just linear slopes"
        );
    }

    #[test]
    fn all_five_portraits_share_obvious_bob_and_room_lighting_without_talk_overlays() {
        let (mut runtime, colliders) = runtime();
        let textures = std::array::from_fn(|_| SpriteHandle::new());
        let mut minimum_lifts = [f32::INFINITY; NPC_COUNT];
        let mut maximum_lifts = [0.0_f32; NPC_COUNT];
        for step in 0..7200 {
            let time = f64::from(step) / 12.0;
            sample(&mut runtime, time, &colliders);
            let mut scene = SceneSnapshot::default();
            runtime.append_sprites(&mut scene, &textures, 0.7, 0.12);
            assert_eq!(scene.sprites.len(), runtime.visible_count());
            assert!(scene.text.is_empty());
            assert!(scene.ui.is_empty());
            let mut sprites = scene.sprites.iter();
            for (index, pose) in runtime.poses.iter().enumerate() {
                let Some(pose) = pose else { continue };
                let sprite = sprites.next().unwrap();
                let base = crate::avatar_instance(textures[index], pose.eye, scene.camera.eye);
                let lift = sprite.transform.translation.y - base.transform.translation.y;
                minimum_lifts[index] = minimum_lifts[index].min(lift);
                maximum_lifts[index] = maximum_lifts[index].max(lift);
                assert!((-0.000_001..=AMBIENT_BOB_HEIGHT + 0.000_001).contains(&lift));
                assert_eq!(sprite.sprite, textures[index]);
                assert_eq!(sprite.transform.rotation, base.transform.rotation);
                assert_eq!(sprite.transform.scale, base.transform.scale);
                let bottom = sprite.transform.translation.y - sprite.transform.scale * 0.5;
                assert!(bottom >= theater::floor_height(pose.eye) - 0.000_001);
                let light = if zone(pose.eye) == Zone::Auditorium {
                    0.12
                } else {
                    0.7
                };
                assert_eq!(sprite.tint, [light, light, light, 1.0]);
            }
        }
        assert!(minimum_lifts.iter().all(|lift| *lift < 0.01));
        assert!(maximum_lifts.iter().all(|lift| *lift > 0.17));
    }

    #[test]
    fn nameplates_follow_live_bobbed_portraits_and_exact_departure_boundaries() {
        let (mut runtime, colliders) = runtime();
        let textures = std::array::from_fn(|_| SpriteHandle::new());
        for (visitor, start, duration) in visit_windows(&runtime) {
            for time in [
                start,
                start + 0.125,
                start + duration - 0.001,
                start + duration,
            ] {
                sample(&mut runtime, time, &colliders);
                let mut scene = SceneSnapshot::default();
                runtime.append_sprites(&mut scene, &textures, 1.0, 1.0);
                let labels: Vec<_> = runtime.nameplates().collect();
                assert_eq!(labels.len(), scene.sprites.len());
                for ((name, eye), sprite) in labels.iter().zip(&scene.sprites) {
                    let index = textures
                        .iter()
                        .position(|texture| *texture == sprite.sprite)
                        .unwrap();
                    let pose = runtime.poses[index].unwrap();
                    assert_eq!(*name, config().definitions[index].display_name);
                    assert_eq!(*eye, pose.eye + Vec3::Y * pose.bob_height);
                    let expected_center =
                        *eye - Vec3::Y * (crate::EYE_HEIGHT - crate::PLAYER_BILLBOARD_HEIGHT * 0.5);
                    assert!(
                        sprite
                            .transform
                            .translation
                            .abs_diff_eq(expected_center, 0.000_001)
                    );
                }
                assert_eq!(
                    labels
                        .iter()
                        .any(|(name, _)| *name == config().definitions[visitor].display_name),
                    time < start + duration,
                );
                let frozen: Vec<_> = labels
                    .iter()
                    .map(|(name, eye)| ((*name).to_owned(), *eye))
                    .collect();
                sample(&mut runtime, f64::NAN, &colliders);
                assert_eq!(
                    runtime
                        .nameplates()
                        .map(|(name, eye)| (name.to_owned(), eye))
                        .collect::<Vec<_>>(),
                    frozen
                );
            }
        }
    }

    #[test]
    fn forward_utc_sampling_is_frame_cadence_independent_and_direct_after_large_jumps() {
        let (mut dense, colliders) = runtime();
        let mut sparse = NpcRuntime::new(&colliders);
        for time in [
            0.0,
            1.0 / 60.0,
            37.25,
            171.125,
            317.5,
            599.999,
            600.0,
            1_800_000_177.125,
            MAX_SAFE_SECONDS,
        ] {
            let previous = dense.last_accepted_unix_seconds.unwrap_or(0.0);
            for frame in 0..=240 {
                let sample_time = previous + (time - previous) * (f64::from(frame) / 240.0);
                sample(&mut dense, sample_time.min(time), &colliders);
            }
            sample(&mut dense, time, &colliders);
            sparse.update(time, Vec3::splat(f32::NAN), f64::NEG_INFINITY, &[]);
            let mut fresh = NpcRuntime::new(&colliders);
            fresh.update(time, waypoint("auditorium_back"), 123.0, &colliders);
            assert_eq!(dense.last_accepted_unix_seconds, Some(time));
            assert_eq!(sparse.last_accepted_unix_seconds, Some(time));
            assert_eq!(dense.poses, sparse.poses, "cadence at UTC {time}");
            assert_eq!(
                dense.poses, fresh.poses,
                "no first-frame epoch at UTC {time}"
            );
            assert_occupancy(&dense, time);
        }
    }

    #[test]
    fn no_op_movie_and_player_hooks_cannot_rephase_or_pause_ambient_traffic() {
        let (mut runtime, colliders) = runtime();
        let time = 1_800_000_093.125;
        sample(&mut runtime, time, &colliders);
        let expected = runtime.poses;
        for (anchor, cycle) in [
            (0.0, 3600.0),
            (1_799_999_999.0, 600.0),
            (MAX_SAFE_SECONDS, 1.0),
            (f64::NAN, f64::INFINITY),
            (-1.0, 0.0),
        ] {
            assert!(!set_npc_show_cycle(anchor, cycle));
            clear_npc_dialogue();
            set_npc_interaction_active(true);
            runtime.update(time, waypoint("lobby_door"), anchor, &colliders);
            assert_eq!(runtime.poses, expected);
            set_npc_interaction_active(false);
            runtime.update(time, Vec3::splat(f32::INFINITY), cycle, &[]);
            assert_eq!(runtime.poses, expected);
            assert!(get_npc_interaction_context().is_none());
        }
        sample(&mut runtime, time + 10.0, &colliders);
        assert_ne!(runtime.poses, expected);
    }

    #[test]
    fn forged_or_legacy_interaction_contexts_never_authorize_questions_or_answers() {
        let (mut runtime, colliders) = runtime();
        sample(&mut runtime, 125.0, &colliders);
        let expected = runtime.poses;
        let forged = r#"{"version":1,"npc_id":"guest_02","profile":"npc_projectionist","scene_generation":"forged","frame_sequence":"1","distance_squared":0,"radius_world_units":100,"room":"lobby","show_state":"showing","nearby_project":null,"player_has_seen_intro":true}"#;
        set_npc_interaction_active(true);
        for id in [
            "guest_01",
            "guest_02",
            "guest_03",
            "guest_04",
            "guest_05",
            "concession_support",
            "",
            forged,
        ] {
            assert!(!submit_npc_question(id, "forged-request", "hello"));
            for gesture in ["idle", "welcome", "thinking", "point_left", "unknown"] {
                assert!(!present_npc_response(id, "forged-request", forged, gesture));
            }
        }
        for _ in 0..1024 {
            assert!(!submit_npc_question("guest_02", "replay", forged));
            assert!(!present_npc_response(
                "guest_02", "replay", "answer", "explain"
            ));
            clear_npc_dialogue();
        }
        assert!(get_npc_interaction_context().is_none());
        sample(&mut runtime, 125.0, &colliders);
        assert_eq!(runtime.poses, expected);
        drop(runtime);
        assert!(get_npc_interaction_context().is_none());
        assert!(!submit_npc_question("guest_01", "after-drop", "hello"));
    }

    #[test]
    fn invalid_or_backward_utc_freezes_safe_poses_without_using_now_ms() {
        let (mut runtime, colliders) = runtime();
        let initial = runtime.poses;
        let invalid = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            MAX_SAFE_SECONDS + 2.0,
        ];
        for time in invalid {
            sample(&mut runtime, time, &colliders);
            assert_eq!(runtime.poses, initial);
            assert_eq!(runtime.last_accepted_unix_seconds, None);
            assert_occupancy(&runtime, time);
        }
        sample(&mut runtime, 83.125, &colliders);
        let last_safe = runtime.poses;
        for time in invalid {
            runtime.update(time, Vec3::ZERO, 1_000_000.0, &colliders);
            assert_eq!(runtime.poses, last_safe);
            assert_eq!(runtime.last_accepted_unix_seconds, Some(83.125));
            assert_occupancy(&runtime, time);
        }
        for now_ms in [f64::NAN, f64::INFINITY, -1.0, 0.0, 999_999.0] {
            runtime.update(84.125, Vec3::ZERO, now_ms, &colliders);
            let mut fresh = NpcRuntime::new(&colliders);
            sample(&mut fresh, 84.125, &colliders);
            assert_eq!(runtime.poses, fresh.poses);
        }
        let last_safe = runtime.poses;
        sample(&mut runtime, 0.0, &colliders);
        assert_eq!(runtime.poses, last_safe);
        assert_eq!(runtime.last_accepted_unix_seconds, Some(84.125));
        assert_occupancy(&runtime, 0.0);
        sample(&mut runtime, 84.125, &colliders);
        assert_eq!(runtime.poses, last_safe);
        sample(&mut runtime, 84.25, &colliders);
        let mut fresh = NpcRuntime::new(&colliders);
        sample(&mut fresh, 84.25, &colliders);
        assert_eq!(runtime.poses, fresh.poses);
    }

    #[test]
    fn rewinds_across_arrival_departure_and_cycle_boundaries_freeze_until_utc_catches_up() {
        let (layout, colliders) = runtime();
        let door = waypoint("lobby_door");
        for (index, offset, visible) in visit_windows(&layout) {
            let mut runtime = NpcRuntime::new(&colliders);
            let base = if offset == 0.0 {
                runtime.period_seconds
            } else {
                0.0
            };
            let arrival = base + offset + 0.5;
            sample(&mut runtime, arrival, &colliders);
            let arrived = runtime.poses;
            assert!(arrived[index].unwrap().eye.distance(door) > 0.0);
            // Includes the reported guest_05 sequence 300.5 -> 299.5 -> 300.5.
            for rejected in [
                arrival - 1.0,
                arrival - 0.1,
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ] {
                sample(&mut runtime, rejected, &colliders);
                assert_eq!(runtime.poses, arrived, "arrival rewind {rejected}");
                assert_eq!(runtime.last_accepted_unix_seconds, Some(arrival));
                assert_occupancy(&runtime, rejected);
            }
            sample(&mut runtime, arrival, &colliders);
            assert_eq!(runtime.poses, arrived);
            sample(&mut runtime, arrival + 0.1, &colliders);
            assert!(
                runtime.poses[index]
                    .unwrap()
                    .eye
                    .distance(arrived[index].unwrap().eye)
                    > 0.0
            );
            let mut fresh = NpcRuntime::new(&colliders);
            sample(&mut fresh, arrival + 0.1, &colliders);
            assert_eq!(runtime.poses, fresh.poses);
            assert_occupancy(&runtime, arrival + 0.1);

            sample(&mut runtime, base + offset + visible - 0.5, &colliders);
            assert_eq!(runtime.poses[index].unwrap().eye, door);
            let departure = base + offset + visible + 0.000_001;
            sample(&mut runtime, departure, &colliders);
            let departed = runtime.poses;
            assert!(departed[index].is_none());
            assert_occupancy(&runtime, departure);
            sample(&mut runtime, departure - 0.5, &colliders);
            assert_eq!(
                runtime.poses, departed,
                "rewind must not revive a departed visitor"
            );
            assert_eq!(runtime.last_accepted_unix_seconds, Some(departure));
            assert_occupancy(&runtime, departure - 0.5);
            sample(&mut runtime, departure, &colliders);
            assert_eq!(runtime.poses, departed);

            let next_arrival = arrival + runtime.period_seconds;
            sample(&mut runtime, next_arrival, &colliders);
            let reentered = runtime.poses;
            assert!(reentered[index].unwrap().eye.distance(door) > 0.0);
            assert_occupancy(&runtime, next_arrival);
            sample(&mut runtime, next_arrival - 1.0, &colliders);
            assert_eq!(
                runtime.poses, reentered,
                "cycle rewind must not hide a visitor"
            );
            assert_eq!(runtime.last_accepted_unix_seconds, Some(next_arrival));
            assert_occupancy(&runtime, next_arrival - 1.0);
            sample(&mut runtime, next_arrival, &colliders);
            assert_eq!(runtime.poses, reentered);
        }
    }

    #[test]
    fn compiled_storage_is_fixed_and_forward_skips_resynchronize_without_replaying_routes() {
        let (mut runtime, colliders) = runtime();
        let storage = |runtime: &NpcRuntime| {
            runtime
                .slots
                .iter()
                .filter_map(|slot| match slot {
                    Slot::DoorVisit { timeline, .. } => {
                        assert!(timeline.len() <= MAX_ACTIONS);
                        Some((timeline.as_ptr(), timeline.len(), timeline.capacity()))
                    }
                    Slot::Resident(_) => None,
                })
                .collect::<Vec<_>>()
        };
        let original = storage(&runtime);
        for time in [0.0, 60.0, 1_800_000_000.0, MAX_SAFE_SECONDS, 1.0] {
            sample(&mut runtime, time, &colliders);
            assert_eq!(storage(&runtime), original);
            assert_eq!(runtime.slots.len(), NPC_COUNT);
            assert_eq!(runtime.poses.len(), NPC_COUNT);
            assert_occupancy(&runtime, time);
        }
        assert_eq!(runtime.last_accepted_unix_seconds, Some(MAX_SAFE_SECONDS));
        let door = waypoint("lobby_door");
        for (index, offset, visible) in visit_windows(&runtime) {
            let Slot::DoorVisit { timeline, .. } = &runtime.slots[index] else {
                panic!("visitor slot");
            };
            let pause = timeline
                .iter()
                .find(|segment| segment.from == segment.to && segment.from != door)
                .unwrap();
            let interior_time = offset + pause.start_seconds + pause.duration_seconds * 0.5;
            let mut resumed = NpcRuntime::new(&colliders);
            let original = storage(&resumed);
            sample(&mut resumed, interior_time, &colliders);
            assert_ne!(resumed.poses[index].unwrap().eye, door);
            assert_occupancy(&resumed, interior_time);

            // Missing frames can omit the departure at the door: direct phase
            // resynchronization may hide an actor last observed in the interior.
            sample(&mut resumed, offset + visible, &colliders);
            assert!(resumed.poses[index].is_none());
            assert_occupancy(&resumed, offset + visible);
            let forward_jump = interior_time + resumed.period_seconds * 1_000_000.0;
            sample(&mut resumed, forward_jump, &colliders);
            // The next observed entry can likewise already be inside. No queued
            // positions or door animations are replayed after suspension.
            assert_ne!(resumed.poses[index].unwrap().eye, door);
            let mut fresh = NpcRuntime::new(&colliders);
            sample(&mut fresh, forward_jump, &colliders);
            assert_eq!(resumed.poses, fresh.poses);
            assert_eq!(resumed.last_accepted_unix_seconds, Some(forward_jump));
            assert_eq!(storage(&resumed), original);
            assert_occupancy(&resumed, forward_jump);
        }
    }
}
