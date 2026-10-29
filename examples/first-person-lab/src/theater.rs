//! Fixed theater layout and curated materials: north is -Z; dimensions are meters.

pub(crate) mod decor;
mod projector;

#[cfg(test)]
pub(crate) const PROJECTOR_MESH_COUNT: usize = projector::MESH_COUNT;

use glam::{Vec2, Vec3};
use std::sync::OnceLock;
use weaver_app_core::{AppAssets, CameraBounds, MeshAsset, TextureAsset};
use weaver_render::{
    MeshHandle, MeshInstance, MeshLighting, MeshRenderMode, MeshTextureFit, RenderTransform,
    SpriteHandle,
};
use weaver_render_wgpu::Vertex;

pub(crate) const HALF_WIDTH: f32 = 12.5;
pub(crate) const MIN_Z: f32 = -28.0;
pub(crate) const MAX_Z: f32 = 10.0;
pub(crate) const HEIGHT: f32 = 12.0;
pub(crate) const LOBBY_LIGHT_LEVEL: f32 = 1.0;
pub(crate) const AUDITORIUM_LIGHT_LEVEL: f32 = 0.48;
const LOBBY_AMBIENT_SHARE: f32 = 0.42;
const AUDITORIUM_AMBIENT_SHARE: f32 = 0.62;
pub(crate) const PLAYER_RADIUS: f32 = 0.3;
pub(crate) const SPAWN: Vec3 = Vec3::new(0.0, super::EYE_HEIGHT, MAX_Z - 2.0);
const LOBBY_MIN: Vec2 = Vec2::new(-HALF_WIDTH, 0.0);
const LOBBY_MAX: Vec2 = Vec2::new(HALF_WIDTH, MAX_Z);
const AUDITORIUM_MIN: Vec2 = Vec2::new(-22.5, MIN_Z);
const AUDITORIUM_MAX: Vec2 = Vec2::new(9.0, LOBBY_MIN.y);
const PASSAGE_MIN: f32 = AUDITORIUM_MAX.x;
const PASSAGE_MAX: f32 = HALF_WIDTH;
const CORRIDOR_MIN: Vec2 = Vec2::new(PASSAGE_MIN, MIN_Z);
const CORRIDOR_MAX: Vec2 = Vec2::new(PASSAGE_MAX, 0.0);
const SIDE_ENTRY_MIN_Z: f32 = -12.0;
const SIDE_ENTRY_MAX_Z: f32 = -9.0;
const REAR_PLATFORM_HEIGHT: f32 = 2.4;
const LOBBY_FLOOR: [(f32, f32); 2] = [(0.0, 0.0), (MAX_Z, 0.0)];
// (Z, height), north to south. Equal heights make broad flats; changing
// heights make continuous ramps. Rendering and all ground queries share these.
const AUDITORIUM_FLOOR: [(f32, f32); 6] = [
    (MIN_Z, 0.0),
    (-25.0, 0.0),
    (-21.0, 1.2),
    (-16.0, 1.2),
    (SIDE_ENTRY_MIN_Z, REAR_PLATFORM_HEIGHT),
    (0.0, REAR_PLATFORM_HEIGHT),
];
const CORRIDOR_FLOOR: [(f32, f32); 3] = [
    (MIN_Z, REAR_PLATFORM_HEIGHT),
    (SIDE_ENTRY_MAX_Z, REAR_PLATFORM_HEIGHT),
    (0.0, 0.0),
];
const LOW_CEILING_CLEARANCE: f32 = 4.5;
const LOBBY_CEILING: [(f32, f32); 2] = raised_profile(LOBBY_FLOOR);
const CORRIDOR_CEILING: [(f32, f32); 3] = raised_profile(CORRIDOR_FLOOR);
const AUDITORIUM_CEILING: [(f32, f32); 2] = [(MIN_Z, HEIGHT), (0.0, HEIGHT)];

const fn raised_profile<const N: usize>(mut profile: [(f32, f32); N]) -> [(f32, f32); N] {
    let mut i = 0;
    while i < N {
        profile[i].1 += LOW_CEILING_CLEARANCE;
        i += 1;
    }
    profile
}

const fn ceiling_profile(position: Vec3) -> &'static [(f32, f32)] {
    if position.z >= 0.0 {
        &LOBBY_CEILING
    } else if position.x < PASSAGE_MIN {
        &AUDITORIUM_CEILING
    } else {
        &CORRIDOR_CEILING
    }
}

const fn ceiling_height(position: Vec3) -> f32 {
    profile_height(position.z, ceiling_profile(position))
}

const SCREEN_CENTER_X: f32 = f32::midpoint(AUDITORIUM_MIN.x, AUDITORIUM_MAX.x);
pub(crate) const DEFAULT_VIDEO_ASPECT: f32 = 16.0 / 9.0;
const SCREEN_ASPECT: f32 = 2.39;
const SCREEN_HEIGHT: f32 = 19.0 / DEFAULT_VIDEO_ASPECT;
const SCREEN_WIDTH: f32 = SCREEN_HEIGHT * SCREEN_ASPECT;
const SCREEN_BOTTOM: f32 = 0.65;
const SCREEN_TOP: f32 = SCREEN_BOTTOM + SCREEN_HEIGHT;
const SCREEN_FRAME_BORDER: f32 = 0.15;
pub(crate) const SCREEN_TARGET: Vec3 = Vec3::new(
    SCREEN_CENTER_X,
    SCREEN_BOTTOM.midpoint(SCREEN_TOP),
    MIN_Z + 0.09,
);

const fn floor_profile(position: Vec3) -> &'static [(f32, f32)] {
    if position.z >= 0.0 {
        &LOBBY_FLOOR
    } else if position.x < PASSAGE_MIN {
        &AUDITORIUM_FLOOR
    } else {
        &CORRIDOR_FLOOR
    }
}

const fn profile_height(z: f32, profile: &[(f32, f32)]) -> f32 {
    if z <= profile[0].0 {
        return profile[0].1;
    }
    let mut i = 1;
    while i < profile.len() {
        let (end_z, end_height) = profile[i];
        if z <= end_z {
            let (start_z, start_height) = profile[i - 1];
            return start_height + (end_height - start_height) * (z - start_z) / (end_z - start_z);
        }
        i += 1;
    }
    profile[profile.len() - 1].1
}

/// Ground surface in meters; Y is ignored. Different room heights meet only
/// through the level side entrance, with the remaining boundaries sealed by walls.
pub(crate) const fn floor_height(position: Vec3) -> f32 {
    profile_height(position.z, floor_profile(position))
}

const fn grounded_eye(position: Vec3) -> Vec3 {
    Vec3::new(
        position.x,
        super::EYE_HEIGHT + floor_height(position),
        position.z,
    )
}

/// Contain source video on the fixed cinema surface without changing its mesh,
/// transform, UVs or lighting. Invalid aspects preserve the last valid fit.
pub(crate) fn fit_screen(
    instances: &mut [MeshInstance],
    screen_texture: SpriteHandle,
    aspect: f32,
) -> bool {
    if !aspect.is_finite() || aspect <= 0.0 {
        return false;
    }
    let mut changed = false;
    for instance in instances {
        // Additive consumers opt into screen mapping explicitly; the lens uses
        // fixed center sampling and must not acquire contain-fit metadata.
        if instance.texture != Some(screen_texture)
            || (instance.render_mode == MeshRenderMode::Additive && instance.texture_fit.is_none())
            || instance.texture_fit.as_ref().is_some_and(|fit| {
                fit.surface_aspect == SCREEN_ASPECT && fit.content_aspect == aspect
            })
        {
            continue;
        }
        instance.texture_fit = Some(MeshTextureFit {
            surface_aspect: SCREEN_ASPECT,
            content_aspect: aspect,
        });
        changed = true;
    }
    changed
}

const CHAIR_HALF_WIDTH: f32 = 0.45;
const CHAIR_HALF_DEPTH: f32 = 0.45;
const CHAIR_BACK_HEIGHT: f32 = 0.95;
const SEAT_HEIGHT: f32 = 0.48;
const SEATED_EYE_HEIGHT: f32 = 1.25;
#[cfg(test)]
pub(crate) const CHAIR_MESH_COUNT: usize = SEATS.len();

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Seat {
    pub(crate) id: &'static str,
    /// Floor/base center, not a standing or seated eye. All chairs face -Z.
    pub(crate) position: Vec3,
}

impl Seat {
    const fn new(id: &'static str, x: f32, z: f32) -> Self {
        Self {
            id,
            position: Vec3::new(x, floor_height(Vec3::new(x, 0.0, z)), z),
        }
    }

    /// Grounded standing exit: in front of the rear row, behind the middle row.
    /// These sides avoid the rear NPC diagonal and the middle resident marks.
    pub(crate) fn approach(self) -> Vec3 {
        let offset = if self.position.z >= SIDE_ENTRY_MIN_Z {
            -1.1
        } else {
            1.1
        };
        grounded_eye(self.position + Vec3::Z * offset)
    }

    pub(crate) fn seated_eye(self) -> Vec3 {
        self.position + Vec3::Y * SEATED_EYE_HEIGHT
    }

    /// Unexpanded world-space bounds for picking and the physical chair envelope.
    pub(crate) fn bounds(self) -> (Vec3, Vec3) {
        (
            self.position + Vec3::new(-CHAIR_HALF_WIDTH, 0.0, -CHAIR_HALF_DEPTH),
            self.position + Vec3::new(CHAIR_HALF_WIDTH, CHAIR_BACK_HEIGHT, CHAIR_HALF_DEPTH),
        )
    }
}

// Two six-seat banks per flat leave a 3.1m center aisle. The original eight
// entries retain their wire indices; new seats extend each bank outward.
pub(crate) const SEAT_COUNT: usize = 24;
pub(crate) const SEATS: [Seat; SEAT_COUNT] = [
    Seat::new("seat_a1", -10.5, -9.5),
    Seat::new("seat_a2", -8.75, -9.5),
    Seat::new("seat_a3", -4.75, -9.5),
    Seat::new("seat_a4", -3.0, -9.5),
    Seat::new("seat_b1", -10.5, -17.5),
    Seat::new("seat_b2", -8.75, -17.5),
    Seat::new("seat_b3", -4.75, -17.5),
    Seat::new("seat_b4", -3.0, -17.5),
    Seat::new("seat_a5", -12.25, -9.5),
    Seat::new("seat_a6", -14.0, -9.5),
    Seat::new("seat_a7", -15.75, -9.5),
    Seat::new("seat_a8", -17.5, -9.5),
    Seat::new("seat_a9", -1.25, -9.5),
    Seat::new("seat_a10", 0.5, -9.5),
    Seat::new("seat_a11", 2.25, -9.5),
    Seat::new("seat_a12", 4.0, -9.5),
    Seat::new("seat_b5", -12.25, -17.5),
    Seat::new("seat_b6", -14.0, -17.5),
    Seat::new("seat_b7", -15.75, -17.5),
    Seat::new("seat_b8", -17.5, -17.5),
    Seat::new("seat_b9", -1.25, -17.5),
    Seat::new("seat_b10", 0.5, -17.5),
    Seat::new("seat_b11", 2.25, -17.5),
    Seat::new("seat_b12", 4.0, -17.5),
];

pub(crate) fn seat_index(id: &str) -> Option<u8> {
    SEATS
        .iter()
        .position(|seat| seat.id == id)
        .and_then(|index| u8::try_from(index).ok())
}

pub(crate) fn seat(id: &str) -> Option<Seat> {
    seat_index(id).map(|index| SEATS[usize::from(index)])
}

pub(crate) fn npc_waypoint(id: &str) -> Option<Vec3> {
    let eye = super::EYE_HEIGHT;
    Some(grounded_eye(match id {
        "lobby_entry" => SPAWN,
        "lobby_door" => Vec3::new(0.0, eye, MAX_Z - 0.6),
        "lobby_center" => Vec3::new(0.0, eye, 5.0),
        "lobby_west" => Vec3::new(-9.5, eye, 5.0),
        "lobby_east" => Vec3::new(6.0, eye, 5.0),
        "queue_west" => Vec3::new(-7.0, eye, 3.2),
        "queue_center" => Vec3::new(-5.0, eye, 3.2),
        "queue_east" => Vec3::new(-3.0, eye, 3.2),
        "lobby_aisle" => Vec3::new(10.75, eye, 3.2),
        "passage_lobby" => Vec3::new(10.75, eye, 0.6),
        "corridor_turn" => Vec3::new(10.75, eye, -10.5),
        "auditorium_entry" => Vec3::new(8.0, eye, -10.5),
        "row_a" => Vec3::new(8.0, eye, -10.0),
        "row_b" => Vec3::new(8.0, eye, -19.0),
        "row_b_approach_aisle" => Vec3::new(8.0, eye, -16.4),
        "auditorium_back_aisle" => Vec3::new(8.0, eye, -7.5),
        "auditorium_back" => Vec3::new(SCREEN_CENTER_X, eye, -6.0),
        "auditorium_left" => Vec3::new(-17.0, eye, -19.0),
        "auditorium_right" => Vec3::new(4.0, eye, -19.0),
        "auditorium_center" => Vec3::new(SCREEN_CENTER_X, eye, -19.0),
        _ => seat(id.strip_suffix("_approach")?)?.approach(),
    }))
}

const PNGS: [&[u8]; 4] = [
    include_bytes!("../assets/environment/wall_bronze_steel_panels.png"),
    include_bytes!("../assets/textures/door_starlit_cinema.png"),
    include_bytes!("../assets/environment/ground_lobby_stone_tiles.png"),
    include_bytes!("../assets/environment/ground_auditorium_carpet_base.png"),
];

