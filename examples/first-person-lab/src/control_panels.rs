//! Local world meshes and a synchronous, permission-neutral browser picking seam.

#[cfg(any(target_arch = "wasm32", test))]
use crate::theater::{self, Collider};
use glam::Vec3;
#[cfg(any(target_arch = "wasm32", test))]
use glam::{UVec2, Vec2};
use std::{cell::RefCell, rc::Rc};
use weaver_app_core::{AppAssets, MeshAsset, TextureAsset};
#[cfg(any(target_arch = "wasm32", test))]
use weaver_render::Camera;
use weaver_render::{MeshHandle, MeshInstance, MeshLighting, RenderTransform, SpriteHandle};
use weaver_render_wgpu::Vertex;

const STATE_COUNT: usize = 5;
#[cfg(any(target_arch = "wasm32", test))]
const MAX_RANGE: f32 = 10.0;
#[cfg(any(target_arch = "wasm32", test))]
const MAX_AGE_MS: f64 = 250.0;
const WIDTH: f32 = 2.6;
const HEIGHT: f32 = 0.9;
const CENTERS: [Vec3; 2] = [Vec3::new(-1.5, 1.7, 0.04), Vec3::new(1.5, 1.7, 0.04)];

#[derive(Default)]
struct Bridge {
    disposed: bool,
    states: [usize; 2],
    #[cfg(any(target_arch = "wasm32", test))]
    snapshot: Option<Snapshot>,
}

#[cfg(any(target_arch = "wasm32", test))]
struct Snapshot {
    camera: Camera,
    viewport: UVec2,
    colliders: Vec<Collider>,
    published_ms: f64,
    enabled: bool,
}

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static BRIDGE: RefCell<Rc<RefCell<Bridge>>> = RefCell::new(Rc::new(RefCell::new(Bridge::default())));
}

/// Pick a world control synchronously. NDC is -1..=1, +Y upward; (0,0) is center.
/// Returns 0 for no enabled hit, 1 for sound, or 2 for fullscreen.
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

/// Publish browser-observed state: panel 1/2; state 0 unavailable, 1 off,
/// 2 pending, 3 on, 4 blocked/retry. Invalid or disposed updates return false.
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

impl Bridge {
    #[cfg(any(target_arch = "wasm32", test))]
    fn set_state(&mut self, panel: u32, state: u32) -> bool {
        if self.disposed || !(1..=2).contains(&panel) || state >= STATE_COUNT as u32 {
            return false;
        }
        self.states[(panel - 1) as usize] = state as usize;
        true
    }

    fn invalidate(&mut self) {
        self.disposed = true;
        #[cfg(any(target_arch = "wasm32", test))]
        {
            self.snapshot = None;
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn pick(&self, ndc: Vec2, now: f64) -> u32 {
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
                if matches!(self.states[index], 0 | 2) || direction.z >= -f32::EPSILON {
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

#[cfg(any(target_arch = "wasm32", test))]
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
    mesh_indices: [usize; 2],
    textures: [[SpriteHandle; STATE_COUNT]; 2],
}

impl ControlPanels {
    pub(crate) fn new(assets: &mut AppAssets, meshes: &mut Vec<MeshInstance>) -> Self {
        let bridge = Rc::new(RefCell::new(Bridge::default()));
        #[cfg(target_arch = "wasm32")]
        BRIDGE.with(|slot| *slot.borrow_mut() = Rc::clone(&bridge));
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
            mesh.texture = Some(textures[panel][0]);
            mesh.lighting = Some(MeshLighting {
                position: Vec3::ZERO,
                ambient: 1.0,
                intensity: 0.0,
            });
            let index = meshes.len();
            meshes.push(mesh);
            index
        });
        Self {
            bridge,
            mesh_indices,
            textures,
        }
    }

    pub(crate) fn refresh_meshes(&self, meshes: &mut [MeshInstance]) {
        let states = self.bridge.borrow().states;
        for (panel, index) in self.mesh_indices.iter().enumerate() {
            meshes[*index].texture = Some(self.textures[panel][states[panel]]);
            // Room lighting changes must not darken the control's own display.
            meshes[*index].lighting = Some(MeshLighting {
                position: Vec3::ZERO,
                ambient: 1.0,
                intensity: 0.0,
            });
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn publish(
        &self,
        camera: &Camera,
        viewport: UVec2,
        colliders: &[Collider],
        now: f64,
        enabled: bool,
    ) {
        let mut bridge = self.bridge.borrow_mut();
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
        3 => [70, 220, 150, 255],
        4 => [255, 175, 65, 255],
        2 => [110, 175, 255, 255],
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
    let title = if panel == 0 { "SOUND" } else { "FULLSCREEN" };
    let status = ["UNAVAILABLE", "OFF", "PENDING", "ON", "RETRY"][state];
    for (text, top, scale) in [(title, 18, 3), (status, 55, 3), ("CLICK PANEL", 99, 2)] {
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
            states: [1, 1],
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
        for state in 0..5 {
            assert!(bridge.set_state(1, state));
            assert_eq!(
                bridge.pick(Vec2::ZERO, 100.0),
                u32::from(!matches!(state, 0 | 2))
            );
        }
        assert!(!bridge.set_state(0, 1));
        assert!(!bridge.set_state(3, 1));
        assert!(!bridge.set_state(1, 5));
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
    fn displayed_camera_projection_maps_both_panels_and_is_unobstructed_after_counter_removal() {
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
            app.scene.meshes[app.control_panels.mesh_indices[0]].texture,
            Some(app.control_panels.textures[0][3])
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
            app.scene.meshes[app.control_panels.mesh_indices[0]]
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
        let panels = ControlPanels::new(&mut assets, &mut meshes);
        assert_eq!(assets.meshes.len(), 2);
        assert_eq!(assets.textures.len(), 10);
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
