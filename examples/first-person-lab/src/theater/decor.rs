//! Fixed, baked decoration: translucent sprites and one opaque floor rug.
//! No architectural geometry, collision, or real lights are changed.

use super::{
    AUDITORIUM_MAX, AUDITORIUM_MIN, CHAIR_HALF_WIDTH, CORRIDOR_MAX, HALF_WIDTH,
    LOBBY_AMBIENT_SHARE, LOBBY_LIGHT_LEVEL, LOBBY_MAX, LOBBY_MIN, MAX_Z, MIN_Z, POSTER_CENTER_Y,
    POSTER_RATIOS, SCREEN_BOTTOM, SCREEN_CENTER_X, SCREEN_FRAME_BORDER, SCREEN_TOP, SCREEN_WIDTH,
    SIDE_ENTRY_MAX_Z, SIDE_ENTRY_MIN_Z, ceiling_height, floor_height, floor_profile, seat,
};
use glam::{Mat3, Quat, Vec3};
use std::sync::OnceLock;
use weaver_app_core::{AppAssets, MeshAsset, TextureAsset};
use weaver_render::{
    MeshHandle, MeshInstance, MeshLighting, MeshRenderMode, RenderTransform, SpriteHandle,
    SpriteInstance, SpriteSpace,
};
use weaver_render_wgpu::Vertex;

#[cfg(test)]
pub(crate) const TEXTURE_COUNT: usize = 16;
const BORDER: f32 = 0.08;
const FRAME_DEPTH: f32 = 0.055;
const FLOOR_OFFSET: f32 = 0.012;

// Midpoint of the clear bay between the Godfather's right frame edge (-8.02)
// and Star Wars' left frame edge (3.12), aligned to their artwork centerline.
pub(crate) const PORTFOLIO_CENTER: Vec3 = Vec3::new(-2.45, POSTER_CENTER_Y, 0.04);
// The board occupies (270,100)..(1267,822) inside the 1536x870 lit texture.
// Maximize uniform scale with 5cm ceiling clearance, accounting for the lamp
// above the board rather than centering the transparent image canvas.
const PORTFOLIO_PIXEL_SCALE: f32 = (POSTER_CENTER_Y - 0.05) / 461.0;
const PORTFOLIO_HEIGHT: f32 = 870.0 * PORTFOLIO_PIXEL_SCALE;
const PORTFOLIO_IMAGE_CENTER: Vec3 = Vec3::new(
    PORTFOLIO_CENTER.x - 0.5 * PORTFOLIO_PIXEL_SCALE,
    PORTFOLIO_CENTER.y + 26.0 * PORTFOLIO_PIXEL_SCALE,
    PORTFOLIO_CENTER.z,
);
pub(crate) const PORTFOLIO_HALF_WIDTH: f32 = 997.0 * 0.5 * PORTFOLIO_PIXEL_SCALE;
pub(crate) const PORTFOLIO_BOTTOM: f32 = PORTFOLIO_CENTER.y - 361.0 * PORTFOLIO_PIXEL_SCALE;
pub(crate) const PORTFOLIO_TOP: f32 = PORTFOLIO_CENTER.y + 361.0 * PORTFOLIO_PIXEL_SCALE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Art {
    Sconce,
    Step,
    Star,
    CurtainLeft,
    CurtainRight,
    Rug,
    WallGlow,
    FloorGlow,
    Frame(usize),
    Portfolio,
}

struct Resource {
    art: Art,
    texture: TextureAsset,
}

fn resources() -> &'static [Resource] {
    static RESOURCES: OnceLock<Vec<Resource>> = OnceLock::new();
    RESOURCES.get_or_init(|| {
        [
            (
                Art::Sconce,
                include_bytes!("../../assets/environment/wall_sconce_weathered_amber.png")
                    .as_slice(),
            ),
            (
                Art::Step,
                include_bytes!("../../assets/environment/step_light_amber.png").as_slice(),
            ),
            (
                Art::Star,
                include_bytes!("../../assets/environment/star_marker_light.png").as_slice(),
            ),
            (
                Art::CurtainLeft,
                include_bytes!("../../assets/environment/curtain_left.png").as_slice(),
            ),
            (
                Art::CurtainRight,
                include_bytes!("../../assets/environment/curtain_right.png").as_slice(),
            ),
            (
                Art::Rug,
                include_bytes!("../../assets/environment/ground_compass_rug.png").as_slice(),
            ),
            (
                Art::WallGlow,
                include_bytes!("../../assets/environment/warm_glow.png").as_slice(),
            ),
            (
                Art::FloorGlow,
                include_bytes!("../../assets/environment/warm_floor_glow.png").as_slice(),
            ),
            (
                Art::Frame(0),
                include_bytes!("../../assets/environment/poster_frame_01.png").as_slice(),
            ),
            (
                Art::Frame(1),
                include_bytes!("../../assets/environment/poster_frame_02.png").as_slice(),
            ),
            (
                Art::Frame(2),
                include_bytes!("../../assets/environment/poster_frame_03.png").as_slice(),
            ),
            (
                Art::Frame(3),
                include_bytes!("../../assets/environment/poster_frame_04.png").as_slice(),
            ),
            (
                Art::Frame(4),
                include_bytes!("../../assets/environment/poster_frame_05.png").as_slice(),
            ),
            (
                Art::Frame(5),
                include_bytes!("../../assets/environment/poster_frame_06.png").as_slice(),
            ),
            (
                Art::Frame(6),
                include_bytes!("../../assets/environment/poster_frame_07.png").as_slice(),
            ),
            (
                Art::Portfolio,
                include_bytes!("../../assets/portfolio_wall/portfolio_wall.png").as_slice(),
            ),
        ]
        .into_iter()
        .map(|(art, png)| {
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                .expect("bundled environment PNG must decode")
                .into_rgba8();
            Resource {
                art,
                texture: TextureAsset {
                    handle: SpriteHandle::new(),
                    width: image.width(),
                    height: image.height(),
                    rgba: image.into_raw(),
                },
            }
        })
        .collect()
    })
}

fn texture(art: Art) -> &'static TextureAsset {
    &resources()
        .iter()
        .find(|resource| resource.art == art)
        .expect("complete bundled environment catalog")
        .texture
}

fn aspect(art: Art) -> f32 {
    let texture = texture(art);
    texture.width as f32 / texture.height as f32
}