pub(crate) const POSTER_COUNT: usize = 7;
const POSTER_CENTER_Y: f32 = LOW_CEILING_CLEARANCE * 0.5;
const POSTER_PNGS: [&[u8]; POSTER_COUNT] = [
    include_bytes!("../assets/posters/Banner_Game_DragonsOfAtlantis.png"),
    include_bytes!("../assets/posters/Banner_Game_GodfatherFiveFamilies.png"),
    include_bytes!("../assets/posters/Banner_Game_StarWars.png"),
    include_bytes!("../assets/posters/Banner_Game_TheHobbit.png"),
    include_bytes!("../assets/posters/Banner_XR_BEP_MOTSTour.png"),
    include_bytes!("../assets/posters/Banner_XR_StarWars_GM.png"),
    include_bytes!("../assets/posters/Banner_XR_VerizonAthleteAR.png"),
];
// Append materials after video/posters: the browser's video texture stays at slot 4.
const MATERIAL_PNGS: [&[u8]; 2] = [
    include_bytes!("../assets/environment/wall_gunmetal_riveted_panels.png"),
    include_bytes!("../assets/environment/seat_material_atlas.png"),
];
#[cfg(test)]
pub(crate) const MATERIAL_TEXTURE_COUNT: usize = MATERIAL_PNGS.len();
const CEILING_TEXTURE_INDEX: usize = 5 + POSTER_COUNT;
const SEAT_TEXTURE_INDEX: usize = CEILING_TEXTURE_INDEX + 1;
const SEAT_ATLAS_INSET: f32 = 128.5;
const SEAT_ATLAS_SPAN: f32 = 255.0;

// Original image ratios, independent of integer rounding in downsampled textures.
const POSTER_RATIOS: [f32; POSTER_COUNT] = [
    2.0 / 3.0,
    749.0 / 623.0,
    16.0 / 9.0,
    16.0 / 9.0,
    2200.0 / 1238.0,
    2200.0 / 1238.0,
    2200.0 / 1238.0,
];

fn texture_assets() -> Vec<TextureAsset> {
    static TEXTURES: OnceLock<Vec<TextureAsset>> = OnceLock::new();
    TEXTURES
        .get_or_init(|| {
            PNGS.iter()
                .map(|png| {
                    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                        .expect("bundled theater PNG must decode")
                        .into_rgba8();
                    TextureAsset {
                        mipmaps: Vec::new(),
                        handle: SpriteHandle::new(),
                        width: image.width(),
                        height: image.height(),
                        rgba: image.into_raw(),
                    }
                })
                .chain(std::iter::once(TextureAsset {
                    mipmaps: Vec::new(),
                    handle: SpriteHandle::new(),
                    width: 1,
                    height: 1,
                    rgba: vec![0, 0, 0, 255],
                }))
                .chain(POSTER_PNGS.iter().chain(MATERIAL_PNGS.iter()).map(|png| {
                    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                        .expect("bundled poster or material must decode")
                        .into_rgba8();
                    TextureAsset {
                        mipmaps: Vec::new(),
                        handle: SpriteHandle::new(),
                        width: image.width(),
                        height: image.height(),
                        rgba: image.into_raw(),
                    }
                }))
                .collect()
        })
        .clone()
}

/// Static scene resources and the local-only physical footprint.
pub(crate) struct Theater {
    pub(crate) assets: AppAssets,
    pub(crate) instances: Vec<MeshInstance>,
    pub(crate) colliders: Vec<Collider>,
}

