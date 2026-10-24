//! Local world meshes and a synchronous, permission-neutral browser picking seam.

use crate::theater::{self, Collider};
use glam::Vec3;
use glam::{UVec2, Vec2};
use std::{cell::RefCell, rc::Rc};
use weaver_app_core::{AppAssets, MeshAsset, TextureAsset};
use weaver_render::Camera;
use weaver_render::{MeshHandle, MeshInstance, MeshLighting, RenderTransform, SpriteHandle};
use weaver_render_wgpu::Vertex;

const STATE_COUNT: usize = 7;
const FORCED_ON: usize = 6;
const MAX_RANGE: f32 = 10.0;
const MAX_AGE_MS: f64 = 250.0;
const WIDTH: f32 = 2.6;
const HEIGHT: f32 = 0.9;
const CENTERS: [Vec3; 3] = [
    Vec3::new(-1.5, 1.7, 0.04),
    Vec3::new(1.5, 1.7, 0.04),
    Vec3::new(0.0, 0.65, 0.04),
];
struct Bridge {
    disposed: bool,
    world_panels: bool,
    states: [usize; 3],
    snapshot: Option<Snapshot>,
    player_eye: Vec3,
    seated: Option<u8>,
    occupied: [bool; theater::SEAT_COUNT],
    last_pick: Option<(Vec2, u32)>,
    pending: Option<u32>,
}

impl Default for Bridge {
    fn default() -> Self {
        Self {
            disposed: false,
            world_panels: true,
            states: [0, 0, 3],
            snapshot: None,
            player_eye: Vec3::NAN,
            seated: None,
            occupied: [false; theater::SEAT_COUNT],
            last_pick: None,
            pending: None,
        }
    }
}

struct Snapshot {
    camera: Camera,
    viewport: UVec2,
    colliders: Vec<Collider>,
    published_ms: f64,
    enabled: bool,
}

std::thread_local! {
    static BRIDGE: RefCell<Rc<RefCell<Bridge>>> = RefCell::new(Rc::new(RefCell::new(Bridge::default())));
}

/// Pick a world control synchronously. NDC is -1..=1, +Y upward; (0,0) is center.
/// Returns 0 for no enabled hit, 1 for sound, 2 for fullscreen, or 3 for subtitles.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pick_control_panel(x: f32, y: f32) -> u32 {
    let Some(window) = web_sys::window() else {
        return 0;
    };
    if window.document().is_none_or(|document| document.hidden()) {
        return 0;
    }
    let Some(now) = window.performance().map(|performance| performance.now()) else {
        return 0;
    };
    BRIDGE.with(|slot| slot.borrow().borrow().pick(Vec2::new(x, y), now))
}

/// Publish browser-observed state: panel 1 sound, 2 fullscreen, 3 subtitles;
/// state 0 unavailable, 1 off, 2 pending, 3 on, 4 blocked/retry, 5 loading,
/// 6 forced on (subtitles only, not pickable). Subtitles default to on.
/// Sound off also forces the subtitle display on without overwriting its state.
/// The host owns subtitle HUD visibility. Invalid or disposed updates return false.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_control_panel_state(panel: u32, state: u32) -> bool {
    BRIDGE.with(|slot| slot.borrow().borrow_mut().set_state(panel, state))
}

/// Retire picking immediately on host shutdown/fatal; only a new app revives it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn invalidate_control_panels() {
    BRIDGE.with(|slot| slot.borrow().borrow_mut().invalidate());
}

/// Update physical eligibility separately from the displayed (possibly zoomed) camera.
pub(crate) fn set_seating_context(
    player_eye: Vec3,
    seated: Option<u8>,
    occupied: [bool; theater::SEAT_COUNT],
) {
    BRIDGE.with(|slot| {
        let binding = slot.borrow();
        let mut bridge = binding.borrow_mut();
        bridge.player_eye = player_eye;
        bridge.seated = seated.filter(|index| usize::from(*index) < theater::SEAT_COUNT);
        bridge.occupied = occupied;
    });
}

pub(crate) fn take_seat_request() -> Option<u32> {
    BRIDGE.with(|slot| slot.borrow().borrow_mut().pending.take())
}

pub(crate) fn pick_current_interaction(ndc: Vec2, now_ms: f64) -> u32 {
    BRIDGE.with(|slot| {
        let binding = slot.borrow();
        let mut bridge = binding.borrow_mut();
        let target = bridge.pick_scene(ndc, now_ms);
        bridge.last_pick = Some((ndc, target));
        target
    })
}

pub(crate) fn queue_seat_request(target: u32, now_ms: f64) -> bool {
    BRIDGE.with(|slot| slot.borrow().borrow_mut().queue(target, now_ms))
}

#[cfg(target_arch = "wasm32")]
fn browser_now() -> Option<f64> {
    let window = web_sys::window()?;
    if window.document()?.hidden() {
        return None;
    }
    Some(window.performance()?.now())
}

/// Scene targets: 1–3 legacy controls, 4 GitHub, 5 LinkedIn, 100+ seat, 200 exit.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pick_scene_interaction(x: f32, y: f32) -> u32 {
    browser_now().map_or(0, |now| pick_current_interaction(Vec2::new(x, y), now))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn activate_seat_interaction(target: u32) -> bool {
    browser_now().is_some_and(|now| queue_seat_request(target, now))
}