fn wall_floor_height(base: Vec3, normal: Vec3) -> f32 {
    // Divider coordinates select the hallway profile; a fixture on its opposite
    // face must sample inside the auditorium, where floor heights can differ.
    floor_height(base + normal * 0.01)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Room {
    Lobby,
    Hallway,
    Auditorium,
}

#[derive(Clone, Copy)]
enum Response {
    Physical,
    // Baked lamps and low-alpha light pools remain legible with room lights off.
    Practical(f32),
}

struct Piece {
    sprite: SpriteInstance,
    room: Room,
    response: Response,
}

pub(crate) struct Decor {
    pieces: Vec<Piece>,
    rug: MeshInstance,
}

fn rug_mesh() -> &'static MeshAsset {
    static RUG: OnceLock<MeshAsset> = OnceLock::new();
    RUG.get_or_init(|| {
        let center = Vec3::new(0.0, 0.0, 5.0);
        let y = floor_height(center) + FLOOR_OFFSET;
        let vertices = [
            ([-1.8, y, center.z - 1.8], [0.0, 0.0]),
            ([-1.8, y, center.z + 1.8], [0.0, 1.0]),
            ([1.8, y, center.z + 1.8], [1.0, 1.0]),
            ([1.8, y, center.z - 1.8], [1.0, 0.0]),
        ]
        .map(|(position, uv)| Vertex {
            position,
            normal: Vec3::Y.to_array(),
            color: [1.0; 4],
            uv,
        })
        .to_vec();
        MeshAsset::new(MeshHandle::new(), vertices, vec![0, 1, 2, 0, 2, 3])
    })
}

impl Decor {
    pub(crate) fn new(assets: &mut AppAssets) -> Self {
        for resource in resources() {
            if !assets
                .textures
                .iter()
                .any(|asset| asset.handle == resource.texture.handle)
            {
                assets.textures.push(resource.texture.clone());
            }
        }
        let mesh = rug_mesh();
        if !assets
            .meshes
            .iter()
            .any(|asset| asset.handle == mesh.handle)
        {
            assets.meshes.push(mesh.clone());
        }
        // An opaque rug must write scene depth before avatars are drawn. Keeping
        // it in the center-sorted sprite pass can otherwise paint over their feet.
        let mut rug = MeshInstance::new(mesh.handle, RenderTransform::default());
        rug.render_mode = MeshRenderMode::Surface;
        rug.texture = Some(texture(Art::Rug).handle);
        let mut light_position = Vec3::new(
            LOBBY_MIN.x.midpoint(LOBBY_MAX.x),
            0.0,
            LOBBY_MIN.y.midpoint(LOBBY_MAX.y),
        );
        light_position.y = ceiling_height(light_position) - 0.3;
        rug.lighting = Some(MeshLighting {
            position: light_position,
            ambient: LOBBY_AMBIENT_SHARE * LOBBY_LIGHT_LEVEL,
            intensity: (1.0 - LOBBY_AMBIENT_SHARE) * LOBBY_LIGHT_LEVEL,
        });
        let mut decor = Self {
            pieces: Vec::new(),
            rug,
        };
        decor.posters();
        decor.curtains();
        decor.push(
            Art::Portfolio,
            RenderTransform {
                translation: PORTFOLIO_IMAGE_CENTER,
                scale: PORTFOLIO_HEIGHT,
                ..RenderTransform::default()
            },
            Room::Lobby,
            Response::Practical(0.85),
        );

        // Clear bays between the lobby artwork, entrance, and controls.
        for (position, normal) in [
            (Vec3::new(-HALF_WIDTH, 2.6, 1.6), Vec3::X),
            (Vec3::new(-HALF_WIDTH, 2.6, 8.4), Vec3::X),
            (Vec3::new(-2.9, 2.8, MAX_Z), Vec3::NEG_Z),
            (Vec3::new(2.9, 2.8, MAX_Z), Vec3::NEG_Z),
        ] {
            decor.sconce(position, normal, Room::Lobby);
        }
        for z in [-3.5, -10.5] {
            let base = Vec3::new(CORRIDOR_MAX.x, 0.0, z);
            decor.sconce(
                base + Vec3::Y * (wall_floor_height(base, Vec3::NEG_X) + 2.5),
                Vec3::NEG_X,
                Room::Hallway,
            );
        }
        for (x, z, normal) in [
            (AUDITORIUM_MIN.x, -8.0, Vec3::X),
            (AUDITORIUM_MIN.x, -18.5, Vec3::X),
            (AUDITORIUM_MAX.x, -5.5, Vec3::NEG_X),
            (AUDITORIUM_MAX.x, -20.0, Vec3::NEG_X),
        ] {
            let base = Vec3::new(x, 0.0, z);
            decor.sconce(
                base + Vec3::Y * (wall_floor_height(base, normal) + 3.0),
                normal,
                Room::Auditorium,
            );
        }

        for z in [-1.5, -6.0, SIDE_ENTRY_MIN_Z.midpoint(SIDE_ENTRY_MAX_Z)] {
            decor.route_light(
                Vec3::new(CORRIDOR_MAX.x, 0.0, z),
                Vec3::NEG_X,
                Room::Hallway,
            );
        }
        for z in [-7.2, -13.2, -19.0, -23.3] {
            decor.route_light(
                Vec3::new(AUDITORIUM_MAX.x, 0.0, z),
                Vec3::NEG_X,
                Room::Auditorium,
            );
        }
        let bend = Vec3::new(
            CORRIDOR_MAX.x,
            0.0,
            SIDE_ENTRY_MIN_Z.midpoint(SIDE_ENTRY_MAX_Z),
        );
        decor.wall_piece(
            Art::Star,
            bend + Vec3::Y * (wall_floor_height(bend, Vec3::NEG_X) + 1.2),
            Vec3::NEG_X,
            0.28,
            0.035,
            Room::Hallway,
            Response::Practical(0.55),
        );

        for id in ["seat_a2", "seat_a3", "seat_b2", "seat_b3"] {
            let seat = seat(id).expect("existing center-aisle chair");
            let normal = if seat.position.x < SCREEN_CENTER_X {
                Vec3::X
            } else {
                Vec3::NEG_X
            };
            let center = seat.position + normal * CHAIR_HALF_WIDTH + Vec3::new(0.0, 0.52, -0.16);
            decor.wall_piece(
                Art::Step,
                center,
                normal,
                0.12,
                0.012,
                Room::Auditorium,
                Response::Practical(0.65),
            );
        }
        decor
    }

    /// Append once during construction, before capturing the persistent mesh count.
    /// Runtime room-light updates then include the rug with the other base meshes.
    pub(crate) fn append_meshes(&self, meshes: &mut Vec<MeshInstance>) {
        meshes.push(self.rug.clone());
    }

    /// Submit into the SAME world-sprite list as avatars. The existing backend
    /// globally depth-sorts it and depth-tests without writing depth, so a frame's
    /// alpha opening cannot hide its artwork (unlike an ordinary surface mesh).
    pub(crate) fn append(&self, sprites: &mut Vec<SpriteInstance>, lobby: f32, auditorium: f32) {
        sprites.extend(self.pieces.iter().map(|piece| {
            let level = match piece.room {
                Room::Lobby | Room::Hallway => lobby,
                Room::Auditorium => auditorium,
            };
            let brightness = match piece.response {
                Response::Physical => level,
                Response::Practical(minimum) => minimum + (1.0 - minimum) * level,
            };
            let mut sprite = piece.sprite.clone();
            for channel in &mut sprite.tint[..3] {
                *channel *= brightness;
            }
            sprite
        }));
    }

    #[cfg(test)]
    pub(crate) fn owns_texture(&self, handle: SpriteHandle) -> bool {
        self.pieces
            .iter()
            .any(|piece| piece.sprite.sprite == handle)
    }