impl Theater {
    pub(crate) fn new() -> Self {
        let textures = texture_assets();
        let wall = Some(textures[0].handle);
        let door = Some(textures[1].handle);
        let lobby = Some(textures[2].handle);
        let auditorium = Some(textures[3].handle);
        let lobby_sw = Vec2::new(LOBBY_MIN.x, LOBBY_MAX.y);
        let lobby_ne = Vec2::new(LOBBY_MAX.x, LOBBY_MIN.y);
        let west_join = Vec2::new(AUDITORIUM_MIN.x, AUDITORIUM_MAX.y);
        let north_east = Vec2::new(HALF_WIDTH, AUDITORIUM_MIN.y);
        // Endpoints run left-to-right as viewed from inside each room. Reuse
        // these segments for physical collision and keep the mesh ordering.
        let outer_walls = [
            (LOBBY_MAX, lobby_sw),
            (lobby_sw, LOBBY_MIN),
            (lobby_ne, LOBBY_MAX),
            (AUDITORIUM_MIN, north_east),
            (west_join, AUDITORIUM_MIN),
            (north_east, lobby_ne),
            (LOBBY_MIN, west_join),
        ];
        let divider = [
            (LOBBY_MIN, Vec2::new(PASSAGE_MIN, 0.0)),
            (CORRIDOR_MIN, Vec2::new(PASSAGE_MIN, SIDE_ENTRY_MIN_Z)),
            (
                Vec2::new(PASSAGE_MIN, SIDE_ENTRY_MAX_Z),
                Vec2::new(PASSAGE_MIN, 0.0),
            ),
        ];
        let colliders: Vec<_> = outer_walls
            .iter()
            .chain(&divider)
            .map(|(a, b)| Collider::new(a.min(*b).to_array(), a.max(*b).to_array()))
            .collect();

        let mut scene = Self {
            assets: AppAssets {
                meshes: Vec::new(),
                textures,
            },
            instances: Vec::new(),
            colliders,
        };
        scene.floor(LOBBY_MIN, LOBBY_MAX, lobby, &LOBBY_FLOOR);
        scene.floor(
            AUDITORIUM_MIN,
            AUDITORIUM_MAX,
            auditorium,
            &AUDITORIUM_FLOOR,
        );
        scene.floor(CORRIDOR_MIN, CORRIDOR_MAX, lobby, &CORRIDOR_FLOOR);
        scene.ceiling(LOBBY_MIN, LOBBY_MAX, &LOBBY_CEILING);
        scene.ceiling(AUDITORIUM_MIN, AUDITORIUM_MAX, &AUDITORIUM_CEILING);
        scene.ceiling(CORRIDOR_MIN, CORRIDOR_MAX, &CORRIDOR_CEILING);

        for (a, b) in outer_walls {
            scene.wall(a, b, wall);
        }
        for (a, b) in divider {
            scene.wall(a, b, wall);
            scene.wall(b, a, wall);
        }

        // Face the theater to seal the void above the lower corridor. The ceiling
        // profile discontinuity blocks sight/camera sweeps, not walking routes.
        let lintel_bottom = ceiling_height(Vec3::new(PASSAGE_MIN, 0.0, SIDE_ENTRY_MIN_Z));
        scene.quad(
            [
                Vec3::new(PASSAGE_MIN, lintel_bottom, SIDE_ENTRY_MIN_Z),
                Vec3::new(PASSAGE_MIN, lintel_bottom, SIDE_ENTRY_MAX_Z),
                Vec3::new(PASSAGE_MIN, HEIGHT, SIDE_ENTRY_MAX_Z),
                Vec3::new(PASSAGE_MIN, HEIGHT, SIDE_ENTRY_MIN_Z),
            ],
            [
                [SIDE_ENTRY_MIN_Z / 3.0, (HEIGHT - lintel_bottom) / 3.0],
                [SIDE_ENTRY_MAX_Z / 3.0, (HEIGHT - lintel_bottom) / 3.0],
                [SIDE_ENTRY_MAX_Z / 3.0, 0.0],
                [SIDE_ENTRY_MIN_Z / 3.0, 0.0],
            ],
            [1.0; 4],
            wall,
        );

        // Enlarge the portrait door by 30% from 2.16x3.24m, keeping it grounded
        // and at 2:3 aspect. Stand clear of the sealed wall to avoid depth fighting.
        // Map once, top at v=0, without mirroring either leaf.
        let door_z = LOBBY_MAX.y - 0.15;
        scene.quad(
            [
                Vec3::new(1.404, 0.0, door_z),
                Vec3::new(-1.404, 0.0, door_z),
                Vec3::new(-1.404, 4.212, door_z),
                Vec3::new(1.404, 4.212, door_z),
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            [1.0; 4],
            door,
        );

        // Black 2.39:1 cinema area, centered on the widened auditorium. Separate
        // frame, backing and contained video to avoid depth fighting.
        scene.panel(
            SCREEN_CENTER_X - SCREEN_WIDTH * 0.5 - SCREEN_FRAME_BORDER,
            SCREEN_CENTER_X + SCREEN_WIDTH * 0.5 + SCREEN_FRAME_BORDER,
            SCREEN_BOTTOM - SCREEN_FRAME_BORDER,
            SCREEN_TOP + SCREEN_FRAME_BORDER,
            AUDITORIUM_MIN.y + 0.06,
            [0.0, 0.0, 0.0, 1.0],
        );
        scene.panel(
            SCREEN_CENTER_X - SCREEN_WIDTH * 0.5,
            SCREEN_CENTER_X + SCREEN_WIDTH * 0.5,
            SCREEN_BOTTOM,
            SCREEN_TOP,
            AUDITORIUM_MIN.y + 0.08,
            [0.0, 0.0, 0.0, 1.0],
        );
        // The video quad fills the cinema area. Opt-in texture fitting contains
        // the full source image inside it; black bars never require mesh changes.
        let half_video_width = SCREEN_WIDTH * 0.5;
        scene.quad(
            [
                Vec3::new(
                    SCREEN_CENTER_X - half_video_width,
                    SCREEN_BOTTOM,
                    SCREEN_TARGET.z,
                ),
                Vec3::new(
                    SCREEN_CENTER_X + half_video_width,
                    SCREEN_BOTTOM,
                    SCREEN_TARGET.z,
                ),
                Vec3::new(
                    SCREEN_CENTER_X + half_video_width,
                    SCREEN_TOP,
                    SCREEN_TARGET.z,
                ),
                Vec3::new(
                    SCREEN_CENTER_X - half_video_width,
                    SCREEN_TOP,
                    SCREEN_TARGET.z,
                ),
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            [1.0; 4],
            Some(scene.assets.textures[4].handle),
        );
        // The legacy shader lights `None` from the origin. Fixed ambient=1,
        // diffuse=0 instead samples video at full brightness without a material API.
        scene.instances.last_mut().unwrap().lighting = Some(MeshLighting {
            position: Vec3::ZERO,
            ambient: 1.0,
            intensity: 0.0,
        });
        fit_screen(
            &mut scene.instances,
            scene.assets.textures[4].handle,
            DEFAULT_VIDEO_ASPECT,
        );
        scene.posters();
        for seat in SEATS {
            scene.chair(seat);
        }
        projector::append(&mut scene);
        apply_room_lighting(
            &mut scene.instances,
            LOBBY_LIGHT_LEVEL,
            AUDITORIUM_LIGHT_LEVEL,
            scene.assets.textures[4].handle,
        );
        scene
    }

    pub(crate) fn camera_bounds() -> CameraBounds {
        CameraBounds::new(
            Vec3::new(
                AUDITORIUM_MIN.x + PLAYER_RADIUS,
                super::EYE_HEIGHT,
                AUDITORIUM_MIN.y + PLAYER_RADIUS,
            ),
            Vec3::new(
                LOBBY_MAX.x - PLAYER_RADIUS,
                super::EYE_HEIGHT + REAR_PLATFORM_HEIGHT,
                LOBBY_MAX.y - PLAYER_RADIUS,
            ),
        )
    }

    fn quad(
        &mut self,
        positions: [Vec3; 4],
        uvs: [[f32; 2]; 4],
        color: [f32; 4],
        texture: Option<SpriteHandle>,
    ) {
        let normal = (positions[1] - positions[0])
            .cross(positions[2] - positions[0])
            .normalize();
        let center = positions.iter().copied().sum::<Vec3>() * 0.25;
        let corridor = center.z < 0.0
            && (center.x > PASSAGE_MIN || (center.x == PASSAGE_MIN && normal.x > 0.0));
        let auditorium = !corridor && (center.z < 0.0 || (center.z == 0.0 && normal.z < 0.0));
        let vertices = positions
            .into_iter()
            .zip(uvs)
            .map(|(position, uv)| Vertex {
                position: position.to_array(),
                normal: normal.to_array(),
                color,
                uv,
            })
            .collect();
        let handle = MeshHandle::new();
        self.assets
            .meshes
            .push(MeshAsset::new(handle, vertices, vec![0, 1, 2, 0, 2, 3]));
        let mut instance = MeshInstance::new(handle, RenderTransform::default());
        instance.texture = texture;
        instance.lighting = Some(MeshLighting {
            position: Vec3::new(
                if corridor {
                    PASSAGE_MIN.midpoint(PASSAGE_MAX)
                } else if auditorium {
                    SCREEN_CENTER_X
                } else {
                    0.0
                },
                HEIGHT - 0.3,
                if auditorium || corridor {
                    AUDITORIUM_MIN.y.midpoint(AUDITORIUM_MAX.y)
                } else {
                    LOBBY_MIN.y.midpoint(LOBBY_MAX.y)
                },
            ),
            ambient: 0.35,
            intensity: 0.65,
        });
        if let Some(light) = &mut instance.lighting {
            light.position.y = ceiling_height(light.position) - 0.3;
        }
        self.instances.push(instance);
    }

    fn chair(&mut self, seat: Seat) {
        let upholstery = [1.0, 0.96, 0.92, 1.0];
        let frame = [0.70, 0.66, 0.60, 1.0];
        // The same five boxes retain their exact positions and picking envelope.
        // A guttered atlas dresses leather, padded backs, and metal in one draw.
        let parts = [
            (
                Vec3::new(-0.28, 0.0, -0.28),
                Vec3::new(0.28, 0.36, 0.28),
                frame,
                2,
            ),
            (
                Vec3::new(-0.33, 0.36, -CHAIR_HALF_DEPTH),
                Vec3::new(0.33, SEAT_HEIGHT, 0.30),
                upholstery,
                0,
            ),
            (
                Vec3::new(-CHAIR_HALF_WIDTH, 0.36, 0.30),
                Vec3::new(CHAIR_HALF_WIDTH, CHAIR_BACK_HEIGHT, CHAIR_HALF_DEPTH),
                upholstery,
                1,
            ),
            (
                Vec3::new(-CHAIR_HALF_WIDTH, 0.36, -CHAIR_HALF_DEPTH),
                Vec3::new(-0.33, 0.65, 0.30),
                frame,
                3,
            ),
            (
                Vec3::new(0.33, 0.36, -CHAIR_HALF_DEPTH),
                Vec3::new(CHAIR_HALF_WIDTH, 0.65, 0.30),
                frame,
                3,
            ),
        ];
        let mut vertices = Vec::with_capacity(parts.len() * 24);
        let mut indices = Vec::with_capacity(parts.len() * 36);
        for (min, max, color, quadrant) in parts {
            // Wide extruded gutters isolate materials through mip 8, where the
            // sampled 256px region is already only one screen pixel across.
            let u0 = (SEAT_ATLAS_INSET + (quadrant % 2) as f32 * 512.0) / 1024.0;
            let v0 = (SEAT_ATLAS_INSET + (quadrant / 2) as f32 * 512.0) / 1024.0;
            let u1 = u0 + SEAT_ATLAS_SPAN / 1024.0;
            let v1 = v0 + SEAT_ATLAS_SPAN / 1024.0;
            let uvs = [[u0, v1], [u1, v1], [u1, v0], [u0, v0]];
            let min = seat.position + min;
            let max = seat.position + max;
            let corners = [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(max.x, min.y, min.z),
                Vec3::new(max.x, max.y, min.z),
                Vec3::new(min.x, max.y, min.z),
                Vec3::new(min.x, min.y, max.z),
                Vec3::new(max.x, min.y, max.z),
                Vec3::new(max.x, max.y, max.z),
                Vec3::new(min.x, max.y, max.z),
            ];
            for face in [
                [1, 0, 3, 2],
                [4, 5, 6, 7],
                [0, 4, 7, 3],
                [5, 1, 2, 6],
                [3, 7, 6, 2],
                [0, 1, 5, 4],
            ] {
                let points = face.map(|index| corners[index]);
                let normal = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize();
                let base = u16::try_from(vertices.len()).expect("bounded chair mesh");
                vertices.extend(points.into_iter().zip(uvs).map(|(position, uv)| Vertex {
                    position: position.to_array(),
                    normal: normal.to_array(),
                    color,
                    uv,
                }));
                indices.extend([0, 1, 2, 0, 2, 3].map(|index| base + index));
            }
        }
        let handle = MeshHandle::new();
        self.assets
            .meshes
            .push(MeshAsset::new(handle, vertices, indices));
        let mut instance = MeshInstance::new(handle, RenderTransform::default());
        instance.lighting = Some(MeshLighting {
            position: Vec3::new(
                SCREEN_CENTER_X,
                HEIGHT - 0.3,
                AUDITORIUM_MIN.y.midpoint(AUDITORIUM_MAX.y),
            ),
            ambient: 0.35,
            intensity: 0.65,
        });
        instance.texture = Some(self.assets.textures[SEAT_TEXTURE_INDEX].handle);
        self.instances.push(instance);
        let (min, max) = seat.bounds();
        self.colliders.push(Collider {
            min: Vec2::new(min.x, min.z),
            max: Vec2::new(max.x, max.z),
            height: max.y,
        });
    }

    fn posters(&mut self) {
        // North bays flank the controls and stop before the passage. East bays
        // leave the passage approach clear; south bays flank the entrance artwork.
        let placements = [
            (
                Vec3::new(-HALF_WIDTH, POSTER_CENTER_Y, 5.0),
                Vec3::NEG_Z,
                Vec3::X,
                2.0,
            ),
            (Vec3::new(-9.9, POSTER_CENTER_Y, 0.0), Vec3::X, Vec3::Z, 3.6),
            (Vec3::new(5.3, POSTER_CENTER_Y, 0.0), Vec3::X, Vec3::Z, 4.2),
            (
                Vec3::new(HALF_WIDTH, POSTER_CENTER_Y, 2.6),
                Vec3::Z,
                Vec3::NEG_X,
                4.0,
            ),
            (
                Vec3::new(HALF_WIDTH, POSTER_CENTER_Y, 7.3),
                Vec3::Z,
                Vec3::NEG_X,
                4.0,
            ),
            (
                Vec3::new(-7.0, POSTER_CENTER_Y, MAX_Z),
                Vec3::NEG_X,
                Vec3::NEG_Z,
                4.8,
            ),
            (
                Vec3::new(7.0, POSTER_CENTER_Y, MAX_Z),
                Vec3::NEG_X,
                Vec3::NEG_Z,
                4.8,
            ),
        ];
        for (index, (ratio, (center, right, normal, max_width))) in
            POSTER_RATIOS.into_iter().zip(placements).enumerate()
        {
            let height = 3.0_f32.min(max_width / ratio);
            let width = height * ratio;
            for (border, depth, color, texture) in [
                (0.08, 0.025, [0.12, 0.10, 0.07, 1.0], None),
                (
                    0.0,
                    0.04,
                    [1.0; 4],
                    Some(self.assets.textures[5 + index].handle),
                ),
            ] {
                let origin = center + normal * depth;
                let x = right * (width * 0.5 + border);
                let y = Vec3::Y * (height * 0.5 + border);
                self.quad(
                    [
                        origin - x - y,
                        origin + x - y,
                        origin + x + y,
                        origin - x + y,
                    ],
                    [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                    color,
                    texture,
                );
            }
        }
    }

    fn floor(
        &mut self,
        min: Vec2,
        max: Vec2,
        texture: Option<SpriteHandle>,
        profile: &[(f32, f32)],
    ) {
        // Lobby and hallway share one stone grid through their existing opening;
        // the different auditorium carpet keeps its own continuous 4m projection.
        let origin = if min == CORRIDOR_MIN { LOBBY_MIN } else { min };
        for pair in profile.windows(2) {
            let points = [
                Vec2::new(min.x, pair[0].0),
                Vec2::new(min.x, pair[1].0),
                Vec2::new(max.x, pair[1].0),
                Vec2::new(max.x, pair[0].0),
            ];
            self.quad(
                points.map(|p| Vec3::new(p.x, profile_height(p.y, profile), p.y)),
                points.map(|p| [(p.x - origin.x) / 4.0, (p.y - origin.y) / 4.0]),
                [1.0, 0.98, 0.94, 1.0],
                texture,
            );
        }
    }

    fn ceiling(&mut self, min: Vec2, max: Vec2, profile: &[(f32, f32)]) {
        let origin = if min == CORRIDOR_MIN { LOBBY_MIN } else { min };
        for pair in profile.windows(2) {
            let positions = [
                Vec3::new(min.x, pair[1].1, pair[1].0),
                Vec3::new(min.x, pair[0].1, pair[0].0),
                Vec3::new(max.x, pair[0].1, pair[0].0),
                Vec3::new(max.x, pair[1].1, pair[1].0),
            ];
            self.quad(
                positions,
                positions.map(|p| [(p.x - origin.x) / 3.0, (p.z - origin.y) / 3.0]),
                [0.56, 0.55, 0.54, 1.0],
                Some(self.assets.textures[CEILING_TEXTURE_INDEX].handle),
            );
        }
    }

    fn wall(&mut self, a: Vec2, b: Vec2, texture: Option<SpriteHandle>) {
        let direction = (b - a).normalize();
        let start_u = a.dot(direction) / 3.0;
        let end_u = b.dot(direction) / 3.0;
        let v = HEIGHT / 3.0;
        self.quad(
            [
                Vec3::new(a.x, 0.0, a.y),
                Vec3::new(b.x, 0.0, b.y),
                Vec3::new(b.x, HEIGHT, b.y),
                Vec3::new(a.x, HEIGHT, a.y),
            ],
            [[start_u, v], [end_u, v], [end_u, 0.0], [start_u, 0.0]],
            [1.0, 0.95, 0.86, 1.0],
            texture,
        );
    }

    fn panel(&mut self, left: f32, right: f32, bottom: f32, top: f32, z: f32, color: [f32; 4]) {
        self.quad(
            [
                Vec3::new(left, bottom, z),
                Vec3::new(right, bottom, z),
                Vec3::new(right, top, z),
                Vec3::new(left, top, z),
            ],
            [[0.0; 2]; 4],
            color,
            None,
        );
    }
}

pub(crate) fn in_auditorium(position: Vec3) -> bool {
    position.z < 0.0 && position.x < PASSAGE_MIN
}

/// Apply independent room light levels without modifying artwork or emission.
pub(crate) fn apply_room_lighting(
    instances: &mut [MeshInstance],
    lobby: f32,
    auditorium: f32,
    screen_texture: SpriteHandle,
) {
    for instance in instances {
        if instance.texture == Some(screen_texture) {
            continue;
        }
        if let Some(light) = &mut instance.lighting {
            let level = if in_auditorium(light.position) {
                auditorium
            } else {
                lobby
            };
            // A larger ambient share keeps dark wall/seat details and ramps
            // readable without brightening the unlit screen or adding real lights.
            let ambient_share = if in_auditorium(light.position) {
                AUDITORIUM_AMBIENT_SHARE
            } else {
                LOBBY_AMBIENT_SHARE
            };
            light.ambient = ambient_share * level;
            light.intensity = (1.0 - ambient_share) * level;
        }
    }
}

/// Solid column from world Y=0 to an absolute top; raised floors hide its base.
/// Walking clearance expands XZ; visibility uses the finite physical top.
#[derive(Clone)]
pub(crate) struct Collider {
    min: Vec2,
    max: Vec2,
    height: f32,
}

impl Collider {
    fn new(min: [f32; 2], max: [f32; 2]) -> Self {
        Self {
            min: Vec2::from_array(min),
            max: Vec2::from_array(max),
            height: HEIGHT,
        }
    }

    fn intersects_segment(&self, start: Vec3, end: Vec3) -> bool {
        let min = Vec3::new(self.min.x, 0.0, self.min.y).as_dvec3();
        let max = Vec3::new(self.max.x, self.height, self.max.y).as_dvec3();
        let start = start.as_dvec3();
        let direction = end.as_dvec3() - start;
        let mut enter: f64 = 0.0;
        let mut exit: f64 = 1.0;
        // Closed slabs include contact at an endpoint or edge. Zero-width wall
        // planes and parallel/coplanar segments need no artificial thickness.
        // f64 intermediates keep tiny nonzero directions safe to divide by.
        for axis in 0..3 {
            if direction[axis] == 0.0 {
                if start[axis] < min[axis] || start[axis] > max[axis] {
                    return false;
                }
            } else {
                let a = (min[axis] - start[axis]) / direction[axis];
                let b = (max[axis] - start[axis]) / direction[axis];
                enter = enter.max(a.min(b));
                exit = exit.min(a.max(b));
                if enter > exit {
                    return false;
                }
            }
        }
        true
    }
}

// A ray crosses only finitely many linear floor/ceiling pieces. Splitting at every
// profile/room boundary finds first contact even when both endpoints are clear
// but the intervening raised flat blocks the ray. No frame-size sampling is used.
fn floor_contact(start: Vec3, end: Vec3, clearance: f32) -> Option<f64> {
    surface_contact(start, end, clearance, false)
}

fn ceiling_contact(start: Vec3, end: Vec3, clearance: f32) -> Option<f64> {
    surface_contact(start, end, clearance, true)
}

fn surface_contact(start: Vec3, end: Vec3, clearance: f32, ceiling: bool) -> Option<f64> {
    let origin = start.as_dvec3();
    let direction = end.as_dvec3() - origin;
    let radius = f64::from(clearance);
    let offsets = [
        glam::DVec3::ZERO,
        glam::DVec3::new(-radius, 0.0, -radius),
        glam::DVec3::new(-radius, 0.0, radius),
        glam::DVec3::new(radius, 0.0, -radius),
        glam::DVec3::new(radius, 0.0, radius),
    ];
    let mut contact: Option<f64> = None;
    for offset in &offsets[..if clearance == 0.0 { 1 } else { 5 }] {
        let origin = origin + offset;
        let mut splits = vec![0.0, 1.0];
        for (axis, plane) in std::iter::once((0, PASSAGE_MIN)).chain(
            AUDITORIUM_FLOOR
                .iter()
                .chain(&CORRIDOR_FLOOR)
                .map(|&(z, _)| (2, z)),
        ) {
            if direction[axis] != 0.0 {
                let t = (f64::from(plane) - origin[axis]) / direction[axis];
                if t > 0.0 && t < 1.0 {
                    splits.push(t);
                }
            }
        }
        splits.sort_by(f64::total_cmp);
        splits.dedup();
        for interval in splits.windows(2) {
            let from = origin + direction * interval[0];
            let to = origin + direction * interval[1];
            // Select by interval interior so both sides of a height discontinuity
            // are tested. This also closes the lintel when entering horizontally.
            let midpoint = ((from + to) * 0.5).as_vec3();
            let profile = if ceiling {
                ceiling_profile(midpoint)
            } else {
                floor_profile(midpoint)
            };
            let gap = |point: glam::DVec3| {
                let height = f64::from(profile_height(point.z as f32, profile));
                if ceiling {
                    height - point.y - radius
                } else {
                    point.y - height - radius
                }
            };
            let from_gap = gap(from);
            let to_gap = gap(to);
            let hit = if from_gap < -1.0e-6 {
                Some(interval[0])
            } else if to_gap < -1.0e-6 {
                Some(
                    interval[0]
                        + (interval[1] - interval[0]) * from_gap.max(0.0) / (from_gap - to_gap),
                )
            } else {
                None
            };
            if let Some(hit) = hit {
                contact = Some(contact.map_or(hit, |previous| previous.min(hit)));
                break;
            }
        }
    }
    contact
}

/// Visibility uses physical walls, ground and ceilings, never player-radius-expanded boxes.
pub(crate) fn has_line_of_sight(start: Vec3, end: Vec3, colliders: &[Collider]) -> bool {
    start.is_finite()
        && end.is_finite()
        && floor_contact(start, end, 0.0).is_none()
        && ceiling_contact(start, end, 0.0).is_none()
        && !colliders
            .iter()
            .any(|collider| collider.intersects_segment(start, end))
}

/// Validate grounded endpoints and the analytic XZ walking route against player
/// clearance. Evaluate walking Y with `floor_height`, not an endpoint-to-endpoint lerp.
pub(crate) fn npc_path_clear(start: Vec3, end: Vec3, colliders: &[Collider]) -> bool {
    let bounds = Theater::camera_bounds();
    let grounded = |point: Vec3| {
        point.is_finite()
            && point == point.clamp(bounds.min, bounds.max)
            && (point.z < 0.0 || point.x >= LOBBY_MIN.x)
            && (point.y - grounded_eye(point).y).abs() < 0.001
    };
    grounded(start)
        && grounded(end)
        && !colliders.iter().any(|collider| {
            Collider {
                min: collider.min - Vec2::splat(PLAYER_RADIUS),
                max: collider.max + Vec2::splat(PLAYER_RADIUS),
                height: HEIGHT,
            }
            .intersects_segment(start, end)
        })
}

/// Sweep a camera clearance box from the player eye, stopping at the first
/// expanded wall, tier/ramp surface or ceiling. This never changes player pose.
pub(crate) fn clip_camera(
    start: Vec3,
    desired: Vec3,
    clearance: f32,
    colliders: &[Collider],
) -> Vec3 {
    let direction = desired.as_dvec3() - start.as_dvec3();
    let origin = start.as_dvec3();
    let radius = f64::from(clearance);
    let mut limit = floor_contact(start, desired, clearance)
        .unwrap_or(1.0)
        .min(ceiling_contact(start, desired, clearance).unwrap_or(1.0));
    for collider in colliders {
        let min =
            Vec3::new(collider.min.x, 0.0, collider.min.y).as_dvec3() - glam::DVec3::splat(radius);
        let max = Vec3::new(collider.max.x, collider.height, collider.max.y).as_dvec3()
            + glam::DVec3::splat(radius);
        let mut enter: f64 = 0.0;
        let mut exit: f64 = 1.0;
        for axis in 0..3 {
            if direction[axis] == 0.0 {
                if origin[axis] < min[axis] || origin[axis] > max[axis] {
                    exit = -1.0;
                    break;
                }
            } else {
                let a = (min[axis] - origin[axis]) / direction[axis];
                let b = (max[axis] - origin[axis]) / direction[axis];
                enter = enter.max(a.min(b));
                exit = exit.min(a.max(b));
            }
        }
        if enter <= exit {
            limit = limit.min(enter);
        }
    }
    (origin + direction * limit.clamp(0.0, 1.0)).as_vec3()
}

/// Resolve bounded substeps axis-by-axis, clamping to the first swept face.
/// Tangential motion remains free, including at exact contact with a wall.
pub(crate) fn resolve_movement(start: Vec3, desired: Vec3, colliders: &[Collider]) -> Vec3 {
    let bounds = Theater::camera_bounds();
    let desired = desired.clamp(bounds.min, bounds.max);
    let displacement = Vec2::new(desired.x - start.x, desired.z - start.z);
    let steps = (displacement.abs().max_element() / 0.1).ceil().max(1.0) as u32;
    let step = displacement / steps as f32;
    let mut position = Vec2::new(start.x, start.z);
    for _ in 0..steps {
        for axis in 0..2 {
            let other = 1 - axis;
            let mut next = position[axis] + step[axis];
            for collider in colliders {
                let min = collider.min - Vec2::splat(PLAYER_RADIUS);
                let max = collider.max + Vec2::splat(PLAYER_RADIUS);
                if position[other] <= min[other] || position[other] >= max[other] {
                    continue;
                }
                if position[axis] <= min[axis] && next > min[axis] {
                    next = min[axis];
                } else if position[axis] >= max[axis] && next < max[axis] {
                    next = max[axis];
                }
            }
            position[axis] = next;
        }
    }
    grounded_eye(Vec3::new(position.x, 0.0, position.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_fingerprint(scene: &Theater) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut feed = |bytes: &[u8]| {
            for byte in bytes {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
            }
        };
        for mesh in &scene.assets.meshes {
            for vertex in &mesh.vertices {
                for value in vertex.position.into_iter().chain(vertex.normal) {
                    feed(&value.to_bits().to_le_bytes());
                }
            }
            for index in &mesh.indices {
                feed(&index.to_le_bytes());
            }
        }
        for instance in &scene.instances {
            for value in instance
                .transform
                .translation
                .to_array()
                .into_iter()
                .chain(instance.transform.rotation.to_array())
                .chain([instance.transform.scale])
            {
                feed(&value.to_bits().to_le_bytes());
            }
        }
        for collider in &scene.colliders {
            for value in collider
                .min
                .to_array()
                .into_iter()
                .chain(collider.max.to_array())
                .chain([collider.height])
            {
                feed(&value.to_bits().to_le_bytes());
            }
        }
        for seat in SEATS {
            feed(seat.id.as_bytes());
            for value in seat.position.to_array() {
                feed(&value.to_bits().to_le_bytes());
            }
        }
        hash
    }

    #[test]
    fn visual_pass_preserves_exact_layout() {
        let scene = Theater::new();
        assert_eq!(
            scene.assets.meshes.len(),
            44 + CHAIR_MESH_COUNT + PROJECTOR_MESH_COUNT
        );
        assert_eq!(scene.colliders.len(), 10 + SEAT_COUNT);
        // Layout baseline includes the fitted posters and 30% larger entrance
        // door with 15cm wall clearance; all other geometry and collision stay fixed.
        assert_eq!(layout_fingerprint(&scene), 12_294_538_374_175_280_347);
    }

    fn eye(x: f32, z: f32) -> Vec3 {
        grounded_eye(Vec3::new(x, 0.0, z))
    }

    fn textured_mesh(scene: &Theater, texture: SpriteHandle) -> usize {
        let instance = scene
            .instances
            .iter()
            .find(|instance| instance.texture == Some(texture))
            .unwrap();
        scene
            .assets
            .meshes
            .iter()
            .position(|mesh| mesh.handle == instance.mesh)
            .unwrap()
    }

    #[test]
    fn seat_catalog_has_floor_bases_physical_bounds_and_grounded_exits() {
        assert_eq!(SEATS.len(), SEAT_COUNT);
        assert_eq!(SEAT_COUNT, 24);
        let mut ids = std::collections::HashSet::new();
        for (index, chair) in SEATS.into_iter().enumerate() {
            assert!(ids.insert(chair.id));
            let (rear, number) = match index {
                0..4 => (true, index + 1),
                4..8 => (false, index - 3),
                8..16 => (true, index - 3),
                _ => (false, index - 11),
            };
            let x = [
                -10.5, -8.75, -4.75, -3.0, -12.25, -14.0, -15.75, -17.5, -1.25, 0.5, 2.25, 4.0,
            ][number - 1];
            let z = if rear { -9.5 } else { -17.5 };
            let floor = if rear { 2.4 } else { 1.2 };
            assert_eq!(
                chair.id,
                format!("seat_{}{}", if rear { 'a' } else { 'b' }, number)
            );
            assert_eq!(seat_index(chair.id), Some(index as u8));
            assert_eq!(seat(chair.id), Some(chair));
            assert_eq!(chair.position, Vec3::new(x, floor, z));
            assert_eq!(chair.position.y, floor_height(chair.position));
            assert_eq!(chair.seated_eye(), Vec3::new(x, floor + 1.25, z));
            assert!(chair.approach().abs_diff_eq(
                Vec3::new(
                    x,
                    floor + crate::EYE_HEIGHT,
                    if rear { -10.6 } else { -16.4 }
                ),
                0.00001,
            ));
            assert_eq!(
                npc_waypoint(&format!("{}_approach", chair.id)),
                Some(chair.approach())
            );
            let (min, max) = chair.bounds();
            assert!((max - min).abs_diff_eq(Vec3::new(0.9, 0.95, 0.9), 0.00001));
            assert_eq!(min.y, floor);
            assert!(chair.seated_eye().y > max.y + crate::view::CAMERA_CLEARANCE);
            for x in [min.x, max.x] {
                for z in [min.z, max.z] {
                    assert_eq!(floor_height(Vec3::new(x, 0.0, z)), floor);
                }
            }
        }
        assert!(seat("").is_none());
        assert!(seat("seat_a0").is_none());
        assert!(seat_index("seat_b13").is_none());
        for z in [-9.5, -17.5] {
            let mut row: Vec<_> = SEATS.iter().filter(|seat| seat.position.z == z).collect();
            assert_eq!(row.len(), 12);
            row.sort_by(|a, b| a.position.x.total_cmp(&b.position.x));
            assert_eq!(
                row.iter()
                    .filter(|seat| seat.position.x < SCREEN_CENTER_X)
                    .count(),
                6
            );
            assert_eq!(
                row.iter()
                    .filter(|seat| seat.position.x > SCREEN_CENTER_X)
                    .count(),
                6
            );
            assert!((row[6].bounds().0.x - row[5].bounds().1.x - 3.1).abs() < 0.00001);
            for pair in row.windows(2) {
                assert!(pair[0].bounds().1.x < pair[1].bounds().0.x);
            }
        }
    }

    #[test]
    fn chair_meshes_fill_their_bounds_with_low_backs_and_auditorium_lighting() {
        let mut scene = Theater::new();
        assert_eq!(scene.colliders.len(), 10 + SEATS.len());
        assert_eq!(
            scene.assets.meshes.len(),
            44 + CHAIR_MESH_COUNT + PROJECTOR_MESH_COUNT
        );
        assert_eq!(
            scene.instances.len(),
            44 + CHAIR_MESH_COUNT + PROJECTOR_MESH_COUNT
        );
        for (index, chair) in SEATS.into_iter().enumerate() {
            let mesh = &scene.assets.meshes[44 + index];
            let instance = &scene.instances[44 + index];
            let collider = &scene.colliders[10 + index];
            let (min, max) = chair.bounds();
            assert_eq!(mesh.vertices.len(), 5 * 24);
            assert_eq!(mesh.indices.len(), 5 * 36);
            let mut mesh_min = Vec3::splat(f32::INFINITY);
            let mut mesh_max = Vec3::splat(f32::NEG_INFINITY);
            for vertex in &mesh.vertices {
                let point = Vec3::from(vertex.position);
                mesh_min = mesh_min.min(point);
                mesh_max = mesh_max.max(point);
                assert!(point.y >= floor_height(point));
                if (point.y - max.y).abs() < 0.00001 {
                    assert!(point.z >= chair.position.z + 0.30 - 0.00001);
                }
            }
            assert!(mesh_min.abs_diff_eq(min, 0.00001));
            assert!(mesh_max.abs_diff_eq(max, 0.00001));
            assert!(mesh.vertices.iter().any(|vertex| {
                (vertex.position[1] - chair.position.y - SEAT_HEIGHT).abs() < 0.00001
            }));
            assert_eq!(collider.min, Vec2::new(min.x, min.z));
            assert_eq!(collider.max, Vec2::new(max.x, max.z));
            assert_eq!(collider.height, max.y);
            assert_eq!(instance.mesh, mesh.handle);
            assert_eq!(instance.transform, RenderTransform::IDENTITY);
            assert_eq!(
                instance.texture,
                Some(scene.assets.textures[SEAT_TEXTURE_INDEX].handle)
            );
            assert_eq!(instance.emissive, 0.0);
            let light = instance.lighting.unwrap();
            assert!(in_auditorium(light.position));
            assert!((light.ambient + light.intensity - AUDITORIUM_LIGHT_LEVEL).abs() < 0.00001);
        }
        for level in [0.0, 0.6, 1.0] {
            apply_room_lighting(
                &mut scene.instances,
                1.0,
                level,
                scene.assets.textures[4].handle,
            );
            for instance in &scene.instances[44..44 + CHAIR_MESH_COUNT] {
                let light = instance.lighting.unwrap();
                assert_eq!(light.ambient, AUDITORIUM_AMBIENT_SHARE * level);
                assert_eq!(light.intensity, (1.0 - AUDITORIUM_AMBIENT_SHARE) * level);
            }
        }
    }

    #[test]
    fn chair_aisles_and_exits_preserve_entry_ramps_and_authored_npc_routes() {
        let scene = Theater::new();
        // Construction validates every resident anchor and complete authored leg,
        // not just selected time samples or the named route endpoints.
        let _npcs = crate::npcs::NpcRuntime::new(&scene.colliders);
        for chair in SEATS {
            let approach = chair.approach();
            let route = [
                SPAWN,
                npc_waypoint("lobby_aisle").unwrap(),
                npc_waypoint("passage_lobby").unwrap(),
                npc_waypoint("corridor_turn").unwrap(),
                npc_waypoint("auditorium_entry").unwrap(),
                eye(SCREEN_CENTER_X, -10.6),
                eye(SCREEN_CENTER_X, approach.z),
                approach,
            ];
            for pair in route.windows(2) {
                for (from, to) in [(pair[0], pair[1]), (pair[1], pair[0])] {
                    assert!(
                        npc_path_clear(from, to, &scene.colliders),
                        "{}: {from:?} -> {to:?}",
                        chair.id
                    );
                    assert!(resolve_movement(from, to, &scene.colliders).abs_diff_eq(to, 0.0001));
                }
            }
            assert!(has_line_of_sight(
                approach,
                chair.seated_eye(),
                &scene.colliders
            ));
        }
        assert!(!npc_path_clear(
            eye(8.0, -10.5),
            eye(-6.75, -6.0),
            &scene.colliders
        ));
        for (from, to) in [
            (eye(8.0, -10.5), eye(8.0, -7.5)),
            (eye(8.0, -7.5), eye(-6.75, -6.0)),
            (eye(8.0, -19.0), eye(-17.0, -19.0)),
            (eye(8.0, -10.5), eye(8.0, -26.0)),
            (eye(SCREEN_CENTER_X, -6.0), eye(SCREEN_CENTER_X, -26.0)),
        ] {
            for (from, to) in [(from, to), (to, from)] {
                assert!(npc_path_clear(from, to, &scene.colliders));
                assert!(resolve_movement(from, to, &scene.colliders).abs_diff_eq(to, 0.0001));
            }
        }
        for z in [-9.5, -17.5] {
            for x in [-9.625, SCREEN_CENTER_X, -3.875] {
                for (from, to) in [(z - 1.1, z + 1.1), (z + 1.1, z - 1.1)] {
                    assert!(npc_path_clear(eye(x, from), eye(x, to), &scene.colliders));
                    assert!(
                        resolve_movement(eye(x, from), eye(x, to), &scene.colliders)
                            .abs_diff_eq(eye(x, to), 0.0001)
                    );
                }
            }
        }
    }

    #[test]
    fn chair_footprints_block_walking_in_both_directions_without_tunneling() {
        let scene = Theater::new();
        for chair in SEATS {
            let (min, max) = chair.bounds();
            for (from, to, stop) in [
                (min.z - 1.0, max.z + 1.0, min.z - PLAYER_RADIUS),
                (max.z + 1.0, min.z - 1.0, max.z + PLAYER_RADIUS),
            ] {
                let start = eye(chair.position.x, from);
                let end = eye(chair.position.x, to);
                assert!(!npc_path_clear(start, end, &scene.colliders));
                assert!(
                    resolve_movement(start, end, &scene.colliders)
                        .abs_diff_eq(eye(chair.position.x, stop), 0.0001)
                );
            }
            for (from, to, stop) in [
                (min.x - 0.5, max.x + 0.5, min.x - PLAYER_RADIUS),
                (max.x + 0.5, min.x - 0.5, max.x + PLAYER_RADIUS),
            ] {
                let start = eye(from, chair.position.z);
                let end = eye(to, chair.position.z);
                assert!(!npc_path_clear(start, end, &scene.colliders));
                assert!(
                    resolve_movement(start, end, &scene.colliders)
                        .abs_diff_eq(eye(stop, chair.position.z), 0.0001)
                );
            }
            assert!(!npc_path_clear(
                chair.approach(),
                grounded_eye(chair.position),
                &scene.colliders
            ));
        }
    }

    #[test]
    fn chair_visibility_and_camera_sweeps_use_finite_world_space_tops() {
        let scene = Theater::new();
        let radius = crate::view::CAMERA_CLEARANCE;
        for chair in SEATS {
            let (min, max) = chair.bounds();
            let low_y = chair.position.y + 0.7;
            for (from, to, stop) in [
                (min.z - 0.6, max.z + 0.6, min.z - radius),
                (max.z + 0.6, min.z - 0.6, max.z + radius),
            ] {
                let low_start = Vec3::new(chair.position.x, low_y, from);
                let low_end = Vec3::new(chair.position.x, low_y, to);
                assert!(!has_line_of_sight(low_start, low_end, &scene.colliders));
                assert!(
                    clip_camera(low_start, low_end, radius, &scene.colliders)
                        .abs_diff_eq(Vec3::new(chair.position.x, low_y, stop), 0.00001)
                );
                let sight_start = Vec3::new(chair.position.x, max.y + 0.01, from);
                let sight_end = Vec3::new(chair.position.x, max.y + 0.01, to);
                assert!(has_line_of_sight(sight_start, sight_end, &scene.colliders));
                let high_start = sight_start + Vec3::Y * radius;
                let high_end = sight_end + Vec3::Y * radius;
                assert!(
                    clip_camera(high_start, high_end, radius, &scene.colliders)
                        .abs_diff_eq(high_end, 0.00001)
                );
            }
            let seated = chair.seated_eye();
            let down = clip_camera(seated, chair.position, radius, &scene.colliders);
            assert!(down.abs_diff_eq(Vec3::new(seated.x, max.y + radius, seated.z), 0.00001));
            assert_eq!(
                clip_camera(seated, seated, radius, &scene.colliders),
                seated
            );
            assert_eq!(
                clip_camera(seated, seated + Vec3::Y, radius, &scene.colliders),
                seated + Vec3::Y
            );
        }
    }

    #[test]
    fn every_seated_eye_has_clear_static_sight_lines_to_the_cinema_surface() {
        let scene = Theater::new();
        assert_eq!(SCREEN_TARGET.z, -27.91);
        for chair in SEATS {
            for x in [
                SCREEN_CENTER_X - SCREEN_WIDTH * 0.5,
                SCREEN_CENTER_X,
                SCREEN_CENTER_X + SCREEN_WIDTH * 0.5,
            ] {
                for y in [SCREEN_BOTTOM, SCREEN_TARGET.y, SCREEN_TOP] {
                    let target = Vec3::new(x, y, SCREEN_TARGET.z);
                    assert!(
                        has_line_of_sight(chair.seated_eye(), target, &scene.colliders),
                        "{}: screen {target:?}",
                        chair.id
                    );
                }
            }
        }
    }

    #[test]
    fn rendered_ceilings_and_lintel_match_shared_profiles_and_lights_stay_below() {
        let scene = Theater::new();
        let ceilings: Vec<_> = scene
            .assets
            .meshes
            .iter()
            .filter(|mesh| mesh.vertices[0].normal[1] < -0.9)
            .collect();
        assert_eq!(ceilings.len(), 4);
        for mesh in ceilings {
            let center = mesh
                .vertices
                .iter()
                .map(|v| Vec3::from(v.position))
                .sum::<Vec3>()
                / 4.0;
            let profile = ceiling_profile(center);
            for vertex in &mesh.vertices {
                let p = Vec3::from(vertex.position);
                assert!((p.y - profile_height(p.z, profile)).abs() < 0.00001);
            }
            assert!((center.y - ceiling_height(center)).abs() < 0.00001);
            if in_auditorium(center) {
                assert_eq!(center.y, HEIGHT);
            } else {
                assert!((center.y - floor_height(center) - LOW_CEILING_CLEARANCE).abs() < 0.00001);
            }
        }
        let lintel = &scene.assets.meshes[25];
        assert_eq!(lintel.vertices[0].normal, Vec3::NEG_X.to_array());
        assert_eq!(
            lintel.vertices[0].position,
            [PASSAGE_MIN, 6.9, SIDE_ENTRY_MIN_Z]
        );
        assert_eq!(
            lintel.vertices[2].position,
            [PASSAGE_MIN, HEIGHT, SIDE_ENTRY_MAX_Z]
        );
        for instance in &scene.instances {
            if instance.texture == Some(scene.assets.textures[4].handle) {
                continue;
            }
            let light = instance.lighting.unwrap();
            assert!((light.position.y - (ceiling_height(light.position) - 0.3)).abs() < 0.00001);
        }
    }

    #[test]
    fn local_ceiling_camera_sweeps_cover_raised_hallway_ramp_and_horizontal_lintel() {
        let scene = Theater::new();
        let radius = crate::view::CAMERA_CLEARANCE;
        for (x, z, height) in [
            (0.0, 5.0, 4.5),
            (10.75, -20.0, 6.9),
            (10.75, -4.5, 5.7),
            (0.0, -10.5, HEIGHT),
        ] {
            let start = eye(x, z);
            let result = clip_camera(start, Vec3::new(x, 20.0, z), radius, &scene.colliders);
            let expected = height
                - radius
                - if x > PASSAGE_MIN && z > SIDE_ENTRY_MAX_Z {
                    radius * REAR_PLATFORM_HEIGHT / -SIDE_ENTRY_MAX_Z
                } else {
                    0.0
                };
            assert!(
                (result.y - expected).abs() < 0.00001,
                "{result:?} != {expected}"
            );
        }
        let start = Vec3::new(8.0, 8.0, -10.5);
        let end = Vec3::new(11.0, 8.0, -10.5);
        let result = clip_camera(start, end, radius, &scene.colliders);
        assert!((result.x - (PASSAGE_MIN - radius)).abs() < 0.00001);
        assert_eq!(result.y, start.y);
        let ramp = clip_camera(
            Vec3::new(10.75, 6.0, -10.5),
            Vec3::new(10.75, 6.0, -1.0),
            radius,
            &scene.colliders,
        );
        let expected_z =
            -(6.0 + radius - LOW_CEILING_CLEARANCE) * 9.0 / REAR_PLATFORM_HEIGHT - radius;
        assert!((ramp.z - expected_z).abs() < 0.00001);
        for (start, end) in [
            (eye(8.0, -10.5), eye(11.0, -10.5)),
            (eye(11.0, -10.5), eye(8.0, -10.5)),
        ] {
            assert_eq!(clip_camera(start, end, radius, &scene.colliders), end);
            assert!(npc_path_clear(start, end, &scene.colliders));
            assert!(resolve_movement(start, end, &scene.colliders).abs_diff_eq(end, 0.0001));
        }
    }

    #[test]
    fn visibility_blocks_ceilings_and_lintel_but_preserves_normal_entry() {
        let scene = Theater::new();
        for (start, end) in [
            (Vec3::new(0.0, 4.0, 5.0), Vec3::new(0.0, 5.0, 5.0)),
            (Vec3::new(10.75, 6.0, -20.0), Vec3::new(10.75, 7.0, -20.0)),
            (Vec3::new(10.75, 6.0, -10.5), Vec3::new(10.75, 6.0, -1.0)),
            (Vec3::new(8.0, 8.0, -10.5), Vec3::new(11.0, 8.0, -10.5)),
            (Vec3::new(0.0, 11.0, -10.5), Vec3::new(0.0, 13.0, -10.5)),
        ] {
            assert!(!has_line_of_sight(start, end, &scene.colliders));
            assert!(!has_line_of_sight(end, start, &scene.colliders));
        }
        for (start, end) in [
            (eye(8.0, -10.5), eye(11.0, -10.5)),
            (Vec3::new(8.0, 6.8, -10.5), Vec3::new(11.0, 6.8, -10.5)),
            (eye(10.75, 3.0), eye(10.75, -5.0)),
            (Vec3::new(0.0, 11.0, -10.5), Vec3::new(8.0, 11.0, -10.5)),
        ] {
            assert!(has_line_of_sight(start, end, &scene.colliders));
            assert!(has_line_of_sight(end, start, &scene.colliders));
        }
    }

    #[test]
    fn camera_sweep_stops_before_walls_floor_ceiling_and_removed_counter_is_clear() {
        let scene = Theater::new();
        let radius = crate::view::CAMERA_CLEARANCE;
        let south = clip_camera(eye(0.0, 8.0), eye(0.0, 14.0), radius, &scene.colliders);
        assert!((south.z - (MAX_Z - radius)).abs() < 0.0001);
        let divider = clip_camera(eye(0.0, 3.0), eye(0.0, -3.0), radius, &scene.colliders);
        assert!((divider.z - radius).abs() < 0.0001);
        let passage = clip_camera(eye(9.5, 3.0), eye(9.5, -3.0), radius, &scene.colliders);
        assert_eq!(passage, eye(9.5, -3.0));
        let start = eye(0.0, 5.0);
        for (desired_y, expected_y) in [(-5.0, radius), (20.0, LOW_CEILING_CLEARANCE - radius)] {
            let result = clip_camera(
                start,
                Vec3::new(0.0, desired_y, 5.0),
                radius,
                &scene.colliders,
            );
            assert!((result.y - expected_y).abs() < 0.0001);
        }
        let above = clip_camera(eye(-5.0, 3.0), eye(-5.0, 0.5), radius, &scene.colliders);
        assert_eq!(above, eye(-5.0, 0.5));
        let down = clip_camera(
            eye(-5.0, 1.5),
            Vec3::new(-5.0, 0.0, 1.5),
            radius,
            &scene.colliders,
        );
        assert!((down.y - radius).abs() < 0.0001);
        let tangent = clip_camera(eye(0.0, radius), eye(1.0, radius), radius, &scene.colliders);
        assert_eq!(tangent, eye(0.0, radius));
    }

    #[test]
    fn sight_uses_physical_planes_passage_and_actual_obstacle_heights() {
        let scene = Theater::new();
        assert_eq!(
            scene
                .colliders
                .iter()
                .filter(|collider| collider.height == HEIGHT)
                .count(),
            10
        );
        assert_eq!(
            scene
                .colliders
                .iter()
                .filter(|collider| collider.height < HEIGHT)
                .count(),
            SEATS.len()
        );
        assert!(!has_line_of_sight(
            eye(0.0, 3.0),
            eye(0.0, -2.0),
            &scene.colliders
        ));
        // These rays are inside the physical passage but outside its narrower
        // player-radius clearance. Visibility must not reuse expanded bounds.
        for x in [PASSAGE_MIN + 0.0001, 9.5, PASSAGE_MAX - 0.0001] {
            assert!(has_line_of_sight(
                eye(x, 3.0),
                eye(x, -2.0),
                &scene.colliders
            ));
            assert!(has_line_of_sight(
                eye(x, -2.0),
                eye(x, 3.0),
                &scene.colliders
            ));
        }
        assert!(has_line_of_sight(
            eye(-10.0, 1.5),
            eye(0.5, 1.5),
            &scene.colliders
        ));
        assert!(has_line_of_sight(
            Vec3::new(-10.0, 0.5, 1.5),
            Vec3::new(0.5, 0.5, 1.5),
            &scene.colliders
        ));
        // Retain low-obstacle height coverage independently of the removed counter.
        let low = [Collider {
            min: Vec2::new(-9.0, 1.0),
            max: Vec2::new(-1.0, 2.2),
            height: 1.0,
        }];
        assert!(has_line_of_sight(eye(-10.0, 1.5), eye(0.5, 1.5), &low));
        assert!(!has_line_of_sight(
            Vec3::new(-10.0, 0.5, 1.5),
            Vec3::new(0.5, 0.5, 1.5),
            &low
        ));
        assert!(!has_line_of_sight(
            Vec3::new(0.0, HEIGHT + 0.01, 2.0),
            Vec3::new(0.0, HEIGHT + 0.01, -2.0),
            &scene.colliders
        ));
    }

    #[test]
    fn segment_slab_test_handles_parallel_coplanar_edges_and_endpoints() {
        let wall = Collider::new([-1.0, 0.0], [1.0, 0.0]);
        for (start, end, blocked) in [
            (eye(0.0, 1.0), eye(0.0, -1.0), true),
            (eye(0.0, 1.0), eye(0.0, 0.0), true),
            (eye(0.0, 0.0), eye(0.0, 1.0), true),
            (eye(-2.0, 0.0), eye(2.0, 0.0), true),
            (eye(-2.0, 0.001), eye(2.0, 0.001), false),
            (eye(1.0, 1.0), eye(1.0, -1.0), true),
            (eye(1.0001, 1.0), eye(1.0001, -1.0), false),
            (eye(0.0, 0.0), eye(0.0, 0.0), true),
            (eye(0.0, 0.001), eye(0.0, 0.001), false),
            (eye(0.0, 1.0e-30), eye(0.0, -1.0e-30), true),
            (eye(0.0, 1.0), eye(0.0, 0.0001), false),
            (
                Vec3::new(0.0, HEIGHT, 1.0),
                Vec3::new(0.0, HEIGHT, -1.0),
                true,
            ),
        ] {
            assert_eq!(
                wall.intersects_segment(start, end),
                blocked,
                "{start:?} -> {end:?}"
            );
            assert_eq!(
                wall.intersects_segment(end, start),
                blocked,
                "reversed: {end:?} -> {start:?}"
            );
        }
        let side_wall = Collider::new([0.0, -1.0], [0.0, 1.0]);
        assert!(side_wall.intersects_segment(eye(-1.0, 0.0), eye(1.0, 0.0)));
        assert!(!has_line_of_sight(Vec3::NAN, eye(0.0, 0.0), &[]));
    }

    #[test]
    fn room_lights_are_independent_including_the_two_divider_faces() {
        let mut scene = Theater::new();
        for (index, position, auditorium) in [
            (0, Vec3::new(0.0, 4.2, 5.0), false),
            (1, Vec3::new(-6.75, 11.7, -14.0), true),
            (6, Vec3::new(10.75, 6.6, -14.0), false),
            (18, Vec3::new(-6.75, 11.7, -14.0), true),
            (19, Vec3::new(0.0, 4.2, 5.0), false),
            (20, Vec3::new(-6.75, 11.7, -14.0), true),
            (21, Vec3::new(-6.75, 11.7, -14.0), true),
            (22, Vec3::new(10.75, 6.6, -14.0), false),
            (23, Vec3::new(-6.75, 11.7, -14.0), true),
            (24, Vec3::new(10.75, 6.6, -14.0), false),
            (25, Vec3::new(-6.75, 11.7, -14.0), true),
        ] {
            let light = scene.instances[index].lighting.unwrap();
            assert!(light.position.abs_diff_eq(position, 0.000_001));
            let expected = if auditorium {
                AUDITORIUM_LIGHT_LEVEL
            } else {
                LOBBY_LIGHT_LEVEL
            };
            assert!((light.ambient + light.intensity - expected).abs() < 1e-6);
            assert_eq!(scene.instances[index].emissive, 0.0);
        }
        let lobby_before = scene.instances[0].lighting;
        apply_room_lighting(
            &mut scene.instances,
            LOBBY_LIGHT_LEVEL,
            0.0,
            scene.assets.textures[4].handle,
        );
        assert_eq!(scene.instances[0].lighting, lobby_before);
        let dark = scene.instances[1].lighting.unwrap();
        assert_eq!(dark.ambient, 0.0);
        assert_eq!(dark.intensity, 0.0);
        apply_room_lighting(
            &mut scene.instances,
            0.25,
            1.0,
            scene.assets.textures[4].handle,
        );
        let auditorium = scene.instances[1].lighting.unwrap();
        assert_eq!(auditorium.ambient, AUDITORIUM_AMBIENT_SHARE);
        assert_eq!(auditorium.intensity, 1.0 - AUDITORIUM_AMBIENT_SHARE);
    }

    #[test]
    fn video_surface_is_centered_cinematic_with_full_uvs_unlit_and_opt_in_fit() {
        let mut scene = Theater::new();
        let screen_index = textured_mesh(&scene, scene.assets.textures[4].handle);
        let screen = &scene.assets.meshes[screen_index];
        let bottom_left = Vec3::from(screen.vertices[0].position);
        let top_right = Vec3::from(screen.vertices[2].position);
        let extent = top_right - bottom_left;
        assert!((extent.x / extent.y - SCREEN_ASPECT).abs() < 1e-6);
        assert!((f32::midpoint(bottom_left.x, top_right.x) - SCREEN_CENTER_X).abs() < 0.00001);
        assert!((extent.x - SCREEN_WIDTH).abs() < 0.00001);
        assert_eq!(extent.y, 10.6875);
        assert_eq!(bottom_left.y, 0.65);
        assert_eq!(top_right.y, 11.3375);
        assert_eq!(bottom_left.x, SCREEN_CENTER_X - SCREEN_WIDTH * 0.5);
        assert_eq!(top_right.x, SCREEN_CENTER_X + SCREEN_WIDTH * 0.5);
        assert_eq!(bottom_left.z, -27.91);
        assert!(top_right.y < HEIGHT);
        for (vertex, uv) in
            screen
                .vertices
                .iter()
                .zip([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]])
        {
            assert_eq!(vertex.uv, uv);
            assert_eq!(vertex.color, [1.0; 4]);
        }
        let before = scene.instances[screen_index].clone();
        assert_eq!(before.texture, Some(scene.assets.textures[4].handle));
        assert_eq!(before.transform, RenderTransform::IDENTITY);
        assert_eq!(
            before.texture_fit,
            Some(MeshTextureFit {
                surface_aspect: SCREEN_ASPECT,
                content_aspect: DEFAULT_VIDEO_ASPECT,
            })
        );
        for (index, instance) in scene.instances.iter().enumerate() {
            if index != screen_index && instance.render_mode == MeshRenderMode::Surface {
                assert!(instance.texture_fit.is_none());
            }
        }
        assert_eq!(before.lighting.unwrap().ambient, 1.0);
        assert_eq!(before.lighting.unwrap().intensity, 0.0);
        assert_eq!(before.emissive, 0.0);
        for levels in [(0.0, 0.0), (1.0, 1.0), (0.25, 0.8)] {
            apply_room_lighting(
                &mut scene.instances,
                levels.0,
                levels.1,
                scene.assets.textures[4].handle,
            );
            assert_eq!(scene.instances[screen_index], before);
        }
        assert_eq!(scene.assets.textures[4].rgba, [0, 0, 0, 255]);
        assert_eq!(
            (
                scene.assets.textures[4].width,
                scene.assets.textures[4].height
            ),
            (1, 1)
        );
        assert_eq!(scene.colliders.len(), 10 + SEATS.len());
    }

    #[test]
    fn screen_mock_is_black_and_spacious_with_no_emission() {
        let scene = Theater::new();
        let video_index = textured_mesh(&scene, scene.assets.textures[4].handle);
        let screen = &scene.assets.meshes[video_index - 1];
        let frame = &scene.assets.meshes[video_index - 2];
        for mesh in [screen, frame] {
            assert!(
                mesh.vertices
                    .iter()
                    .all(|v| v.color == [0.0, 0.0, 0.0, 1.0])
            );
            assert_eq!(
                scene
                    .instances
                    .iter()
                    .find(|i| i.mesh == mesh.handle)
                    .unwrap()
                    .emissive,
                0.0
            );
        }
        let left = SCREEN_CENTER_X - SCREEN_WIDTH * 0.5;
        let right = SCREEN_CENTER_X + SCREEN_WIDTH * 0.5;
        assert_eq!(
            screen.vertices[0].position,
            [left, SCREEN_BOTTOM, MIN_Z + 0.08]
        );
        assert_eq!(
            screen.vertices[2].position,
            [right, SCREEN_TOP, MIN_Z + 0.08]
        );
        let extent =
            Vec3::from(screen.vertices[2].position) - Vec3::from(screen.vertices[0].position);
        assert!((extent.x / extent.y - 2.39).abs() < 1.0e-6);
        assert!(left > AUDITORIUM_MIN.x && right < AUDITORIUM_MAX.x);
        assert!(Vec3::from(frame.vertices[0].position).abs_diff_eq(
            Vec3::new(left - 0.15, SCREEN_BOTTOM - 0.15, MIN_Z + 0.06),
            0.00001
        ));
        assert!(Vec3::from(frame.vertices[2].position).abs_diff_eq(
            Vec3::new(right + 0.15, SCREEN_TOP + 0.15, MIN_Z + 0.06),
            0.00001
        ));
        let video = &scene.assets.meshes[video_index];
        for (video, backing) in video.vertices.iter().zip(&screen.vertices) {
            assert_eq!(video.position[..2], backing.position[..2]);
            assert!(video.position[2] > backing.position[2]);
        }
        let fit = scene.instances[video_index].texture_fit.as_ref().unwrap();
        let size = contained_screen_size(*fit);
        assert!((size.x - 19.0).abs() < 0.00001);
        let center_x = f32::midpoint(video.vertices[0].position[0], video.vertices[2].position[0]);
        let left_margin = center_x - size.x * 0.5 - left;
        let right_margin = right - (center_x + size.x * 0.5);
        assert!(left_margin > 3.0);
        assert!((left_margin - right_margin).abs() < 0.00001);
        assert!(frame.vertices[2].position[1] < HEIGHT);
    }

    fn contained_screen_size(fit: MeshTextureFit) -> Vec2 {
        let fraction = if fit.content_aspect <= fit.surface_aspect {
            Vec2::new(fit.content_aspect / fit.surface_aspect, 1.0)
        } else {
            Vec2::new(1.0, fit.surface_aspect / fit.content_aspect)
        };
        Vec2::new(SCREEN_WIDTH, SCREEN_HEIGHT) * fraction
    }

    #[test]
    fn containment_metadata_preserves_source_ratio_with_pillar_or_letterboxing() {
        let mut scene = Theater::new();
        let texture = scene.assets.textures[4].handle;
        let index = textured_mesh(&scene, texture);
        let mut previous_aspect = DEFAULT_VIDEO_ASPECT;
        for aspect in [
            DEFAULT_VIDEO_ASPECT,
            4.0 / 3.0,
            9.0 / 16.0,
            2.39,
            21.0 / 9.0,
            32.0 / 9.0,
        ] {
            assert_eq!(
                fit_screen(&mut scene.instances, texture, aspect),
                aspect != previous_aspect
            );
            previous_aspect = aspect;
            assert!(!fit_screen(&mut scene.instances, texture, aspect));
            let fit = scene.instances[index].texture_fit.as_ref().unwrap();
            assert_eq!(fit.surface_aspect, SCREEN_ASPECT);
            assert_eq!(fit.content_aspect, aspect);
            let size = contained_screen_size(*fit);
            assert!((size.x / size.y - aspect).abs() < 0.00001);
            assert!(size.x <= SCREEN_WIDTH && size.y <= SCREEN_HEIGHT);
            let margins = (Vec2::new(SCREEN_WIDTH, SCREEN_HEIGHT) - size) * 0.5;
            if aspect < SCREEN_ASPECT {
                assert!(margins.x > 0.0);
                assert!(margins.y.abs() < 0.00001);
            } else if aspect > SCREEN_ASPECT {
                assert!(margins.y > 0.0);
                assert!(margins.x.abs() < 0.00001);
            } else {
                assert!(margins.abs().max_element() < 0.00001);
            }
        }
    }

    #[test]
    fn aspect_updates_only_change_video_fit_metadata_never_geometry_uvs_or_lighting() {
        let mut scene = Theater::new();
        let texture = scene.assets.textures[4].handle;
        let index = textured_mesh(&scene, texture);
        let before = scene.instances.clone();
        let vertices: Vec<_> = scene.assets.meshes[index]
            .vertices
            .iter()
            .map(|v| (v.position, v.normal, v.color, v.uv))
            .collect();
        let indices = scene.assets.meshes[index].indices.clone();
        for aspect in [
            4.0 / 3.0,
            9.0 / 16.0,
            SCREEN_ASPECT,
            32.0 / 9.0,
            DEFAULT_VIDEO_ASPECT,
        ] {
            assert!(fit_screen(&mut scene.instances, texture, aspect));
            let mut expected = before.clone();
            for instance in &mut expected {
                if instance.texture == Some(texture) && instance.texture_fit.is_some() {
                    instance.texture_fit = Some(MeshTextureFit {
                        surface_aspect: SCREEN_ASPECT,
                        content_aspect: aspect,
                    });
                }
            }
            assert_eq!(scene.instances, expected);
            assert_eq!(scene.assets.meshes[index].indices, indices);
            let after: Vec<_> = scene.assets.meshes[index]
                .vertices
                .iter()
                .map(|v| (v.position, v.normal, v.color, v.uv))
                .collect();
            assert_eq!(after, vertices);
        }
        assert_eq!(scene.instances, before);
    }

    #[test]
    fn invalid_aspects_and_missing_textures_preserve_the_last_valid_fit() {
        let mut scene = Theater::new();
        let texture = scene.assets.textures[4].handle;
        assert!(fit_screen(&mut scene.instances, texture, 4.0 / 3.0));
        let before = scene.instances.clone();
        for invalid in [0.0, -0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!fit_screen(&mut scene.instances, texture, invalid));
            assert_eq!(scene.instances, before);
        }
        assert!(!fit_screen(
            &mut scene.instances,
            SpriteHandle::new(),
            DEFAULT_VIDEO_ASPECT
        ));
        assert_eq!(scene.instances, before);
        assert!(!fit_screen(&mut [], texture, DEFAULT_VIDEO_ASPECT));
    }

    #[test]
    fn fit_screen_opts_in_and_repairs_all_matching_instances_only() {
        let scene = Theater::new();
        let texture = scene.assets.textures[4].handle;
        let index = textured_mesh(&scene, texture);
        let mut first = scene.instances[index].clone();
        first.texture_fit = None;
        let mut second = first.clone();
        second.texture_fit = Some(MeshTextureFit {
            surface_aspect: 1.0,
            content_aspect: DEFAULT_VIDEO_ASPECT,
        });
        let mut unrelated = scene.instances[0].clone();
        unrelated.texture_fit = Some(MeshTextureFit {
            surface_aspect: 1.0,
            content_aspect: 1.0,
        });
        let mut instances = vec![first, unrelated, second];
        let mut expected = instances.clone();
        for index in [0, 2] {
            expected[index].texture_fit = Some(MeshTextureFit {
                surface_aspect: SCREEN_ASPECT,
                content_aspect: DEFAULT_VIDEO_ASPECT,
            });
        }
        assert!(fit_screen(&mut instances, texture, DEFAULT_VIDEO_ASPECT));
        assert_eq!(instances, expected);
        assert!(!fit_screen(&mut instances, texture, DEFAULT_VIDEO_ASPECT));
    }

    #[test]
    fn labeled_dimensions_spawn_and_bounds_are_exact() {
        assert_eq!(LOBBY_MAX - LOBBY_MIN, Vec2::new(25.0, 10.0));
        assert_eq!(AUDITORIUM_MAX - AUDITORIUM_MIN, Vec2::new(31.5, 28.0));
        assert_eq!(HEIGHT, 12.0);
        assert_eq!(CORRIDOR_MAX - CORRIDOR_MIN, Vec2::new(3.5, 28.0));
        assert_eq!((SIDE_ENTRY_MIN_Z, SIDE_ENTRY_MAX_Z), (-12.0, -9.0));
        assert_eq!(SPAWN, eye(0.0, 8.0));
        assert_eq!(PASSAGE_MAX - PASSAGE_MIN, 3.5);
        assert_eq!(PASSAGE_MAX.midpoint(PASSAGE_MIN), 10.75);
        assert_eq!(
            Theater::camera_bounds(),
            CameraBounds::new(
                Vec3::new(-22.2, crate::EYE_HEIGHT, -27.7),
                Vec3::new(12.2, crate::EYE_HEIGHT + 2.4, 9.7),
            )
        );
    }

    #[test]
    fn seven_posters_fit_frames_preserve_ratios_and_stay_on_clear_lobby_walls() {
        let scene = Theater::new();
        assert_eq!(scene.colliders.len(), 10 + SEATS.len());
        let normals = [
            Vec3::X,
            Vec3::Z,
            Vec3::Z,
            Vec3::NEG_X,
            Vec3::NEG_X,
            Vec3::NEG_Z,
            Vec3::NEG_Z,
        ];
        let widths = [2.0, 3.6, 4.2, 4.0, 4.0, 4.8, 4.8];
        let mut wall_counts = [0; 4]; // North, east, south, west.
        let mut frame_bounds: Vec<(usize, Vec3, Vec3)> = Vec::new();
        for (index, ratio) in POSTER_RATIOS.into_iter().enumerate() {
            let texture = &scene.assets.textures[5 + index];
            assert!(texture.width.max(texture.height) <= 2048);
            let expected_sizes = [
                (1024, 1536),
                (749, 623),
                (1280, 720),
                (1280, 720),
                (2048, 1152),
                (2048, 1152),
                (2048, 1152),
            ];
            assert_eq!((texture.width, texture.height), expected_sizes[index]);
            assert_eq!(
                texture.rgba.len(),
                (texture.width * texture.height * 4) as usize
            );
            let art_index = textured_mesh(&scene, texture.handle);
            let frame_index = art_index - 1;
            let frame = &scene.assets.meshes[frame_index];
            let art = &scene.assets.meshes[art_index];
            assert_eq!(scene.instances[art_index].texture, Some(texture.handle));
            let points: Vec<_> = art
                .vertices
                .iter()
                .map(|v| Vec3::from_array(v.position))
                .collect();
            let width = points[0].distance(points[1]);
            let height = points[1].distance(points[2]);
            assert!((width / height - ratio).abs() < 0.00001);
            assert!((width - widths[index]).abs() < 0.00001);
            assert!(height <= 3.0 + 0.00001);
            let center_y = points[0].y.midpoint(points[2].y);
            let floor = floor_height(points[0]);
            let ceiling = ceiling_height(points[0]);
            assert!((center_y - floor.midpoint(ceiling)).abs() < 0.00001);
            assert!(
                (Vec3::from_array(frame.vertices[0].position)
                    .distance(Vec3::from_array(frame.vertices[1].position))
                    - width
                    - 0.16)
                    .abs()
                    < 0.00001
            );
            for (vertex, uv) in
                art.vertices
                    .iter()
                    .zip([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]])
            {
                assert_eq!(vertex.uv, uv);
                let p = Vec3::from_array(vertex.position);
                assert!(Vec3::from_array(vertex.normal).abs_diff_eq(normals[index], 0.00001));
                assert!(has_line_of_sight(
                    p + normals[index] * 2.0,
                    p,
                    &scene.colliders
                ));
            }
            let min = frame
                .vertices
                .iter()
                .fold(Vec3::splat(f32::INFINITY), |a, v| {
                    a.min(Vec3::from_array(v.position))
                });
            let max = frame
                .vertices
                .iter()
                .fold(Vec3::splat(f32::NEG_INFINITY), |a, v| {
                    a.max(Vec3::from_array(v.position))
                });
            assert!(min.y > floor && max.y < ceiling);
            assert!(((min.y - floor) - (ceiling - max.y)).abs() < 0.00001);
            assert!(min.x > -HALF_WIDTH && max.x < HALF_WIDTH);
            assert!(min.z > 0.0 && max.z < MAX_Z);
            let wall = if normals[index] == Vec3::Z {
                assert!((min.z - 0.025).abs() < 0.00001);
                assert!(max.x < PASSAGE_MIN);
                // Controls span x=-2.8..2.8. Leave a horizontal gap.
                assert!(max.x < -2.8 || min.x > 2.8);
                0
            } else if normals[index] == Vec3::NEG_X {
                assert!((max.x - (HALF_WIDTH - 0.025)).abs() < 0.00001);
                assert!(min.z > 0.5); // Keep the passage approach/corner clear.
                1
            } else if normals[index] == Vec3::NEG_Z {
                assert!((max.z - (MAX_Z - 0.025)).abs() < 0.00001);
                assert!(max.x < -1.2 || min.x > 1.2);
                2
            } else {
                assert!((min.x - (-HALF_WIDTH + 0.025)).abs() < 0.00001);
                assert!(min.z > 0.5);
                3
            };
            wall_counts[wall] += 1;
            for &(other_wall, other_min, other_max) in &frame_bounds {
                if wall == other_wall {
                    let axis = if wall == 1 || wall == 3 { 2 } else { 0 };
                    assert!(
                        max[axis] + 0.5 <= other_min[axis] || other_max[axis] + 0.5 <= min[axis]
                    );
                }
            }
            frame_bounds.push((wall, min, max));
            for (mesh, instance) in [
                (frame, &scene.instances[frame_index]),
                (art, &scene.instances[art_index]),
            ] {
                for vertex in &mesh.vertices {
                    let p = Vec3::from_array(vertex.position);
                    let plane = match wall {
                        0 => p.z,
                        1 => HALF_WIDTH - p.x,
                        2 => MAX_Z - p.z,
                        _ => p.x + HALF_WIDTH,
                    };
                    let expected = if instance.texture.is_some() {
                        0.04
                    } else {
                        0.025
                    };
                    assert!((plane - expected).abs() < 0.00001);
                }
                assert_eq!(instance.lighting.unwrap().ambient, LOBBY_AMBIENT_SHARE);
            }
        }
        assert_eq!(wall_counts, [2, 2, 2, 1]);
        assert_eq!(texture_assets()[5].handle, scene.assets.textures[5].handle);
    }

    #[test]
    fn floor_materials_are_distinct_and_every_mesh_texture_is_loaded() {
        let scene = Theater::new();
        assert_eq!(
            scene.assets.textures.len(),
            5 + POSTER_COUNT + MATERIAL_TEXTURE_COUNT
        );
        assert_eq!(
            scene.instances[0].texture,
            Some(scene.assets.textures[2].handle)
        );
        assert_eq!(
            scene.instances[1].texture,
            Some(scene.assets.textures[3].handle)
        );
        assert_ne!(scene.instances[0].texture, scene.instances[1].texture);
        for instance in &scene.instances {
            assert!(
                scene
                    .assets
                    .meshes
                    .iter()
                    .any(|mesh| mesh.handle == instance.mesh)
            );
            if let Some(texture) = instance.texture {
                assert!(
                    scene
                        .assets
                        .textures
                        .iter()
                        .any(|asset| asset.handle == texture)
                );
            }
        }
        assert_eq!(texture_assets()[0].handle, scene.assets.textures[0].handle);
    }

    #[test]
    fn geometry_has_ccw_normals_inward_walls_and_unmirrored_top_origin_uvs() {
        let scene = Theater::new();
        for mesh in &scene.assets.meshes {
            for triangle in mesh.indices.as_chunks::<3>().0 {
                let a = &mesh.vertices[usize::from(triangle[0])];
                let b = &mesh.vertices[usize::from(triangle[1])];
                let c = &mesh.vertices[usize::from(triangle[2])];
                let cross = (Vec3::from(b.position) - Vec3::from(a.position))
                    .cross(Vec3::from(c.position) - Vec3::from(a.position));
                assert!(cross.dot(Vec3::from(a.normal)) > 0.0);
            }
        }
        for (index, normal) in [
            (0, Vec3::Y),
            (1, Vec3::Y),
            (2, Vec3::new(0.0, 1.0, -0.3).normalize()),
            (3, Vec3::Y),
            (4, Vec3::new(0.0, 1.0, -0.3).normalize()),
            (5, Vec3::Y),
            (6, Vec3::Y),
            (7, Vec3::new(0.0, 1.0, 2.4 / 9.0).normalize()),
            (8, Vec3::NEG_Y),
            (9, Vec3::NEG_Y),
            (10, Vec3::NEG_Y),
            (11, Vec3::new(0.0, -1.0, -2.4 / 9.0).normalize()),
            (12, Vec3::NEG_Z),
            (13, Vec3::X),
            (14, Vec3::NEG_X),
            (15, Vec3::Z),
            (16, Vec3::X),
            (17, Vec3::NEG_X),
            (18, Vec3::NEG_Z),
            (19, Vec3::Z),
            (20, Vec3::NEG_Z),
            (21, Vec3::NEG_X),
            (22, Vec3::X),
            (23, Vec3::NEG_X),
            (24, Vec3::X),
            (25, Vec3::NEG_X),
        ] {
            assert!(
                Vec3::from(scene.assets.meshes[index].vertices[0].normal)
                    .abs_diff_eq(normal, 0.000_001)
            );
        }
        // Architectural meshes retain their order; chairs and the projector are appended.
        assert_eq!(
            scene.instances.len(),
            44 + CHAIR_MESH_COUNT + PROJECTOR_MESH_COUNT
        );
        assert_eq!(scene.colliders.len(), 10 + SEATS.len());
        for mesh in scene.assets.meshes.iter().filter(|mesh| {
            scene.instances.iter().any(|instance| {
                instance.mesh == mesh.handle
                    && instance.texture == Some(scene.assets.textures[0].handle)
            })
        }) {
            let height = mesh.vertices[3].position[1] - mesh.vertices[0].position[1];
            let start = Vec3::from(mesh.vertices[0].position);
            let end = Vec3::from(mesh.vertices[1].position);
            let direction = (end - start).normalize();
            let start_u = start.dot(direction) / 3.0;
            let end_u = end.dot(direction) / 3.0;
            assert_eq!(mesh.vertices[0].uv, [start_u, height / 3.0]);
            assert_eq!(mesh.vertices[3].uv, [start_u, 0.0]);
            assert_eq!(mesh.vertices[1].uv[0], end_u);
        }
        let door_index = textured_mesh(&scene, scene.assets.textures[1].handle);
        let door = &scene.assets.meshes[door_index];
        assert_eq!(door.vertices[0].uv, [0.0, 1.0]);
        assert_eq!(door.vertices[2].uv, [1.0, 0.0]);
        assert_eq!(door.vertices[0].position, [1.404, 0.0, 9.85]);
        assert_eq!(door.vertices[2].position, [-1.404, 4.212, 9.85]);
        let width = door.vertices[0].position[0] - door.vertices[2].position[0];
        let height = door.vertices[2].position[1];
        assert!((width / 2.16 - 1.30).abs() < 0.000_001);
        assert!((height / 3.24 - 1.30).abs() < 0.000_001);
        assert!((LOBBY_MAX.y - door.vertices[0].position[2] - 0.15).abs() < 0.000_001);
        assert!((ceiling_height(Vec3::new(0.0, 0.0, 9.85)) - height - 0.288).abs() < 0.000_001);
        assert!((width / height - 2.0 / 3.0).abs() < 0.000_001);
        assert_eq!(
            (
                scene.assets.textures[1].width,
                scene.assets.textures[1].height
            ),
            (1024, 1536)
        );
        for (index, mesh) in scene.assets.meshes[..8].iter().enumerate() {
            let origin = match index {
                1..=5 => AUDITORIUM_MIN,
                _ => LOBBY_MIN,
            };
            for vertex in &mesh.vertices {
                assert_eq!(
                    vertex.uv,
                    [
                        (vertex.position[0] - origin.x) / 4.0,
                        (vertex.position[2] - origin.y) / 4.0
                    ]
                );
            }
        }
    }

    #[test]
    fn final_materials_add_ceiling_and_chair_uvs_without_extra_meshes() {
        let scene = Theater::new();
        for mesh in &scene.assets.meshes[8..12] {
            let instance = scene
                .instances
                .iter()
                .find(|instance| instance.mesh == mesh.handle)
                .unwrap();
            assert_eq!(
                instance.texture,
                Some(scene.assets.textures[CEILING_TEXTURE_INDEX].handle)
            );
            assert!(
                mesh.vertices
                    .iter()
                    .any(|vertex| vertex.uv != mesh.vertices[0].uv)
            );
            assert!(
                mesh.vertices
                    .iter()
                    .all(|vertex| vertex.uv.iter().all(|v| v.is_finite()))
            );
        }
        for mesh in &scene.assets.meshes[44..44 + CHAIR_MESH_COUNT] {
            for (part, vertices) in mesh.vertices.as_chunks::<24>().0.iter().enumerate() {
                let quadrant = [2, 0, 1, 3, 3][part];
                let min = Vec2::new(
                    (SEAT_ATLAS_INSET + (quadrant % 2) as f32 * 512.0) / 1024.0,
                    (SEAT_ATLAS_INSET + (quadrant / 2) as f32 * 512.0) / 1024.0,
                );
                let max = min + Vec2::splat(SEAT_ATLAS_SPAN / 1024.0);
                for vertex in vertices {
                    let uv = Vec2::from(vertex.uv);
                    assert!(uv.cmpge(min).all() && uv.cmple(max).all());
                }
                for face in vertices.as_chunks::<4>().0 {
                    assert_ne!(face[0].uv, face[2].uv);
                }
            }
        }
        let doorway = Vec3::new(HALF_WIDTH, 0.0, 0.0).to_array();
        let lobby = scene.assets.meshes[0]
            .vertices
            .iter()
            .find(|v| v.position == doorway)
            .unwrap();
        let hallway = scene.assets.meshes[7]
            .vertices
            .iter()
            .find(|v| v.position == doorway)
            .unwrap();
        assert_eq!(lobby.uv, hallway.uv);
        assert_eq!(layout_fingerprint(&scene), 12_294_538_374_175_280_347);
    }

    #[test]
    fn entrance_partition_and_widened_exterior_walls_are_solid_without_counter() {
        let scene = Theater::new();
        for (start, desired, expected) in [
            (SPAWN, eye(0.0, 20.0), eye(0.0, 9.7)),
            (SPAWN, eye(0.0, -10.0), eye(0.0, 0.3)),
            (eye(-5.0, 8.0), eye(-5.0, 0.0), eye(-5.0, 0.3)),
            (eye(-5.0, 3.2), eye(-5.0, 0.5), eye(-5.0, 0.5)),
            (eye(9.5, -7.5), eye(20.0, -7.5), eye(12.2, -7.5)),
            (eye(-9.5, -7.5), eye(-40.0, -7.5), eye(-22.2, -7.5)),
            (eye(-17.0, -7.5), eye(-17.0, 8.0), eye(-17.0, -0.3)),
            (eye(10.75, -7.5), eye(10.75, -40.0), eye(10.75, -27.7)),
            (eye(0.0, 5.0), eye(20.0, 5.0), eye(12.2, 5.0)),
            (eye(0.0, 5.0), eye(-20.0, 5.0), eye(-12.2, 5.0)),
            (eye(6.0, -7.5), eye(6.0, 8.0), eye(6.0, -0.3)),
        ] {
            let actual = resolve_movement(start, desired, &scene.colliders);
            assert!(
                actual.abs_diff_eq(expected, 0.0001),
                "{start:?} -> {desired:?}: {actual:?}, expected {expected:?}"
            );
        }
    }

    #[test]
    fn passage_is_walkable_both_directions_but_edges_block() {
        let scene = Theater::new();
        for x in [9.4, 10.75, 12.1] {
            for (from, to) in [(8.0, -12.0), (-12.0, 8.0)] {
                assert!(
                    resolve_movement(eye(x, from), eye(x, to), &scene.colliders)
                        .abs_diff_eq(eye(x, to), 0.0001)
                );
            }
        }
        for x in [PASSAGE_MIN - 0.2, PASSAGE_MIN + 0.2] {
            assert!(
                resolve_movement(eye(x, 3.0), eye(x, -3.0), &scene.colliders).z >= PLAYER_RADIUS
            );
            assert!(
                resolve_movement(eye(x, -3.0), eye(x, 3.0), &scene.colliders).z <= -PLAYER_RADIUS
            );
        }
    }

    #[test]
    fn hallway_left_turn_is_the_only_auditorium_entry() {
        let scene = Theater::new();
        let route = [
            eye(10.75, 3.2),
            eye(10.75, -10.5),
            eye(8.0, -10.5),
            eye(8.0, -19.0),
            eye(-17.0, -19.0),
        ];
        for pair in route.windows(2) {
            for (from, to) in [(pair[0], pair[1]), (pair[1], pair[0])] {
                assert!(npc_path_clear(from, to, &scene.colliders));
                assert!(has_line_of_sight(from, to, &scene.colliders));
                assert!(resolve_movement(from, to, &scene.colliders).abs_diff_eq(to, 0.0001));
            }
        }
        for x in [-12.0, -10.0, 0.0, 8.5] {
            let from = eye(x, 0.5);
            let to = eye(x, -2.0);
            assert!(!npc_path_clear(from, to, &scene.colliders));
            assert!(!has_line_of_sight(from, to, &scene.colliders));
            assert!(resolve_movement(from, to, &scene.colliders).abs_diff_eq(eye(x, 0.3), 0.0001));
        }
        for z in [-20.0, -12.2, -8.8, -6.0, -3.0, -1.0] {
            assert!(!npc_path_clear(
                eye(10.75, z),
                eye(8.0, z),
                &scene.colliders
            ));
            assert!(!has_line_of_sight(
                eye(10.75, z),
                eye(8.0, z),
                &scene.colliders
            ));
            assert!(
                resolve_movement(eye(10.75, z), eye(8.0, z), &scene.colliders)
                    .abs_diff_eq(eye(9.3, z), 0.0001)
            );
        }
        for z in [-11.6, -10.5, -9.4] {
            assert!(npc_path_clear(eye(10.75, z), eye(8.0, z), &scene.colliders));
        }
        for z in [-11.8, -9.2] {
            assert!(!npc_path_clear(
                eye(10.75, z),
                eye(8.0, z),
                &scene.colliders
            ));
            assert!(has_line_of_sight(
                eye(10.75, z),
                eye(8.0, z),
                &scene.colliders
            ));
        }
    }

    #[test]
    fn floor_profiles_have_broad_flats_continuous_ramps_and_a_level_side_entry() {
        for (x, z, height) in [
            (-20.0, -26.0, 0.0),
            (7.0, -26.0, 0.0),
            (-20.0, -23.0, 0.6),
            (7.0, -19.0, 1.2),
            (-20.0, -19.0, 1.2),
            (-6.75, -14.0, 1.8),
            (-20.0, -6.0, 2.4),
            (7.0, -6.0, 2.4),
            (10.75, -4.5, 1.2),
            (10.75, -10.5, 2.4),
            (-9.5, 5.0, 0.0),
        ] {
            assert!((floor_height(Vec3::new(x, 1000.0, z)) - height).abs() < 0.00001);
        }
        for x in [-17.0, 8.0, 10.75] {
            for z in [-25.0, -21.0, -16.0, -12.0, -9.0] {
                let height = floor_height(Vec3::new(x, 0.0, z));
                for offset in [-0.00001, 0.00001] {
                    assert!((floor_height(Vec3::new(x, 0.0, z + offset)) - height).abs() < 0.00001);
                }
            }
        }
        for z in [SIDE_ENTRY_MIN_Z, -10.5, SIDE_ENTRY_MAX_Z] {
            for x in [8.0, 8.99, 9.0, 9.01, 10.75] {
                assert_eq!(floor_height(Vec3::new(x, 0.0, z)), REAR_PLATFORM_HEIGHT);
            }
        }
    }

    #[test]
    fn every_rendered_floor_piece_matches_ground_queries_and_keeps_uv_tiles_continuous() {
        let scene = Theater::new();
        assert_eq!(
            scene.assets.meshes.len(),
            44 + CHAIR_MESH_COUNT + PROJECTOR_MESH_COUNT
        );
        for (index, mesh) in scene.assets.meshes[..8].iter().enumerate() {
            for triangle in mesh.indices.as_chunks::<3>().0 {
                let points = triangle.map(|i| Vec3::from(mesh.vertices[usize::from(i)].position));
                for weights in [[0.2, 0.3, 0.5], [0.6, 0.3, 0.1], [0.1, 0.7, 0.2]] {
                    let point =
                        points[0] * weights[0] + points[1] * weights[1] + points[2] * weights[2];
                    assert!(
                        (point.y - floor_height(point)).abs() < 0.00001,
                        "floor {index}: {point:?}"
                    );
                }
            }
            if (1..=5).contains(&index) {
                assert_eq!(
                    scene.instances[index].texture,
                    Some(scene.assets.textures[3].handle)
                );
                let extent =
                    Vec3::from(mesh.vertices[2].position) - Vec3::from(mesh.vertices[0].position);
                assert_eq!(extent.x, 31.5);
            }
        }
        for pair in scene.assets.meshes[1..6].windows(2) {
            for (left, right) in [(1, 0), (2, 3)] {
                assert_eq!(
                    pair[0].vertices[left].position,
                    pair[1].vertices[right].position
                );
                assert_eq!(pair[0].vertices[left].uv, pair[1].vertices[right].uv);
            }
        }
    }

    #[test]
    fn waypoint_contract_is_grounded_and_concession_support_is_removed() {
        let scene = Theater::new();
        for (name, x, z) in [
            ("lobby_door", 0.0, 9.4),
            ("lobby_entry", 0.0, 8.0),
            ("lobby_center", 0.0, 5.0),
            ("lobby_west", -9.5, 5.0),
            ("lobby_east", 6.0, 5.0),
            ("lobby_aisle", 10.75, 3.2),
            ("passage_lobby", 10.75, 0.6),
            ("corridor_turn", 10.75, -10.5),
            ("auditorium_entry", 8.0, -10.5),
            ("auditorium_back", -6.75, -6.0),
            ("auditorium_left", -17.0, -19.0),
            ("auditorium_right", 4.0, -19.0),
            ("auditorium_center", -6.75, -19.0),
            ("row_a", 8.0, -10.0),
            ("row_b", 8.0, -19.0),
        ] {
            let point = npc_waypoint(name).unwrap();
            assert!(point.abs_diff_eq(eye(x, z), 0.00001), "{name}: {point:?}");
            assert!(npc_path_clear(point, point, &scene.colliders), "{name}");
        }
        assert!(npc_waypoint("concession_support").is_none());
        assert!(!npc_path_clear(
            eye(-17.0, 5.0),
            eye(-17.0, 5.0),
            &scene.colliders
        ));
        assert!(!npc_path_clear(
            eye(-6.75, -6.0) - Vec3::Y,
            eye(-6.75, -19.0),
            &scene.colliders
        ));
    }

    #[test]
    fn movement_ascends_descends_and_grounds_diagonals_and_large_frames() {
        let scene = Theater::new();
        for (from, to) in [
            (eye(-6.75, -26.0), eye(-6.75, -6.0)),
            (eye(6.0, -26.0), eye(8.0, -6.0)),
            (eye(10.75, 8.0), eye(10.75, -10.5)),
            (eye(9.4, 8.0), eye(12.1, -10.5)),
            (eye(10.75, -10.5), eye(-17.0, -10.5)),
        ] {
            for (start, end) in [(from, to), (to, from)] {
                assert!(npc_path_clear(start, end, &scene.colliders));
                let mut previous = start;
                for step in 1..=80 {
                    let mut desired = start.lerp(end, step as f32 / 80.0);
                    desired.y = -1000.0;
                    previous = resolve_movement(previous, desired, &scene.colliders);
                    assert!(previous.abs_diff_eq(grounded_eye(desired), 0.0001));
                }
                assert!(previous.abs_diff_eq(end, 0.0001));
                let mut desired = end;
                desired.y = 1000.0;
                assert!(
                    resolve_movement(start, desired, &scene.colliders).abs_diff_eq(end, 0.0001)
                );
            }
        }
    }

    #[test]
    fn controller_eye_and_aim_follow_tiers_in_both_directions() {
        use weaver_app_core::{FirstPersonController, InputFrame};
        use weaver_render::Camera;

        let scene = Theater::new();
        for (start_z, movement_y, end_z) in [(-26.0, -1.0, -4.0), (-4.0, 1.0, -26.0)] {
            let mut camera = Camera {
                eye: eye(-6.75, start_z),
                target: eye(-6.75, start_z) + Vec3::NEG_Z,
                ..Camera::default()
            };
            let mut controller = FirstPersonController::from_camera(&camera);
            controller.bounds = Some(Theater::camera_bounds());
            let previous = camera.eye;
            controller.update(
                &mut camera,
                &InputFrame {
                    movement: Vec2::new(0.0, movement_y),
                    ..InputFrame::default()
                },
                5.5,
            );
            let resolved = resolve_movement(previous, camera.eye, &scene.colliders);
            camera.target += resolved - camera.eye;
            camera.eye = resolved;
            assert!(camera.eye.abs_diff_eq(eye(-6.75, end_z), 0.0001));
            assert!((camera.target - camera.eye).abs_diff_eq(Vec3::NEG_Z, 0.000_001));
        }
    }

    #[test]
    fn third_person_camera_never_goes_under_tiers_or_through_intervening_ramps() {
        let scene = Theater::new();
        let radius = crate::view::CAMERA_CLEARANCE;
        for z in [-26.0, -23.0, -19.0, -14.0, -6.0] {
            let start = eye(-6.75, z);
            let desired = start + Vec3::new(0.0, -4.8, 3.6);
            let result = clip_camera(start, desired, radius, &scene.colliders);
            assert!(result.distance(start) < desired.distance(start));
            assert!(has_line_of_sight(start, result, &scene.colliders));
            for step in 0..=64 {
                let point = start.lerp(result, step as f32 / 64.0);
                for offset in [
                    Vec3::ZERO,
                    Vec3::new(-radius, 0.0, -radius),
                    Vec3::new(radius, 0.0, radius),
                ] {
                    assert!(
                        point.y >= floor_height(point + offset) + radius - 0.00001,
                        "{point:?}"
                    );
                }
            }
        }
        let start = Vec3::new(-6.75, 0.5, -27.0);
        let desired = Vec3::new(-6.75, 0.5, 1.0);
        assert!(start.y > floor_height(start) && desired.y > floor_height(desired));
        let result = clip_camera(start, desired, radius, &[]);
        assert!(result.abs_diff_eq(Vec3::new(-6.75, 0.5, -24.2), 0.00001));
        assert!(!has_line_of_sight(start, desired, &[]));
        let tangent = Vec3::new(-6.75, REAR_PLATFORM_HEIGHT + radius, -10.0);
        let end = tangent + Vec3::X;
        assert!(clip_camera(tangent, end, radius, &[]).abs_diff_eq(end, 0.00001));
    }

    #[test]
    fn view_rig_keeps_third_person_camera_above_each_tier_without_moving_player() {
        use weaver_app_core::InputFrame;
        use weaver_render::Camera;

        let scene = Theater::new();
        for z in [-26.0, -23.0, -19.0, -14.0, -6.0] {
            let player = Camera {
                eye: eye(-6.75, z),
                target: eye(-6.75, z) + Vec3::new(0.0, 0.8, -0.6),
                ..Camera::default()
            };
            let mut rig = crate::view::ViewRig::new(player.clone());
            rig.apply_input(&InputFrame {
                toggle_perspective: true,
                zoom_delta: -100.0,
                ..InputFrame::default()
            });
            assert!(rig.third_person());
            let camera = rig.camera(1.0, &scene.colliders);
            assert_eq!(rig.player.eye, player.eye);
            assert_eq!(rig.player.target, player.target);
            assert_eq!(camera.target, player.target);
            assert!(camera.eye.distance(player.eye) > 0.0);
            assert!(
                camera.eye.y >= floor_height(camera.eye) + crate::view::CAMERA_CLEARANCE - 0.00001
            );
            assert!(has_line_of_sight(player.eye, camera.eye, &scene.colliders));
        }
    }

    #[test]
    fn large_diagonal_movement_cannot_tunnel_and_slides_at_contact() {
        let scene = Theater::new();
        let result = resolve_movement(eye(6.0, -2.0), eye(50.0, -50.0), &scene.colliders);
        assert!(result.abs_diff_eq(eye(8.7, -27.7), 0.0001));
        let result = resolve_movement(eye(12.2, -7.5), eye(20.0, -12.0), &scene.colliders);
        assert!(result.abs_diff_eq(eye(12.2, -12.0), 0.0001));
    }
}