/// Suspend interaction without retiring the app; the next publish can resume it.
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn invalidate_scene_interactions() {
    BRIDGE.with(|slot| slot.borrow().borrow_mut().clear_interactions());
}

impl Bridge {
    fn clear_interactions(&mut self) {
        self.snapshot = None;
        self.last_pick = None;
        self.pending = None;
    }

    fn fresh_snapshot(&self, now: f64) -> Option<&Snapshot> {
        let snapshot = self.snapshot.as_ref()?;
        let age = now - snapshot.published_ms;
        (!self.disposed && snapshot.enabled && age.is_finite() && (0.0..=MAX_AGE_MS).contains(&age))
            .then_some(snapshot)
    }

    fn pick_scene(&self, ndc: Vec2, now: f64) -> u32 {
        let Some(snapshot) = self.fresh_snapshot(now) else {
            return 0;
        };
        let Some((origin, direction)) = snapshot.ray(ndc) else {
            return 0;
        };
        // Even unavailable controls own their hit, so a click cannot exit a seat.
        let panel = self.pick_panel(ndc, now, true);
        if panel != 0 {
            return panel;
        }
        if let Some(target) = Self::pick_portfolio(snapshot, origin, direction) {
            return target;
        }
        if self.seated.is_some() {
            return 200;
        }
        let nearest = theater::SEATS
            .iter()
            .enumerate()
            .filter_map(|(index, seat)| {
                let (min, max) = seat.bounds();
                ray_box(origin, direction, min, max).map(|distance| (index, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((index, distance)) = nearest else {
            return 0;
        };
        let seat = theater::SEATS[index];
        // Stop before the envelope: testing the hit itself would occlude every chair.
        let hit = origin + direction * (distance - 0.001).max(0.0);
        if self.occupied[index]
            || !self.player_eye.is_finite()
            || self.player_eye.distance(seat.seated_eye()) > 2.5
            || !theater::has_line_of_sight(origin, hit, &snapshot.colliders)
        {
            return 0;
        }
        100 + index as u32
    }

    fn pick_portfolio(snapshot: &Snapshot, origin: Vec3, direction: Vec3) -> Option<u32> {
        use theater::decor::{
            PORTFOLIO_BOTTOM, PORTFOLIO_CENTER, PORTFOLIO_HALF_WIDTH, PORTFOLIO_TOP,
        };
        if origin.z <= PORTFOLIO_CENTER.z || direction.z >= -f32::EPSILON {
            return None;
        }
        let distance = (PORTFOLIO_CENTER.z - origin.z) / direction.z;
        if !(0.0..=MAX_RANGE).contains(&distance) {
            return None;
        }
        let hit = origin + direction * distance;
        if (hit.x - PORTFOLIO_CENTER.x).abs() > PORTFOLIO_HALF_WIDTH
            || !(PORTFOLIO_BOTTOM..=PORTFOLIO_TOP).contains(&hit.y)
            || !theater::has_line_of_sight(origin, hit, &snapshot.colliders)
        {
            return None;
        }
        Some(if hit.x < PORTFOLIO_CENTER.x { 4 } else { 5 })
    }

    fn queue(&mut self, target: u32, now: f64) -> bool {
        if !(100..100 + theater::SEAT_COUNT as u32).contains(&target) && target != 200 {
            return false;
        }
        // Direct native callers use the center ray. Once picked, only that exact
        // target/ray is eligible, including after a new frame has been published.
        let (ndc, picked) = self.last_pick.unwrap_or((Vec2::ZERO, target));
        if picked != target || self.pick_scene(ndc, now) != target {
            return false;
        }
        if let Some(pending) = self.pending {
            return pending == target;
        }
        // Keep click intent through camera movement until the frame consumes it;
        // the app rechecks current physical range and occupancy at consumption.
        self.pending = Some(target);
        true
    }

    fn display_state(&self, panel: usize) -> usize {
        // Keep the subtitle preference intact while muted; the host may also
        // explicitly publish forced-on for other audio-unavailable conditions.
        if panel == 2 && self.states[0] == 1 {
            FORCED_ON
        } else {
            self.states[panel]
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn set_state(&mut self, panel: u32, state: u32) -> bool {
        if self.disposed
            || !(1..=3).contains(&panel)
            || state >= STATE_COUNT as u32
            || (state == FORCED_ON as u32 && panel != 3)
        {
            return false;
        }
        self.states[(panel - 1) as usize] = state as usize;
        true
    }

    fn invalidate(&mut self) {
        self.disposed = true;
        {
            self.clear_interactions();
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn pick(&self, ndc: Vec2, now: f64) -> u32 {
        self.pick_panel(ndc, now, false)
    }

    fn pick_panel(&self, ndc: Vec2, now: f64, include_unavailable: bool) -> u32 {
        if !self.world_panels {
            return 0;
        }
        let Some(snapshot) = &self.snapshot else {
            return 0;
        };
        let age = now - snapshot.published_ms;
        if self.disposed
            || !snapshot.enabled
            || !age.is_finite()
            || !(0.0..=MAX_AGE_MS).contains(&age)
        {
            return 0;
        }
        let Some((origin, direction)) = snapshot.ray(ndc) else {
            return 0;
        };
        CENTERS
            .iter()
            .enumerate()
            .find_map(|(index, center)| {
                // Sound may reopen its startup/retry UI while a clip is loading.
                // Fullscreen still ignores a second activation during its request.
                let state = self.display_state(index);
                if (!include_unavailable
                    && (state == 0 || state == FORCED_ON || (index != 0 && state == 2)))
                    || direction.z >= -f32::EPSILON
                {
                    return None;
                }
                let distance = (center.z - origin.z) / direction.z;
                if !(0.0..=MAX_RANGE).contains(&distance) {
                    return None;
                }
                let hit = origin + direction * distance;
                if (hit.x - center.x).abs() > WIDTH * 0.5
                    || (hit.y - center.y).abs() > HEIGHT * 0.5
                    || !theater::has_line_of_sight(origin, hit, &snapshot.colliders)
                {
                    return None;
                }
                Some(index as u32 + 1)
            })
            .unwrap_or(0)
    }
}

fn ray_box(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis].abs() <= f32::EPSILON {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - origin[axis]) / direction[axis];
            let b = (max[axis] - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    Some(near)
}

impl Snapshot {
    fn ray(&self, ndc: Vec2) -> Option<(Vec3, Vec3)> {
        if !ndc.is_finite()
            || ndc.abs().max_element() > 1.0
            || self.viewport.min_element() == 0
            || !self.camera.eye.is_finite()
            || !self.camera.target.is_finite()
            || !self.camera.up.is_finite()
            || (self.camera.target - self.camera.eye)
                .cross(self.camera.up)
                .length_squared()
                <= f32::EPSILON
        {
            return None;
        }
        let inverse = self
            .camera
            .view_projection(self.viewport.x as f32 / self.viewport.y as f32)
            .inverse();
        if !inverse.is_finite() {
            return None;
        }
        let near = inverse.project_point3(ndc.extend(0.0));
        let far = inverse.project_point3(ndc.extend(1.0));
        let direction = (far - near).normalize_or_zero();
        if !near.is_finite() || !direction.is_finite() || direction == Vec3::ZERO {
            return None;
        }
        // Perspective rays start at the displayed eye, not at the near plane.
        let origin = match self.camera.projection {
            weaver_render::Projection::Perspective { .. } => self.camera.eye,
            weaver_render::Projection::Orthographic { .. } => near,
        };
        Some((origin, direction))
    }
}

pub(crate) struct ControlPanels {
    bridge: Rc<RefCell<Bridge>>,
    world: Option<WorldPanels>,
}

struct WorldPanels {
    mesh_indices: [usize; 3],
    textures: [[SpriteHandle; STATE_COUNT]; 3],
}

impl ControlPanels {
    pub(crate) fn new(
        assets: &mut AppAssets,
        meshes: &mut Vec<MeshInstance>,
        world_panels: bool,
    ) -> Self {
        let bridge = Rc::new(RefCell::new(Bridge {
            world_panels,
            ..Bridge::default()
        }));
        BRIDGE.with(|slot| *slot.borrow_mut() = Rc::clone(&bridge));
        let world = world_panels.then(|| {
            let textures = std::array::from_fn(|panel| {
                std::array::from_fn(|state| {
                    let texture = label_texture(panel, state);
                    let handle = texture.handle;
                    assets.textures.push(texture);
                    handle
                })
            });
            let mesh_indices = std::array::from_fn(|panel| {
                let center = CENTERS[panel];
                let offsets = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
                let vertices = offsets
                    .into_iter()
                    .map(|(x, y)| Vertex {
                        position: (center + Vec3::new(x * WIDTH, y * HEIGHT, 0.0)).to_array(),
                        normal: Vec3::Z.to_array(),
                        color: [1.0; 4],
                        uv: [x + 0.5, 0.5 - y],
                    })
                    .collect();
                let handle = MeshHandle::new();
                assets
                    .meshes
                    .push(MeshAsset::new(handle, vertices, vec![0, 1, 2, 0, 2, 3]));
                let mut mesh = MeshInstance::new(handle, RenderTransform::default());
                mesh.texture = Some(textures[panel][bridge.borrow().display_state(panel)]);
                mesh.lighting = Some(MeshLighting {
                    position: Vec3::ZERO,
                    ambient: 1.0,
                    intensity: 0.0,
                });
                let index = meshes.len();
                meshes.push(mesh);
                index
            });
            WorldPanels {
                mesh_indices,
                textures,
            }
        });
        Self { bridge, world }
    }

    pub(crate) fn refresh_meshes(&self, meshes: &mut [MeshInstance]) {
        let Some(world) = &self.world else {
            return;
        };
        let bridge = self.bridge.borrow();
        for (panel, index) in world.mesh_indices.iter().enumerate() {
            meshes[*index].texture = Some(world.textures[panel][bridge.display_state(panel)]);
            // Room lighting changes must not darken the control's own display.
            meshes[*index].lighting = Some(MeshLighting {
                position: Vec3::ZERO,
                ambient: 1.0,
                intensity: 0.0,
            });
        }
    }

    pub(crate) fn publish(
        &self,
        camera: &Camera,
        viewport: UVec2,
        colliders: &[Collider],
        now: f64,
        enabled: bool,
    ) {
        let mut bridge = self.bridge.borrow_mut();
        if !enabled {
            bridge.clear_interactions();
        }
        if !bridge.disposed {
            bridge.snapshot = Some(Snapshot {
                camera: camera.clone(),
                viewport,
                colliders: colliders.to_vec(),
                published_ms: now,
                enabled,
            });
        }
    }
}

impl Drop for ControlPanels {
    fn drop(&mut self) {
        self.bridge.borrow_mut().invalidate();
    }
}

// A tiny fixed uppercase bitmap alphabet produces persistent mesh textures, not HUD text.
fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 14],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'N' => [17, 25, 25, 21, 19, 19, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        _ => [0; 7],
    }
}

fn label_texture(panel: usize, state: usize) -> TextureAsset {
    const W: usize = 384;
    const H: usize = 128;
    let accent = match state {
        3 | FORCED_ON => [70, 220, 150, 255],
        4 => [255, 175, 65, 255],
        2 | 5 => [110, 175, 255, 255],
        _ => [195, 210, 225, 255],
    };
    let mut rgba = [15, 23, 34, 255].repeat(W * H);
    for y in 0..H {
        for x in 0..W {
            if !(4..W - 4).contains(&x) || !(4..H - 4).contains(&y) {
                rgba[(y * W + x) * 4..(y * W + x) * 4 + 4].copy_from_slice(&accent);
            }
        }
    }
    let title = ["SOUND", "FULLSCREEN", "SUBTITLES"][panel];
    let status = [
        "UNAVAILABLE",
        "OFF",
        "PENDING",
        "ON",
        "RETRY",
        "LOADING",
        "FORCED ON",
    ][state];
    let hint = if state == FORCED_ON {
        "AUDIO OFF"
    } else {
        "CLICK PANEL"
    };
    for (text, top, scale) in [(title, 10, 4), (status, 51, 4), (hint, 96, 3)] {
        let left = (W - (text.len() * 6 - 1) * scale) / 2;
        for (letter, character) in text.chars().enumerate() {
            for (row, bits) in glyph(character).iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                let offset = ((top + row * scale + dy) * W
                                    + left
                                    + (letter * 6 + column) * scale
                                    + dx)
                                    * 4;
                                rgba[offset..offset + 4].copy_from_slice(&accent);
                            }
                        }
                    }
                }
            }
        }
    }
    TextureAsset {
        handle: SpriteHandle::new(),
        width: W as u32,
        height: H as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bridge() -> Bridge {
        Bridge {
            states: [3, 1, 3],
            snapshot: Some(Snapshot {
                camera: Camera {
                    eye: theater::SPAWN,
                    target: CENTERS[0],
                    ..Camera::default()
                },
                viewport: UVec2::new(1280, 720),
                colliders: theater::Theater::new().colliders,
                published_ms: 100.0,
                enabled: true,
            }),
            ..Bridge::default()
        }
    }
    fn portfolio_bridge(target: Vec3) -> Bridge {
        let mut bridge = bridge();
        bridge.world_panels = false;
        bridge.player_eye = theater::SPAWN;
        bridge.snapshot.as_mut().unwrap().camera.target = target;
        bridge
    }

    #[test]
    fn portfolio_picks_board_halves_before_seat_exit_without_legacy_panels() {
        use theater::decor::PORTFOLIO_CENTER;
        for (x, expected) in [(-0.6, 4), (0.6, 5)] {
            let mut bridge = portfolio_bridge(PORTFOLIO_CENTER + Vec3::X * x);
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), expected);
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
            bridge.world_panels = true;
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), expected);
            bridge.seated = Some(0);
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), expected);
            assert!(!bridge.queue(expected, 100.0));
            assert!(bridge.pending.is_none());
        }
    }

    #[test]
    fn portfolio_rejects_canvas_lamps_outside_board_and_back_face() {
        use theater::decor::{
            PORTFOLIO_BOTTOM, PORTFOLIO_CENTER, PORTFOLIO_HALF_WIDTH, PORTFOLIO_TOP,
        };
        for target in [
            PORTFOLIO_CENTER + Vec3::X * (PORTFOLIO_HALF_WIDTH + 0.02),
            PORTFOLIO_CENTER - Vec3::X * (PORTFOLIO_HALF_WIDTH + 0.02),
            Vec3::new(0.0, PORTFOLIO_BOTTOM - 0.02, PORTFOLIO_CENTER.z),
            Vec3::new(0.0, PORTFOLIO_TOP + 0.02, PORTFOLIO_CENTER.z),
            Vec3::new(1.7, PORTFOLIO_CENTER.y, PORTFOLIO_CENTER.z),
            Vec3::new(0.0, 4.35, PORTFOLIO_CENTER.z),
        ] {
            assert_eq!(portfolio_bridge(target).pick_scene(Vec2::ZERO, 100.0), 0);
        }
        let mut bridge = portfolio_bridge(PORTFOLIO_CENTER);
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.colliders.clear();
        snapshot.camera.eye = PORTFOLIO_CENTER - Vec3::Z * 2.0;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
    }

    #[test]
    fn portfolio_obeys_freshness_range_los_and_input_lifecycle() {
        use theater::decor::PORTFOLIO_CENTER;
        let mut bridge = portfolio_bridge(PORTFOLIO_CENTER - Vec3::X * 0.5);
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 350.0), 4);
        for now in [99.0, 350.01, f64::NAN] {
            assert_eq!(bridge.pick_scene(Vec2::ZERO, now), 0);
        }
        assert_eq!(bridge.pick_scene(Vec2::NAN, 100.0), 0);
        bridge.snapshot.as_mut().unwrap().camera.eye = Vec3::new(0.0, 3.3, 11.0);
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
        // The lobby ceiling is a real occluder even when the camera is close.
        bridge.snapshot.as_mut().unwrap().camera.eye = Vec3::new(0.0, 5.0, 3.0);
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
        bridge.snapshot.as_mut().unwrap().colliders.clear();
        // Ceiling/floor occlusion is independent of the explicit collider list.
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
        bridge.snapshot.as_mut().unwrap().camera.eye = theater::SPAWN;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 4);
        bridge.snapshot.as_mut().unwrap().enabled = false;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
        bridge.snapshot.as_mut().unwrap().enabled = true;
        bridge.invalidate();
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
    }

    fn seat_bridge(index: usize) -> Bridge {
        let mut bridge = bridge();
        let seat = theater::SEATS[index];
        bridge.player_eye = seat.approach();
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.eye = seat.approach();
        snapshot.camera.target = seat.position + Vec3::Y * 0.5;
        bridge
    }

    #[test]
    fn no_world_panels_allocate_nothing_and_never_own_a_hit() {
        let mut assets = AppAssets::default();
        let mut meshes = Vec::new();
        let panels = ControlPanels::new(&mut assets, &mut meshes, false);
        assert!(panels.world.is_none());
        assert!(assets.meshes.is_empty());
        assert!(assets.textures.is_empty());
        assert!(meshes.is_empty());
        let source = bridge();
        let mut snapshot = source.snapshot.unwrap();
        for (index, center) in CENTERS.iter().enumerate() {
            snapshot.camera.target = *center;
            // Compatibility state updates must not enable invisible targets.
            assert!(panels.bridge.borrow_mut().set_state(index as u32 + 1, 3));
            panels.publish(
                &snapshot.camera,
                snapshot.viewport,
                &snapshot.colliders,
                100.0,
                true,
            );
            assert_eq!(panels.bridge.borrow().pick(Vec2::ZERO, 100.0), 0);
            // The enlarged plaque is behind the absent sound panel, but the
            // invisible control must never intercept its independently picked link.
            let link = if index == 0 { 5 } else { 0 };
            assert_eq!(pick_current_interaction(Vec2::ZERO, 100.0), link);
            set_seating_context(theater::SPAWN, Some(0), [false; theater::SEAT_COUNT]);
            assert_eq!(
                pick_current_interaction(Vec2::ZERO, 100.0),
                if link != 0 { link } else { 200 }
            );
            assert_eq!(queue_seat_request(200, 100.0), link == 0);
            assert_eq!(
                take_seat_request(),
                if link == 0 { Some(200) } else { None }
            );
            set_seating_context(theater::SPAWN, None, [false; theater::SEAT_COUNT]);
        }
        panels.refresh_meshes(&mut meshes);
        assert!(assets.meshes.is_empty());
        assert!(assets.textures.is_empty());
        assert!(meshes.is_empty());
    }

    #[test]
    fn no_world_panels_preserve_seating_suspension_and_retirement() {
        let mut assets = AppAssets::default();
        let mut meshes = Vec::new();
        let panels = ControlPanels::new(&mut assets, &mut meshes, false);
        for index in 0..theater::SEAT_COUNT {
            let source = seat_bridge(index);
            let snapshot = source.snapshot.unwrap();
            let target = 100 + index as u32;
            set_seating_context(source.player_eye, None, [false; theater::SEAT_COUNT]);
            panels.publish(
                &snapshot.camera,
                snapshot.viewport,
                &snapshot.colliders,
                100.0,
                true,
            );
            assert_eq!(pick_current_interaction(Vec2::ZERO, 100.0), target);
            assert!(queue_seat_request(target, 100.0));
            invalidate_scene_interactions();
            assert_eq!(take_seat_request(), None);
            assert_eq!(pick_current_interaction(Vec2::ZERO, 100.0), 0);
            panels.publish(
                &snapshot.camera,
                snapshot.viewport,
                &snapshot.colliders,
                101.0,
                true,
            );
            assert_eq!(pick_current_interaction(Vec2::ZERO, 101.0), target);
            assert!(queue_seat_request(target, 101.0));
            assert_eq!(take_seat_request(), Some(target));
        }
        let bridge = Rc::clone(&panels.bridge);
        bridge.borrow_mut().invalidate();
        let snapshot = seat_bridge(0).snapshot.unwrap();
        panels.publish(
            &snapshot.camera,
            snapshot.viewport,
            &snapshot.colliders,
            102.0,
            true,
        );
        assert!(bridge.borrow().snapshot.is_none());
        assert!(!bridge.borrow_mut().set_state(1, 3));
        assert_eq!(pick_current_interaction(Vec2::ZERO, 102.0), 0);
        assert!(!queue_seat_request(100, 102.0));
        drop(panels);
        assert!(bridge.borrow().disposed);
        assert!(bridge.borrow().snapshot.is_none());
    }

    #[test]
    fn seats_use_physical_range_and_do_not_occlude_themselves() {
        for index in 0..theater::SEAT_COUNT {
            let mut bridge = seat_bridge(index);
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 100 + index as u32);
            assert!(!bridge.queue(100 + theater::SEAT_COUNT as u32, 100.0));
            assert!(bridge.queue(100 + index as u32, 100.0));
            assert_eq!(bridge.pending.take(), Some(100 + index as u32));
            bridge.occupied[index] = true;
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
            assert!(!bridge.queue(100 + index as u32, 100.0));
            bridge.occupied[index] = false;
            bridge.player_eye = theater::SPAWN;
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
            bridge.player_eye = theater::SEATS[index].approach();
            let snapshot = bridge.snapshot.as_mut().unwrap();
            let offset = (snapshot.camera.eye - snapshot.camera.target).normalize() * 3.0;
            snapshot.camera.eye += offset;
            assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 100 + index as u32);
        }
    }

    #[test]
    fn exact_target_occupancy_freshness_and_bounded_pending() {
        let mut bridge = seat_bridge(0);
        assert!(!bridge.queue(101, 100.0));
        assert!(!bridge.queue(200, 100.0));
        assert!(!bridge.queue(99, 100.0));
        bridge.occupied[0] = true;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
        assert!(!bridge.queue(100, 100.0));
        bridge.occupied[0] = false;
        for now in [99.0, 350.001, f64::NAN, f64::INFINITY] {
            assert!(!bridge.queue(100, now));
        }
        assert!(bridge.queue(100, 350.0));
        assert!(bridge.queue(100, 350.0));
        bridge.seated = Some(0);
        assert!(!bridge.queue(200, 100.0));
        assert_eq!(bridge.pending.take(), Some(100));
        assert_eq!(bridge.pending.take(), None);
        assert!(bridge.queue(200, 100.0));
        bridge.clear_interactions();
        assert!(bridge.pending.is_none());
        assert!(!bridge.disposed);
        assert!(!bridge.queue(200, 100.0));
    }

    #[test]
    fn cached_pick_revalidates_and_panels_own_unavailable_hits() {
        let mut bridge = seat_bridge(0);
        bridge.last_pick = Some((Vec2::ZERO, 101));
        assert!(!bridge.queue(100, 100.0));
        bridge.last_pick = Some((Vec2::ZERO, 100));
        bridge.snapshot.as_mut().unwrap().camera.target += Vec3::X * 10.0;
        assert!(!bridge.queue(100, 100.0));
        let mut bridge = self::bridge();
        bridge.seated = Some(0);
        bridge.states = [0; 3];
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 1);
        assert!(!bridge.queue(200, 100.0));
        bridge.snapshot.as_mut().unwrap().camera.target = theater::SPAWN + Vec3::X;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 200);
        assert_eq!(bridge.pick_scene(Vec2::splat(1.1), 100.0), 0);
        bridge.snapshot.as_mut().unwrap().enabled = false;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
    }

    #[test]
    fn walls_and_nearest_occupied_chairs_block_seat_picking() {
        // Aim outside the portfolio plaque so this specifically tests the wall.
        let mut bridge = seat_bridge(9);
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.eye = theater::SPAWN;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
        let mut bridge = seat_bridge(1);
        let seat = theater::SEATS[0];
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.eye = seat.position + Vec3::new(-1.0, 0.5, 0.0);
        snapshot.camera.target = theater::SEATS[1].position + Vec3::Y * 0.5;
        bridge.occupied[0] = true;
        assert_eq!(bridge.pick_scene(Vec2::ZERO, 100.0), 0);
    }

    #[test]
    fn thread_local_binding_publish_suppression_and_revival() {
        let mut assets = AppAssets::default();
        let mut meshes = Vec::new();
        let panels = ControlPanels::new(&mut assets, &mut meshes, true);
        let source = seat_bridge(0);
        let snapshot = source.snapshot.unwrap();
        set_seating_context(source.player_eye, None, [false; theater::SEAT_COUNT]);
        panels.publish(
            &snapshot.camera,
            snapshot.viewport,
            &snapshot.colliders,
            100.0,
            true,
        );
        assert_eq!(pick_current_interaction(Vec2::ZERO, 100.0), 100);
        assert!(queue_seat_request(100, 100.0));
        panels.publish(
            &snapshot.camera,
            snapshot.viewport,
            &snapshot.colliders,
            101.0,
            false,
        );
        assert_eq!(take_seat_request(), None);
        invalidate_scene_interactions();
        panels.publish(
            &snapshot.camera,
            snapshot.viewport,
            &snapshot.colliders,
            102.0,
            true,
        );
        assert!(queue_seat_request(100, 102.0));
        assert_eq!(take_seat_request(), Some(100));
        panels.bridge.borrow_mut().invalidate();
        panels.publish(
            &snapshot.camera,
            snapshot.viewport,
            &snapshot.colliders,
            103.0,
            true,
        );
        assert_eq!(pick_current_interaction(Vec2::ZERO, 103.0), 0);
    }

    #[test]
    fn picking_checks_state_freshness_coordinates_range_and_front_face() {
        let mut bridge = bridge();
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 1);
        assert_eq!(bridge.pick(Vec2::ZERO, 350.0), 1);
        for (ndc, now) in [
            (Vec2::ZERO, 351.0),
            (Vec2::ZERO, 99.0),
            (Vec2::ZERO, f64::NAN),
            (Vec2::new(f32::NAN, 0.0), 100.0),
            (Vec2::new(1.01, 0.0), 100.0),
        ] {
            assert_eq!(bridge.pick(ndc, now), 0);
        }
        for state in 0..FORCED_ON as u32 {
            assert!(bridge.set_state(1, state));
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), u32::from(state != 0));
        }
        assert!(!bridge.set_state(0, 1));
        assert!(!bridge.set_state(4, 1));
        assert!(!bridge.set_state(1, FORCED_ON as u32));
        assert!(!bridge.set_state(2, FORCED_ON as u32));
        assert!(!bridge.set_state(1, STATE_COUNT as u32));
        bridge.set_state(1, 1);
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.eye = Vec3::new(-1.5, 1.7, 11.0);
        snapshot.camera.target = CENTERS[0];
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.eye = Vec3::new(-1.5, 1.7, -1.0);
        snapshot.camera.target = CENTERS[0];
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
    }
    #[test]
    fn sound_loading_and_pending_can_reopen_confirmation_but_fullscreen_pending_cannot() {
        let mut bridge = bridge();
        for state in [2, 4, 5] {
            bridge.set_state(1, state);
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 1);
        }
        bridge.snapshot.as_mut().unwrap().camera.target = CENTERS[1];
        bridge.set_state(2, 2);
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        bridge.set_state(2, 1);
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 2);
    }
    #[test]
    fn displayed_camera_projection_maps_all_panels_and_is_unobstructed_after_counter_removal() {
        let mut bridge = bridge();
        bridge.snapshot.as_mut().unwrap().camera.target = theater::SPAWN + Vec3::NEG_Z;
        for (index, center) in CENTERS.iter().enumerate() {
            let snapshot = bridge.snapshot.as_ref().unwrap();
            let point = snapshot
                .camera
                .view_projection(1280.0 / 720.0)
                .project_point3(*center);
            assert_eq!(bridge.pick(point.truncate(), 100.0), index as u32 + 1);
        }
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.eye = Vec3::new(-1.5, 0.7, 3.0);
        snapshot.camera.target = CENTERS[0];
        assert_eq!(
            bridge.pick(Vec2::ZERO, 100.0),
            1,
            "counter removal intentionally leaves this ray unobstructed"
        );
    }
    #[test]
    fn subtitles_default_on_and_forced_on_preserves_preference() {
        assert_eq!(Bridge::default().states, [0, 0, 3]);
        let mut bridge = bridge();
        bridge.snapshot.as_mut().unwrap().camera.target = CENTERS[2];
        for state in 0..STATE_COUNT as u32 {
            assert!(bridge.set_state(3, state));
            assert_eq!(bridge.display_state(2), state as usize);
            let enabled = ![0, 2, FORCED_ON as u32].contains(&state);
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), if enabled { 3 } else { 0 });
        }
        assert!(!bridge.set_state(3, STATE_COUNT as u32));
        for preference in [1, 3] {
            assert!(bridge.set_state(3, preference));
            assert!(bridge.set_state(1, 1));
            assert_eq!(bridge.display_state(2), FORCED_ON);
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
            assert_eq!(bridge.states[2], preference as usize);
            assert!(bridge.set_state(1, 3));
            assert_eq!(bridge.display_state(2), preference as usize);
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 3);
        }
        bridge.invalidate();
        assert!(!bridge.set_state(3, 3));
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
    }

    #[test]
    fn subtitle_textures_follow_effective_state_without_reallocating() {
        let mut assets = AppAssets::default();
        let mut meshes = Vec::new();
        let panels = ControlPanels::new(&mut assets, &mut meshes, true);
        let handles: Vec<_> = assets
            .textures
            .iter()
            .map(|texture| texture.handle)
            .collect();
        for (sound, subtitles, expected) in [(3, 1, 1), (3, 3, 3), (1, 1, 6), (3, 6, 6)] {
            panels.bridge.borrow_mut().set_state(1, sound);
            panels.bridge.borrow_mut().set_state(3, subtitles);
            panels.refresh_meshes(&mut meshes);
            assert_eq!(
                meshes[2].texture,
                Some(panels.world.as_ref().unwrap().textures[2][expected])
            );
            assert_eq!(meshes[2].lighting.as_ref().unwrap().ambient, 1.0);
            assert_eq!(meshes[2].lighting.as_ref().unwrap().intensity, 0.0);
        }
        assert_eq!(
            handles,
            assets
                .textures
                .iter()
                .map(|texture| texture.handle)
                .collect::<Vec<_>>()
        );
        let base = 2 * STATE_COUNT;
        assert_ne!(
            assets.textures[base + 1].rgba,
            assets.textures[base + 3].rgba
        );
        assert_ne!(
            assets.textures[base + 3].rgba,
            assets.textures[base + FORCED_ON].rgba
        );
    }

    #[test]
    fn disposal_and_disabled_input_fail_closed() {
        let mut bridge = bridge();
        bridge.snapshot.as_mut().unwrap().enabled = false;
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        bridge.invalidate();
        assert!(bridge.snapshot.is_none());
        assert!(!bridge.set_state(1, 3));
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
    }
    #[test]
    fn rectangle_bounds_empty_viewport_and_degenerate_camera_fail_closed() {
        let mut bridge = bridge();
        for offset in [
            Vec3::new(WIDTH * 0.5 - 0.001, 0.0, 0.0),
            Vec3::new(0.0, HEIGHT * 0.5 - 0.001, 0.0),
        ] {
            bridge.snapshot.as_mut().unwrap().camera.target = CENTERS[0] + offset;
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 1);
        }
        for offset in [
            Vec3::new(WIDTH * 0.5 + 0.001, 0.0, 0.0),
            Vec3::new(0.0, HEIGHT * 0.5 + 0.001, 0.0),
        ] {
            bridge.snapshot.as_mut().unwrap().camera.target = CENTERS[0] + offset;
            assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        }
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.camera.target = CENTERS[0];
        snapshot.viewport = UVec2::ZERO;
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        let snapshot = bridge.snapshot.as_mut().unwrap();
        snapshot.viewport = UVec2::new(1280, 720);
        snapshot.camera.target = snapshot.camera.eye;
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
        bridge.snapshot = None;
        assert_eq!(bridge.pick(Vec2::ZERO, 100.0), 0);
    }

    #[test]
    fn app_publishes_updated_camera_preserves_meshes_and_never_revives_disposal() {
        use weaver_app_core::{FrameContext, InputFrame, WeaverApp};
        let mut app = crate::FirstPersonLab::new();
        let bridge = Rc::clone(&app.control_panels.bridge);
        assert!(bridge.borrow().snapshot.is_none());
        bridge.borrow_mut().set_state(1, 3);
        let frame = FrameContext {
            delta_seconds: 0.016,
            elapsed_seconds: 0.1,
            viewport_size: UVec2::new(1280, 720),
        };
        app.update(
            frame,
            &InputFrame {
                look_delta: Vec2::new(30.0, 0.0),
                ..InputFrame::default()
            },
        );
        assert_eq!(
            bridge.borrow().snapshot.as_ref().unwrap().camera,
            app.scene.camera
        );
        assert_ne!(app.scene.camera.eye, app.view.player.eye);
        assert_eq!(app.scene.meshes.len(), app.base_mesh_count);
        assert_eq!(
            app.scene.meshes[app.control_panels.world.as_ref().unwrap().mesh_indices[0]].texture,
            Some(app.control_panels.world.as_ref().unwrap().textures[0][3])
        );
        app.update(
            frame,
            &InputFrame {
                toggle_perspective: true,
                ..InputFrame::default()
            },
        );
        assert_eq!(app.scene.camera.eye, app.view.player.eye);
        assert_eq!(
            bridge.borrow().snapshot.as_ref().unwrap().camera,
            app.scene.camera
        );
        app.set_room_lighting(0.0, 0.0);
        assert_eq!(
            app.scene.meshes[app.control_panels.world.as_ref().unwrap().mesh_indices[0]]
                .lighting
                .as_ref()
                .unwrap()
                .ambient,
            1.0
        );
        app.host_chat.borrow_mut().set_focus(true);
        app.update(frame, &InputFrame::default());
        assert!(!bridge.borrow().snapshot.as_ref().unwrap().enabled);
        bridge.borrow_mut().invalidate();
        app.update(frame, &InputFrame::default());
        assert!(bridge.borrow().snapshot.is_none());
    }

    #[test]
    fn mesh_textures_are_distinct_persistent_and_unlit() {
        let mut assets = AppAssets::default();
        let mut meshes = Vec::new();
        let panels = ControlPanels::new(&mut assets, &mut meshes, true);
        assert_eq!(assets.meshes.len(), 3);
        assert_eq!(assets.textures.len(), 3 * STATE_COUNT);
        assert_eq!(
            meshes[2].texture,
            Some(panels.world.as_ref().unwrap().textures[2][3])
        );
        assert_eq!(assets.meshes[0].vertices[0].uv, [0.0, 1.0]);
        let off = assets.textures[1].rgba.clone();
        assert_ne!(off, assets.textures[3].rgba);
        panels.bridge.borrow_mut().set_state(1, 3);
        panels.refresh_meshes(&mut meshes);
        assert_eq!(meshes[0].texture, Some(assets.textures[3].handle));
        assert_eq!(meshes[0].lighting.as_ref().unwrap().ambient, 1.0);
        let bridge = Rc::clone(&panels.bridge);
        drop(panels);
        assert!(bridge.borrow().disposed);
    }
}