    fn push(&mut self, art: Art, transform: RenderTransform, room: Room, response: Response) {
        self.pieces.push(Piece {
            sprite: SpriteInstance {
                sprite: texture(art).handle,
                transform,
                space: SpriteSpace::World,
                tint: match art {
                    Art::WallGlow => [1.0, 0.94, 0.84, 0.32],
                    Art::FloorGlow => [1.0, 0.94, 0.84, 0.28],
                    _ => [1.0; 4],
                },
                ..SpriteInstance::default()
            },
            room,
            response,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn wall_piece(
        &mut self,
        art: Art,
        center: Vec3,
        normal: Vec3,
        height: f32,
        depth: f32,
        room: Room,
        response: Response,
    ) {
        self.push(
            art,
            RenderTransform {
                translation: center + normal * depth,
                rotation: Quat::from_rotation_y(normal.x.atan2(normal.z)),
                scale: height,
            },
            room,
            response,
        );
    }

    fn floor_piece(
        &mut self,
        art: Art,
        mut center: Vec3,
        width: f32,
        along_z: bool,
        room: Room,
        response: Response,
    ) {
        let profile = floor_profile(center);
        let pair = profile
            .windows(2)
            .find(|pair| center.z >= pair[0].0 && center.z <= pair[1].0)
            .expect("floor decoration inside a profile segment");
        let slope = (pair[1].1 - pair[0].1) / (pair[1].0 - pair[0].0);
        let normal = Vec3::new(0.0, 1.0, -slope).normalize();
        let north = normal.cross(Vec3::X);
        let mut rotation = Quat::from_mat3(&Mat3::from_cols(Vec3::X, north, normal));
        if along_z {
            rotation *= Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        }
        center.y = floor_height(center) + FLOOR_OFFSET;
        self.push(
            art,
            RenderTransform {
                translation: center,
                rotation,
                scale: width / aspect(art),
            },
            room,
            response,
        );
    }

    fn sconce(&mut self, center: Vec3, normal: Vec3, room: Room) {
        self.wall_piece(
            Art::WallGlow,
            center,
            normal,
            1.4,
            0.018,
            room,
            Response::Practical(0.60),
        );
        self.wall_piece(
            Art::Sconce,
            center,
            normal,
            0.85,
            0.045,
            room,
            Response::Practical(0.38),
        );
        self.floor_piece(
            Art::FloorGlow,
            center + normal * 0.6,
            1.4,
            normal.x != 0.0,
            room,
            Response::Practical(0.60),
        );
    }

    fn route_light(&mut self, base: Vec3, normal: Vec3, room: Room) {
        self.wall_piece(
            Art::Step,
            base + Vec3::Y * (wall_floor_height(base, normal) + 0.3),
            normal,
            0.13,
            0.028,
            room,
            Response::Practical(0.65),
        );
        self.floor_piece(
            Art::FloorGlow,
            base + normal * 0.45,
            0.85,
            true,
            room,
            Response::Practical(0.60),
        );
    }

    fn posters(&mut self) {
        // Read-only mirror of Theater::posters; tests compare to its actual art
        // vertices. Parent layout constants and source artwork ratios stay authoritative.
        let placements = [
            (Vec3::new(-HALF_WIDTH, POSTER_CENTER_Y, 5.0), Vec3::X, 2.0),
            (Vec3::new(-9.9, POSTER_CENTER_Y, 0.0), Vec3::Z, 3.6),
            (Vec3::new(5.3, POSTER_CENTER_Y, 0.0), Vec3::Z, 4.2),
            (
                Vec3::new(HALF_WIDTH, POSTER_CENTER_Y, 2.6),
                Vec3::NEG_X,
                4.0,
            ),
            (
                Vec3::new(HALF_WIDTH, POSTER_CENTER_Y, 7.3),
                Vec3::NEG_X,
                4.0,
            ),
            (Vec3::new(-7.0, POSTER_CENTER_Y, MAX_Z), Vec3::NEG_Z, 4.8),
            (Vec3::new(7.0, POSTER_CENTER_Y, MAX_Z), Vec3::NEG_Z, 4.8),
        ];
        for (index, (ratio, (center, normal, max_width))) in
            POSTER_RATIOS.into_iter().zip(placements).enumerate()
        {
            let art_height = 3.0_f32.min(max_width / ratio);
            let width = art_height * ratio + 2.0 * BORDER;
            let height = art_height + 2.0 * BORDER;
            let target_aspect = width / height;
            let raster_aspect = aspect(Art::Frame(index));
            // Crop whichever axis is oversized, never expand into an inner rail.
            // Both crops stay within the transparent half-texel outer guard.
            // World scale remains exact height; sampled U/V aspect sets width.
            let u_extent = (target_aspect / raster_aspect).min(1.0);
            let v_extent = (raster_aspect / target_aspect).min(1.0);
            self.wall_piece(
                Art::Frame(index),
                center,
                normal,
                height,
                FRAME_DEPTH,
                Room::Lobby,
                Response::Physical,
            );
            self.pieces
                .last_mut()
                .expect("frame just appended")
                .sprite
                .uv_rect = [
                (1.0 - u_extent) * 0.5,
                (1.0 - v_extent) * 0.5,
                u_extent,
                v_extent,
            ];
        }
    }

    fn curtains(&mut self) {
        let screen_left = SCREEN_CENTER_X - SCREEN_WIDTH * 0.5 - SCREEN_FRAME_BORDER;
        let screen_right = SCREEN_CENTER_X + SCREEN_WIDTH * 0.5 + SCREEN_FRAME_BORDER;
        let bottom = SCREEN_BOTTOM - SCREEN_FRAME_BORDER;
        let top = SCREEN_TOP + SCREEN_FRAME_BORDER;
        let height = top - bottom;
        for (art, left, right) in [
            (Art::CurtainLeft, AUDITORIUM_MIN.x, screen_left),
            (Art::CurtainRight, screen_right, AUDITORIUM_MAX.x),
        ] {
            let full_width = height * aspect(art);
            let width = full_width.min(right - left - 0.03);
            let center_x = if art == Art::CurtainLeft { right } else { left };
            self.wall_piece(
                art,
                Vec3::new(center_x, bottom.midpoint(top), MIN_Z),
                Vec3::Z,
                height,
                0.12,
                Room::Auditorium,
                Response::Physical,
            );
            // Fit the narrow wall bays by cropping only the outer hem, rather
            // than shortening the curtains or stretching the full-height art.
            let u_extent = width / full_width;
            let u = if art == Art::CurtainLeft {
                1.0 - u_extent
            } else {
                0.0
            };
            self.pieces
                .last_mut()
                .expect("curtain just appended")
                .sprite
                .uv_rect = [u, 0.0, u_extent, 1.0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn corners(sprite: &SpriteInstance) -> [Vec3; 4] {
        let texture = resources()
            .iter()
            .find(|resource| resource.texture.handle == sprite.sprite)
            .unwrap();
        let half_height = sprite.transform.scale * 0.5;
        let half_width = half_height * texture.texture.width as f32 * sprite.uv_rect[2].abs()
            / (texture.texture.height as f32 * sprite.uv_rect[3].abs());
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| {
            sprite.transform.translation
                + sprite.transform.rotation * Vec3::new(x * half_width, y * half_height, 0.0)
        })
    }

    fn bilinear_alpha(texture: &TextureAsset, u: f32, v: f32) -> f32 {
        let x = u * texture.width as f32 - 0.5;
        let y = v * texture.height as f32 - 0.5;
        let fx = x - x.floor();
        let fy = y - y.floor();
        let mut alpha = 0.0;
        for (dx, weight_x) in [(0, 1.0 - fx), (1, fx)] {
            for (dy, weight_y) in [(0, 1.0 - fy), (1, fy)] {
                let px = (x.floor() as i32 + dx).clamp(0, texture.width as i32 - 1) as u32;
                let py = (y.floor() as i32 + dy).clamp(0, texture.height as i32 - 1) as u32;
                alpha += f32::from(texture.rgba[((py * texture.width + px) * 4 + 3) as usize])
                    * weight_x
                    * weight_y;
            }
        }
        alpha
    }

    #[test]
    fn last_two_frames_use_guarded_v_crop_instead_of_unsafe_u_expansion() {
        let decor = Decor::new(&mut AppAssets::default());
        for index in [5, 6] {
            let texture = texture(Art::Frame(index));
            let art_width = 4.8;
            let art_height = art_width / POSTER_RATIOS[index];
            let width = art_width + 2.0 * BORDER;
            let height = art_height + 2.0 * BORDER;
            let frame = &piece(&decor, Art::Frame(index)).sprite;
            assert_eq!(frame.transform.scale, height);
            assert_eq!(frame.uv_rect[0], 0.0);
            assert_eq!(frame.uv_rect[2], 1.0);
            assert!(frame.uv_rect[1] > 0.0);
            assert!(frame.uv_rect[1] * (texture.height as f32) < 0.5);
            assert!(
                (frame.uv_rect[3] - aspect(Art::Frame(index)) / (width / height)).abs() < 0.000_001
            );
            // Expanding U was unsafe even within the same outer-guard budget.
            let extent = (width / height) / aspect(Art::Frame(index));
            assert!((extent - 1.0).abs() * texture.width as f32 * 0.5 < 0.5);
            let u = (1.0 - extent) * 0.5 + (0.5 - art_width * 0.5 / width) * extent;
            assert!(bilinear_alpha(texture, u, 0.5) > 60.0);
        }
    }

    fn piece(decor: &Decor, art: Art) -> &Piece {
        decor
            .pieces
            .iter()
            .find(|piece| piece.sprite.sprite == texture(art).handle)
            .unwrap()
    }

    fn for_art(decor: &Decor, art: Art) -> impl Iterator<Item = &Piece> {
        decor
            .pieces
            .iter()
            .filter(move |piece| piece.sprite.sprite == texture(art).handle)
    }

    #[test]
    fn bundled_resources_are_bounded_rgba_with_transparent_widgets_and_soft_glows() {
        let decor = Decor::new(&mut AppAssets::default());
        for resource in resources() {
            let asset = &resource.texture;
            let bound = match resource.art {
                Art::Sconce | Art::Step => 512,
                Art::Star | Art::FloorGlow => 256,
                Art::WallGlow => 128,
                Art::Portfolio => 1536,
                _ => 1024,
            };
            assert!(asset.width > 0 && asset.height > 0);
            assert!(asset.width.max(asset.height) <= bound);
            assert_eq!(asset.rgba.len(), (asset.width * asset.height * 4) as usize);
            if resource.art == Art::Rug {
                assert_eq!(decor.rug.texture, Some(asset.handle));
                assert_eq!(for_art(&decor, Art::Rug).count(), 0);
            } else {
                assert!(for_art(&decor, resource.art).next().is_some());
            }
            let alpha: Vec<_> = asset
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| pixel[3])
                .collect();
            if resource.art == Art::Rug {
                assert_eq!((asset.width, asset.height), (1024, 1024));
                assert!(alpha.iter().all(|alpha| *alpha == 255));
            } else {
                assert!(alpha.contains(&0));
                assert!(alpha.iter().any(|alpha| *alpha > 0));
            }
            if matches!(resource.art, Art::WallGlow | Art::FloorGlow) {
                assert!(alpha.iter().all(|alpha| *alpha <= 48));
                assert!(
                    alpha[..asset.width as usize]
                        .iter()
                        .all(|alpha| *alpha == 0)
                );
                assert!(
                    alpha[alpha.len() - asset.width as usize..]
                        .iter()
                        .all(|alpha| *alpha == 0)
                );
            }
        }
        assert_eq!(
            (texture(Art::WallGlow).width, texture(Art::WallGlow).height),
            (128, 128)
        );
        assert_eq!(
            (
                texture(Art::FloorGlow).width,
                texture(Art::FloorGlow).height
            ),
            (256, 128)
        );
    }

    #[test]
    fn portfolio_fills_clear_bay_and_aligns_with_neighboring_poster_centers() {
        let mut theater = Theater::new();
        let decor = Decor::new(&mut theater.assets);
        let plaque = piece(&decor, Art::Portfolio);
        let sprite = &plaque.sprite;
        let asset = texture(Art::Portfolio);
        assert_eq!((asset.width, asset.height), (1536, 870));
        assert_eq!(for_art(&decor, Art::Portfolio).count(), 1);
        assert_eq!(sprite.transform.translation, PORTFOLIO_IMAGE_CENTER);
        let godfather = &piece(&decor, Art::Frame(1)).sprite;
        let star_wars = &piece(&decor, Art::Frame(2)).sprite;
        let godfather_right = corners(godfather)
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let star_wars_left = corners(star_wars)
            .iter()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min);
        assert!((PORTFOLIO_CENTER.x - godfather_right.midpoint(star_wars_left)).abs() < 0.0001);
        assert_eq!(PORTFOLIO_CENTER.y, godfather.transform.translation.y);
        assert_eq!(PORTFOLIO_CENTER.y, star_wars.transform.translation.y);
        assert!((PORTFOLIO_BOTTOM.midpoint(PORTFOLIO_TOP) - PORTFOLIO_CENTER.y).abs() < 0.0001);
        const { assert!(PORTFOLIO_CENTER.z < super::super::SPAWN.z) };
        assert_eq!(sprite.transform.rotation * Vec3::Z, Vec3::Z);
        assert_eq!(plaque.room, Room::Lobby);
        let points = corners(sprite);
        for point in points {
            assert!(point.y > 0.0 && point.y <= 4.45 + 0.0001);
            assert!(point.x > godfather_right && point.x < star_wars_left);
            assert!((point.z - 0.04).abs() < 0.00001);
        }
        let width = points[0].distance(points[1]);
        assert!((width / PORTFOLIO_HEIGHT - 1536.0 / 870.0).abs() < 0.00001);
        const { assert!(PORTFOLIO_HEIGHT > 4.1) };
        let top = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
        assert!((top - 4.45).abs() < 0.0001);
        let left = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let right = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
        assert!(((left - godfather_right) - (star_wars_left - right)).abs() < 0.01);
        assert!(PORTFOLIO_HALF_WIDTH < width * 0.5);
        const { assert!(PORTFOLIO_BOTTOM > PORTFOLIO_CENTER.y - PORTFOLIO_HEIGHT * 0.5) };
        const { assert!(PORTFOLIO_TOP < PORTFOLIO_CENTER.y + PORTFOLIO_HEIGHT * 0.5) };
        assert!(super::super::has_line_of_sight(
            super::super::SPAWN,
            PORTFOLIO_CENTER,
            &theater.colliders,
        ));
        let mut dark = Vec::new();
        decor.append(&mut dark, 0.0, 0.0);
        let dark = dark
            .iter()
            .find(|piece| piece.sprite == asset.handle)
            .unwrap();
        assert_eq!(dark.tint, [0.85, 0.85, 0.85, 1.0]);
    }

    #[test]
    fn stable_resources_append_one_rug_without_changing_legacy_texture_or_mesh_prefix() {
        let mut theater = Theater::new();
        let legacy: Vec<_> = theater
            .assets
            .textures
            .iter()
            .map(|asset| (asset.handle, asset.width, asset.height, asset.rgba.clone()))
            .collect();
        let legacy_meshes = theater.assets.meshes.clone();
        let legacy_instances = theater.instances.clone();
        assert_eq!(legacy.len(), 5 + POSTER_COUNT + MATERIAL_TEXTURE_COUNT);
        let decor = Decor::new(&mut theater.assets);
        assert_eq!(resources().len(), TEXTURE_COUNT);
        assert_eq!(theater.assets.textures.len(), legacy.len() + TEXTURE_COUNT);
        for (asset, (handle, width, height, rgba)) in theater.assets.textures.iter().zip(legacy) {
            assert_eq!(
                (asset.handle, asset.width, asset.height, &asset.rgba),
                (handle, width, height, &rgba)
            );
        }
        assert_eq!(theater.assets.meshes.len(), legacy_meshes.len() + 1);
        for (before, after) in legacy_meshes.iter().zip(&theater.assets.meshes) {
            assert_eq!(before.handle, after.handle);
            assert_eq!(before.indices, after.indices);
            let vertex_bits = |mesh: &MeshAsset| {
                mesh.vertices
                    .iter()
                    .flat_map(|vertex| {
                        vertex
                            .position
                            .into_iter()
                            .chain(vertex.normal)
                            .chain(vertex.color)
                            .chain(vertex.uv)
                            .map(f32::to_bits)
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(vertex_bits(before), vertex_bits(after));
        }
        assert_eq!(theater.instances, legacy_instances);
        let mut instances = theater.instances.clone();
        decor.append_meshes(&mut instances);
        assert_eq!(
            &instances[..legacy_instances.len()],
            legacy_instances.as_slice()
        );
        assert_eq!(instances.len(), legacy_instances.len() + 1);
        assert_eq!(instances.last().unwrap().mesh, rug_mesh().handle);
        let again = Decor::new(&mut theater.assets);
        assert_eq!(theater.assets.meshes.len(), legacy_meshes.len() + 1);
        assert_eq!(
            theater.assets.textures.len(),
            5 + POSTER_COUNT + MATERIAL_TEXTURE_COUNT + TEXTURE_COUNT
        );
        let sentinel = SpriteHandle::new();
        let mut unrelated_assets = AppAssets {
            textures: vec![TextureAsset {
                handle: sentinel,
                width: 1,
                height: 1,
                rgba: vec![17, 23, 31, 255],
            }],
            ..AppAssets::default()
        };
        let other = Decor::new(&mut unrelated_assets);
        assert_eq!(unrelated_assets.textures[0].handle, sentinel);
        assert_eq!(unrelated_assets.textures.len(), 1 + TEXTURE_COUNT);
        let mut first = Vec::new();
        decor.append(&mut first, 1.0, 0.2);
        for runtime in [again, other] {
            let mut sprites = Vec::new();
            runtime.append(&mut sprites, 1.0, 0.2);
            assert_eq!(first, sprites);
        }
        for sprite in &first {
            assert!(
                unrelated_assets
                    .textures
                    .iter()
                    .any(|asset| asset.handle == sprite.sprite)
            );
        }
    }

    #[test]
    fn modest_fixed_world_collection_uses_the_non_depth_writing_sprite_path() {
        let mut assets = AppAssets::default();
        let decor = Decor::new(&mut assets);
        assert_eq!(assets.meshes.len(), 1);
        assert_eq!(assets.meshes[0].handle, decor.rug.mesh);
        assert_eq!(decor.pieces.len(), 59);
        assert_eq!(for_art(&decor, Art::Rug).count(), 0);
        assert_eq!(for_art(&decor, Art::Sconce).count(), 10);
        assert_eq!(for_art(&decor, Art::Step).count(), 11);
        assert_eq!(for_art(&decor, Art::Star).count(), 1);
        assert_eq!(for_art(&decor, Art::FloorGlow).count(), 17);
        for (room, expected) in [(Room::Lobby, 4), (Room::Hallway, 2), (Room::Auditorium, 4)] {
            assert_eq!(
                for_art(&decor, Art::Sconce)
                    .filter(|piece| piece.room == room)
                    .count(),
                expected
            );
        }
        for (piece_index, piece) in decor.pieces.iter().enumerate() {
            let sprite = &piece.sprite;
            let art = resources()
                .iter()
                .find(|resource| resource.texture.handle == sprite.sprite)
                .unwrap()
                .art;
            // World selects Less depth testing with depth_write_enabled=false in
            // SpritePipeline. These translucent pieces must stay off the rug's
            // depth-writing Surface path, especially the hollow poster frames.
            assert_eq!(sprite.space, SpriteSpace::World);
            assert_eq!(sprite.layer, 0);
            if !matches!(art, Art::Frame(_) | Art::CurtainLeft | Art::CurtainRight) {
                assert_eq!(sprite.uv_rect, [0.0, 0.0, 1.0, 1.0]);
            }
            assert_eq!(sprite.pivot, Vec2::splat(0.5));
            assert!(sprite.transform.translation.is_finite());
            assert!(sprite.transform.rotation.is_finite());
            assert!((sprite.transform.rotation.length() - 1.0).abs() < 0.00001);
            assert!(sprite.transform.scale.is_finite() && sprite.transform.scale > 0.0);
            assert!(
                sprite
                    .tint
                    .iter()
                    .all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel))
            );
            for (corner_index, corner) in corners(sprite).into_iter().enumerate() {
                assert!(corner.is_finite());
                assert!(
                    corner.y >= floor_height(corner) - 0.00001,
                    "piece {piece_index} {art:?} room={:?} center={:?} normal={:?}: corner {corner_index} {corner:?} below floor={} profile={:?}",
                    piece.room,
                    sprite.transform.translation,
                    sprite.transform.rotation * Vec3::Z,
                    floor_height(corner),
                    floor_profile(corner)
                );
                assert!(
                    corner.y <= ceiling_height(corner) + 0.003,
                    "piece {piece_index} {art:?} room={:?} center={:?}: corner {corner_index} {corner:?} above ceiling={}",
                    piece.room,
                    sprite.transform.translation,
                    ceiling_height(corner)
                );
            }
        }
    }

    #[test]
    fn divider_fixtures_use_the_floor_on_their_visible_auditorium_side() {
        let decor = Decor::new(&mut AppAssets::default());
        for (art, height, expected_count) in [
            (Art::Sconce, 3.0, 2),
            (Art::WallGlow, 3.0, 2),
            (Art::Step, 0.3, 4),
        ] {
            let fixtures: Vec<_> = for_art(&decor, art)
                .filter(|piece| {
                    piece.room == Room::Auditorium
                        && (piece.sprite.transform.translation.x - AUDITORIUM_MAX.x).abs() < 0.1
                })
                .collect();
            assert_eq!(fixtures.len(), expected_count);
            for piece in fixtures {
                let center = piece.sprite.transform.translation;
                let floor = floor_height(center);
                let boundary = Vec3::new(AUDITORIUM_MAX.x, 0.0, center.z);
                assert_ne!(
                    floor,
                    floor_height(boundary),
                    "exercise unequal divider-side floors"
                );
                assert!(
                    (center.y - floor - height).abs() < 0.00001,
                    "{art:?} at {center:?}: expected {height}m above auditorium floor {floor}, hallway floor={}",
                    floor_height(boundary)
                );
            }
        }
    }

    #[test]
    fn seven_hollow_frames_match_real_art_vertices_and_eight_centimeter_borders() {
        let mut theater = Theater::new();
        let decor = Decor::new(&mut theater.assets);
        for index in 0..POSTER_COUNT {
            let art_handle = theater.assets.textures[5 + index].handle;
            let instance = theater
                .instances
                .iter()
                .find(|mesh| mesh.texture == Some(art_handle))
                .unwrap();
            let mesh = theater
                .assets
                .meshes
                .iter()
                .find(|mesh| mesh.handle == instance.mesh)
                .unwrap();
            let points: Vec<_> = mesh
                .vertices
                .iter()
                .map(|vertex| Vec3::from_array(vertex.position))
                .collect();
            let center = points.iter().copied().sum::<Vec3>() * 0.25;
            let right = (points[1] - points[0]).normalize();
            let normal = Vec3::from_array(mesh.vertices[0].normal);
            let art_width = points[0].distance(points[1]);
            let art_height = points[1].distance(points[2]);
            let frame = &piece(&decor, Art::Frame(index)).sprite;
            let texture = texture(Art::Frame(index));
            let outer = corners(frame);
            let width = outer[0].distance(outer[1]);
            let height = outer[1].distance(outer[2]);
            assert!(
                frame
                    .transform
                    .translation
                    .abs_diff_eq(center + normal * 0.015, 0.00001)
            );
            assert!((frame.transform.rotation * Vec3::X).abs_diff_eq(right, 0.00001));
            assert!((frame.transform.rotation * Vec3::Z).abs_diff_eq(normal, 0.00001));
            assert!((width - art_width - 2.0 * BORDER).abs() < 0.00001);
            assert!((height - art_height - 2.0 * BORDER).abs() < 0.00001);
            for (offset, extent, pixels) in [
                (frame.uv_rect[0], frame.uv_rect[2], texture.width),
                (frame.uv_rect[1], frame.uv_rect[3], texture.height),
            ] {
                assert!((offset + extent * 0.5 - 0.5).abs() < 0.000_001);
                assert!(offset >= 0.0 && extent > 0.0 && extent <= 1.0);
                assert!(offset * pixels as f32 <= 0.5);
            }
            let target_aspect = (art_width + 2.0 * BORDER) / (art_height + 2.0 * BORDER);
            if aspect(Art::Frame(index)) > target_aspect {
                assert_eq!(frame.uv_rect[3], 1.0);
            } else {
                assert_eq!(frame.uv_rect[2], 1.0);
            }
            let floor_gap = outer[0].y - floor_height(center);
            let ceiling_gap = ceiling_height(center) - outer[2].y;
            assert!(floor_gap > 0.0);
            assert!((floor_gap - ceiling_gap).abs() < 0.00001);
            // The outer guard survives the fractional crop, and bilinear samples
            // at every art edge/corner remain transparent, not just texel centers.
            for y in 0..texture.height {
                for x in [0, texture.width - 1] {
                    assert_eq!(texture.rgba[((y * texture.width + x) * 4 + 3) as usize], 0);
                }
            }
            for x in 0..texture.width {
                for y in [0, texture.height - 1] {
                    assert_eq!(texture.rgba[((y * texture.width + x) * 4 + 3) as usize], 0);
                }
            }
            for step in 0..=1024 {
                let t = step as f32 / 1024.0 - 0.5;
                for (x, y) in [
                    (-art_width * 0.5, art_height * t),
                    (art_width * 0.5, art_height * t),
                    (art_width * t, -art_height * 0.5),
                    (art_width * t, art_height * 0.5),
                ] {
                    let u = frame.uv_rect[0] + (x / width + 0.5) * frame.uv_rect[2];
                    let v = frame.uv_rect[1] + (y / height + 0.5) * frame.uv_rect[3];
                    assert!(
                        bilinear_alpha(texture, u, v) < 0.01,
                        "frame {index} art edge at {x},{y}"
                    );
                }
            }
            // The entire art opening is transparent, not a black/opaque quad.
            for y in 0..texture.height {
                for x in 0..texture.width {
                    let local_x = (((x as f32 + 0.5) / texture.width as f32 - frame.uv_rect[0])
                        / frame.uv_rect[2]
                        - 0.5)
                        * width;
                    let local_y = (((y as f32 + 0.5) / texture.height as f32 - frame.uv_rect[1])
                        / frame.uv_rect[3]
                        - 0.5)
                        * height;
                    let alpha = texture.rgba[((y * texture.width + x) * 4 + 3) as usize];
                    if local_x.abs() < art_width * 0.5 && local_y.abs() < art_height * 0.5 {
                        assert_eq!(alpha, 0, "frame {index} opening at {x},{y}");
                    }
                }
            }
            for (x, y) in [
                (0.0, art_height * 0.5 + BORDER * 0.5),
                (0.0, -art_height * 0.5 - BORDER * 0.5),
                (art_width * 0.5 + BORDER * 0.5, 0.0),
                (-art_width * 0.5 - BORDER * 0.5, 0.0),
            ] {
                let x = ((frame.uv_rect[0] + (x / width + 0.5) * frame.uv_rect[2])
                    * texture.width as f32) as u32;
                let y = ((frame.uv_rect[1] + (y / height + 0.5) * frame.uv_rect[3])
                    * texture.height as f32) as u32;
                assert!(texture.rgba[((y * texture.width + x) * 4 + 3) as usize] > 0);
            }
        }
    }

    #[test]
    fn opaque_rug_writes_depth_with_exact_floor_plane_north_up_uvs_and_lobby_lighting() {
        let mut theater = Theater::new();
        let decor = Decor::new(&mut theater.assets);
        let mut instances = Vec::new();
        decor.append_meshes(&mut instances);
        assert_eq!(instances.len(), 1);
        let rug = &instances[0];
        assert_eq!(rug.render_mode, MeshRenderMode::Surface);
        assert_eq!(rug.texture, Some(texture(Art::Rug).handle));
        assert_eq!(rug.transform, RenderTransform::default());
        assert_eq!(rug.color, [1.0; 4]);
        assert_eq!(rug.emissive, 0.0);
        assert_eq!(rug.texture_fit, None);
        assert!(
            texture(Art::Rug)
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
        assert!(
            !decor
                .pieces
                .iter()
                .any(|piece| piece.sprite.sprite == texture(Art::Rug).handle)
        );
        let mesh = theater
            .assets
            .meshes
            .iter()
            .find(|mesh| mesh.handle == rug.mesh)
            .unwrap();
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3]);
        let points: Vec<_> = mesh
            .vertices
            .iter()
            .map(|vertex| Vec3::from_array(vertex.position))
            .collect();
        let center = points.iter().copied().sum::<Vec3>() * 0.25;
        assert!(center.abs_diff_eq(Vec3::new(0.0, FLOOR_OFFSET, 5.0), 0.000_001));
        assert!((points[0].distance(points[1]) - 3.6).abs() < 0.000_001);
        assert!((points[1].distance(points[2]) - 3.6).abs() < 0.000_001);
        for (vertex, uv) in
            mesh.vertices
                .iter()
                .zip([[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]])
        {
            let position = Vec3::from_array(vertex.position);
            assert!((position.y - floor_height(position) - FLOOR_OFFSET).abs() < 0.000_001);
            assert_eq!(vertex.normal, Vec3::Y.to_array());
            assert_eq!(vertex.color, [1.0; 4]);
            assert_eq!(vertex.uv, uv);
            assert!((position.x - (uv[0] - 0.5) * 3.6).abs() < 0.000_001);
            assert!((position.z - (5.0 + (uv[1] - 0.5) * 3.6)).abs() < 0.000_001);
        }
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| points[usize::from(i)]);
            assert!(
                (b - a)
                    .cross(c - a)
                    .normalize()
                    .abs_diff_eq(Vec3::Y, 0.000_001)
            );
        }
        assert_eq!(rug.lighting, theater.instances[0].lighting);
        let light_position = rug.lighting.unwrap().position;
        assert!(!in_auditorium(light_position));
        for (lobby, auditorium) in [(0.0, 1.0), (0.4, 0.2), (1.0, 0.0)] {
            apply_room_lighting(
                &mut instances,
                lobby,
                auditorium,
                theater.assets.textures[4].handle,
            );
            let light = instances[0].lighting.unwrap();
            assert_eq!(light.position, light_position);
            assert_eq!(light.ambient, LOBBY_AMBIENT_SHARE * lobby);
            assert_eq!(light.intensity, (1.0 - LOBBY_AMBIENT_SHARE) * lobby);
            assert_eq!(instances[0].mesh, mesh.handle);
            assert_eq!(instances[0].render_mode, MeshRenderMode::Surface);
        }
    }

    #[test]
    fn floor_overlays_follow_each_ramp_without_crossing_profile_breaks() {
        let decor = Decor::new(&mut AppAssets::default());
        for art in [Art::FloorGlow] {
            for piece in for_art(&decor, art) {
                let points = corners(&piece.sprite);
                for corner in points {
                    assert!((corner.y - floor_height(corner) - FLOOR_OFFSET).abs() < 0.00001);
                }
                assert!(
                    floor_profile(piece.sprite.transform.translation)
                        .windows(2)
                        .any(|pair| {
                            points
                                .iter()
                                .all(|point| point.z >= pair[0].0 && point.z <= pair[1].0)
                        })
                );
                assert!((piece.sprite.transform.rotation * Vec3::Z).y > 0.9);
            }
        }
    }

    #[test]
    fn four_tiny_lights_are_on_the_actual_inner_seat_arm_sides() {
        let decor = Decor::new(&mut AppAssets::default());
        for id in ["seat_a2", "seat_a3", "seat_b2", "seat_b3"] {
            let seat = seat(id).unwrap();
            let normal = if seat.position.x < SCREEN_CENTER_X {
                Vec3::X
            } else {
                Vec3::NEG_X
            };
            let expected =
                seat.position + normal * (CHAIR_HALF_WIDTH + 0.012) + Vec3::new(0.0, 0.52, -0.16);
            let lights: Vec<_> = for_art(&decor, Art::Step)
                .filter(|piece| {
                    piece
                        .sprite
                        .transform
                        .translation
                        .abs_diff_eq(expected, 0.00001)
                })
                .collect();
            assert_eq!(lights.len(), 1);
            let sprite = &lights[0].sprite;
            assert!((sprite.transform.rotation * Vec3::Z).abs_diff_eq(normal, 0.00001));
            for corner in corners(sprite) {
                assert!(corner.y >= seat.position.y + 0.36 && corner.y <= seat.position.y + 0.65);
                assert!(
                    corner.z >= seat.position.z - CHAIR_HALF_DEPTH
                        && corner.z <= seat.position.z + 0.30
                );
            }
        }
    }

    #[test]
    fn curtains_center_on_screen_edges_without_stretching_the_artwork() {
        let decor = Decor::new(&mut AppAssets::default());
        for art in [Art::CurtainLeft, Art::CurtainRight] {
            let sprite = &piece(&decor, art).sprite;
            let points = corners(sprite);
            assert!((points[0].y - (SCREEN_BOTTOM - SCREEN_FRAME_BORDER)).abs() < 0.00001);
            assert!((points[2].y - (SCREEN_TOP + SCREEN_FRAME_BORDER)).abs() < 0.00001);
            let frame_x = if art == Art::CurtainLeft {
                SCREEN_CENTER_X - SCREEN_WIDTH * 0.5 - SCREEN_FRAME_BORDER
            } else {
                SCREEN_CENTER_X + SCREEN_WIDTH * 0.5 + SCREEN_FRAME_BORDER
            };
            assert!((sprite.transform.translation.x - frame_x).abs() < 0.00001);
            let half_width = points[0].distance(points[1]) * 0.5;
            assert!((points[0].x - (frame_x - half_width)).abs() < 0.00001);
            assert!((points[1].x - (frame_x + half_width)).abs() < 0.00001);

            // Only the outer hem is cropped; the screen-facing edge and full
            // vertical artwork remain, with equal meters per texel on both axes.
            let [u, v, width, height] = sprite.uv_rect;
            assert_eq!((v, height), (0.0, 1.0));
            assert!(width > 0.95 && width < 1.0);
            if art == Art::CurtainLeft {
                assert!((u + width - 1.0).abs() < 0.00001);
            } else {
                assert_eq!(u, 0.0);
            }
            let texture = texture(art);
            let pixel_width = points[0].distance(points[1]) / (texture.width as f32 * width);
            let pixel_height = points[1].distance(points[2]) / texture.height as f32;
            assert!((pixel_width - pixel_height).abs() < 0.00001);
        }
    }

    #[test]
    fn curtains_controls_entrance_side_entry_and_artwork_remain_clear() {
        let theater_mesh_count = Theater::new().instances.len();
        for minimal_ui in [false, true] {
            let app = crate::FirstPersonLab::with_browser_presentation(minimal_ui);
            let decor = &app.decor;
            assert_eq!(app.minimal_ui, minimal_ui);
            assert_eq!(app.scene.meshes.len(), app.base_mesh_count);
            assert_eq!(app.scene.meshes.last().unwrap().mesh, decor.rug.mesh);
            let panels = &app.scene.meshes[theater_mesh_count..app.base_mesh_count - 1];
            assert_eq!(panels.len(), if minimal_ui { 0 } else { 3 });
            let panel_bounds: Vec<_> = panels
                .iter()
                .map(|instance| {
                    assert_eq!(instance.transform, RenderTransform::default());
                    let mesh = app
                        .assets
                        .meshes
                        .iter()
                        .find(|mesh| mesh.handle == instance.mesh)
                        .unwrap();
                    assert_eq!(mesh.vertices.len(), 4);
                    mesh.vertices
                        .iter()
                        .map(|vertex| Vec3::from_array(vertex.position))
                        .fold(
                            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                            |(min, max), point| (min.min(point), max.max(point)),
                        )
                })
                .collect();

            let portfolio = &piece(decor, Art::Portfolio).sprite;
            assert_eq!(for_art(decor, Art::Portfolio).count(), 1);
            assert_eq!(portfolio.transform.translation, PORTFOLIO_IMAGE_CENTER);
            assert_eq!(portfolio.transform.scale, PORTFOLIO_HEIGHT);
            assert!(app.scene.sprites.contains(portfolio));
            for art in [Art::CurtainLeft, Art::CurtainRight] {
                let sprite = &piece(decor, art).sprite;
                let points = corners(sprite);
                assert!((sprite.transform.rotation * Vec3::Z).abs_diff_eq(Vec3::Z, 0.00001));
                assert!(sprite.transform.scale <= SCREEN_HEIGHT + 2.0 * SCREEN_FRAME_BORDER);
                let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
                let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
                assert!(min_x > AUDITORIUM_MIN.x && max_x < AUDITORIUM_MAX.x);
                assert!(
                    (sprite.transform.translation.x - SCREEN_CENTER_X).abs()
                        >= SCREEN_WIDTH * 0.5 + SCREEN_FRAME_BORDER - 0.00001
                );
            }
            for piece in &decor.pieces {
                let sprite = &piece.sprite;
                let normal = sprite.transform.rotation * Vec3::Z;
                let points = corners(sprite);
                let min = points
                    .iter()
                    .copied()
                    .fold(Vec3::splat(f32::INFINITY), Vec3::min);
                let max = points
                    .iter()
                    .copied()
                    .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
                if normal.abs_diff_eq(Vec3::Z, 0.00001)
                    && sprite.transform.translation.z.abs() < 0.1
                {
                    // The portfolio occupies the old control bay in both modes;
                    // only nonminimal presentation still allocates those panels.
                    // Other decor must clear their actual meshes when present.
                    if sprite.sprite != portfolio.sprite {
                        for (panel_min, panel_max) in &panel_bounds {
                            assert!(
                                max.x < panel_min.x
                                    || min.x > panel_max.x
                                    || max.y < panel_min.y
                                    || min.y > panel_max.y,
                                "decor {:?} overlaps a world panel in minimal_ui={minimal_ui}",
                                sprite.sprite,
                            );
                        }
                    }
                    assert!(max.x < PASSAGE_MIN); // Open hallway mouth.
                }
                if normal.abs_diff_eq(Vec3::NEG_Z, 0.00001)
                    && (sprite.transform.translation.z - MAX_Z).abs() < 0.1
                {
                    assert!(max.x < -1.2 || min.x > 1.2); // Sealed entrance art.
                }
                if normal.abs_diff_eq(Vec3::NEG_X, 0.00001)
                    && (sprite.transform.translation.x - PASSAGE_MIN).abs() < 0.1
                {
                    assert!(max.z < SIDE_ENTRY_MIN_Z || min.z > SIDE_ENTRY_MAX_Z);
                }
                // Non-frame sprites cannot cover any of the seven art/backing bays.
                for index in 0..POSTER_COUNT {
                    if sprite.sprite == texture(Art::Frame(index)).handle {
                        continue;
                    }
                    let frame = &self::piece(decor, Art::Frame(index)).sprite;
                    let frame_normal = frame.transform.rotation * Vec3::Z;
                    let delta = sprite.transform.translation - frame.transform.translation;
                    if !normal.abs_diff_eq(frame_normal, 0.00001)
                        || delta.dot(frame_normal).abs() > 0.1
                    {
                        continue;
                    }
                    let right = frame.transform.rotation * Vec3::X;
                    let half_width = corners(frame)[0].distance(corners(frame)[1]) * 0.5;
                    let half_height = frame.transform.scale * 0.5;
                    let local: Vec<_> = points
                        .iter()
                        .map(|point| {
                            let delta = *point - frame.transform.translation;
                            Vec2::new(delta.dot(right), delta.y)
                        })
                        .collect();
                    assert!(
                        local.iter().all(|p| p.x < -half_width)
                            || local.iter().all(|p| p.x > half_width)
                            || local.iter().all(|p| p.y < -half_height)
                            || local.iter().all(|p| p.y > half_height)
                    );
                }
            }
        }
    }

    #[test]
    fn room_tints_dim_physical_art_but_keep_practicals_readable_without_pose_drift() {
        let decor = Decor::new(&mut AppAssets::default());
        let mut full = Vec::new();
        decor.append(&mut full, 1.0, 1.0);
        for (lobby, auditorium) in [(0.0, 0.0), (0.3, 0.8), (1.0, 0.0), (0.0, 1.0)] {
            let mut dim = Vec::new();
            decor.append(&mut dim, lobby, auditorium);
            for ((piece, full), dim) in decor.pieces.iter().zip(&full).zip(&dim) {
                assert_eq!(full.transform, dim.transform);
                assert_eq!(full.sprite, dim.sprite);
                assert_eq!(full.tint[3], dim.tint[3]);
                let level = if piece.room == Room::Auditorium {
                    auditorium
                } else {
                    lobby
                };
                match piece.response {
                    Response::Physical => assert_eq!(dim.tint, [level, level, level, 1.0]),
                    Response::Practical(minimum) => {
                        assert!(dim.tint[0] >= minimum && dim.tint[0] <= 1.0);
                        assert_eq!(dim.tint[0], minimum + (1.0 - minimum) * level);
                    }
                }
            }
        }
        let mut again = Vec::new();
        decor.append(&mut again, 1.0, 1.0);
        assert_eq!(full, again);
    }
}
