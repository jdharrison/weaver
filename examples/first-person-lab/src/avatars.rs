//! Stable avatar identity and calibrated, camera-facing directional stills.

use glam::{Quat, Vec3};
use serde::Deserialize;
use std::sync::OnceLock;
use weaver_app_core::TextureAsset;
use weaver_render::{RenderTransform, SpriteHandle, SpriteInstance, SpriteSpace};

pub(crate) const AVATAR_COUNT: usize = 9;
pub(crate) const TEXTURE_COUNT: usize = AVATAR_COUNT * 6;

/// Both arrays are [front, back, left]; right reuses left with mirrored UVs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AvatarTextures {
    pub(crate) standing: [SpriteHandle; 3],
    pub(crate) seated: [SpriteHandle; 3],
}

macro_rules! views {
    ($name:literal) => {
        [
            include_bytes!(concat!(
                "../assets/avatars/",
                $name,
                "/avatar_",
                $name,
                ".png"
            ))
            .as_slice(),
            include_bytes!(concat!("../assets/avatars/", $name, "/back.png")).as_slice(),
            include_bytes!(concat!("../assets/avatars/", $name, "/left_side.png")).as_slice(),
            include_bytes!(concat!("../assets/avatars/", $name, "/seated_front.png")).as_slice(),
            include_bytes!(concat!("../assets/avatars/", $name, "/seated_back.png")).as_slice(),
            include_bytes!(concat!(
                "../assets/avatars/",
                $name,
                "/seated_left_side.png"
            ))
            .as_slice(),
        ]
    };
}

const AVATAR_PNGS: [[&[u8]; 6]; AVATAR_COUNT] = [
    views!("01_rust_drifter"),
    views!("02_chrome_usher"),
    views!("03_moth_mechanic"),
    views!("04_velvet_bones"),
    views!("05_spore_scout"),
    views!("06_canid_courier"),
    views!("07_puddle_kid"),
    views!("08_aviary_pilot"),
    views!("09_tortoise_miner"),
];

static TEXTURES: OnceLock<Vec<TextureAsset>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct ViewMetadata {
    dimensions: [u32; 2],
    /// Top-left pixel origin, right/bottom exclusive, alpha >= 16.
    alpha_bounds: [u32; 4],
    floor_anchor_px: [f32; 2],
    /// Cushion contact under the posterior thigh, not the waist/torso center.
    seat_anchor_px: Option<[f32; 2]>,
    /// Shared per-identity cushion-to-crown target for all three seated views.
    head_above_seat: Option<f32>,
    visible_height: f32,
}

fn metadata() -> &'static [ViewMetadata] {
    static METADATA: OnceLock<Vec<ViewMetadata>> = OnceLock::new();
    METADATA.get_or_init(|| {
        let views: Vec<ViewMetadata> =
            serde_json::from_str(include_str!("../assets/avatars/metadata/views.json"))
                .expect("bundled avatar calibration must parse");
        assert_eq!(views.len(), TEXTURE_COUNT);
        views
    })
}

pub(crate) fn texture_assets() -> Vec<TextureAsset> {
    // Decode optimized PNGs once at their original canvas dimensions. Cloned decoded
    // pools are intentionally retained until the separately deferred memory pass.
    TEXTURES
        .get_or_init(|| {
            AVATAR_PNGS
                .iter()
                .flatten()
                .map(|png| {
                    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                        .expect("bundled avatar PNG must decode")
                        .into_rgba8();
                    TextureAsset {
                        mipmaps: Vec::new(),
                        handle: SpriteHandle::new(),
                        width: image.width(),
                        height: image.height(),
                        rgba: image.into_raw(),
                    }
                })
                .collect()
        })
        .clone()
}

pub(crate) fn handles(textures: &[TextureAsset]) -> [AvatarTextures; AVATAR_COUNT] {
    assert_eq!(
        textures.len(),
        TEXTURE_COUNT,
        "expected the avatar-only pool"
    );
    std::array::from_fn(|index| {
        let offset = index * 6;
        AvatarTextures {
            standing: std::array::from_fn(|view| textures[offset + view].handle),
            seated: std::array::from_fn(|view| textures[offset + 3 + view].handle),
        }
    })
}

#[cfg(test)]
pub(crate) fn index_from_avatar_id(id: &str) -> Option<usize> {
    (1..=AVATAR_COUNT)
        .find(|index| id == format!("avatar_{index:02}"))
        .map(|index| index - 1)
}

pub(crate) fn avatar_index(entity_id: u64) -> usize {
    // SplitMix64 finalizer spreads sequential server IDs across the pool. As with
    // the old entity-derived colors, every viewer independently gets the same
    // assignment; no extra profile message or pose bytes are needed.
    let mut hash = entity_id.wrapping_add(0x9e37_79b9_7f4a_7c15);
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;
    (hash % AVATAR_COUNT as u64) as usize
}

fn horizontal_unit(vector: Vec3) -> Option<Vec3> {
    Vec3::new(vector.x, 0.0, vector.z).try_normalize()
}

/// Returns [front, back, left] index, UV mirror flag, and billboard normal.
fn direction(eye: Vec3, viewer: Vec3, forward: Vec3, seated: bool) -> (usize, bool, Vec3) {
    let forward = if seated {
        Vec3::NEG_Z // Chairs face the screen, not the occupant's free-look target.
    } else {
        horizontal_unit(forward).unwrap_or(Vec3::NEG_Z)
    };
    let to_viewer = horizontal_unit(viewer - eye).unwrap_or(forward);
    let front = forward.dot(to_viewer);
    let left = Vec3::Y.cross(forward).dot(to_viewer);
    // Deterministic 45-degree ties favor front/back. Coincident/vertical viewers
    // see front; malformed standing forward falls back to the chair convention.
    if front.abs() >= left.abs() {
        (usize::from(front < 0.0), false, to_viewer)
    } else {
        (2, left < 0.0, to_viewer)
    }
}

// Safety only: callers own amplitude, timing, and locomotion state. Radians.
const MAX_ROCK: f32 = 0.18;
const SEAT_HEIGHT: f32 = 0.48;
// Source proportions require at most 0.068238 m; allow a small numeric margin.
#[cfg(test)]
const SEATED_BOOT_FLOOR_TOLERANCE: f32 = 0.069;

pub(crate) fn sprite(
    textures: AvatarTextures,
    eye: Vec3,
    viewer: Vec3,
    forward: Vec3,
    seated: bool,
    rock: f32,
) -> SpriteInstance {
    let eye = if eye.is_finite() { eye } else { Vec3::ZERO };
    let (view, mirrored, to_viewer) = direction(eye, viewer, forward, seated);
    let handle = if seated {
        textures.seated
    } else {
        textures.standing
    }[view];
    let slot = usize::from(seated) * 3 + view;
    let calibration_index = TEXTURES
        .get()
        .and_then(|pool| pool.iter().position(|texture| texture.handle == handle))
        .unwrap_or(slot);
    // Unknown handles get the corresponding first-character calibration without
    // initializing/decoding a texture pool during scene submission.
    calibrated_sprite(
        handle,
        &metadata()[calibration_index],
        eye,
        to_viewer,
        seated,
        mirrored,
        rock,
    )
}

fn calibrated_sprite(
    handle: SpriteHandle,
    calibration: &ViewMetadata,
    eye: Vec3,
    to_viewer: Vec3,
    seated: bool,
    mirrored: bool,
    rock: f32,
) -> SpriteInstance {
    let width = calibration.dimensions[0] as f32;
    let height = calibration.dimensions[1] as f32;
    let visible_pixels = (calibration.alpha_bounds[3] - calibration.alpha_bounds[1]) as f32;
    let (anchor, scale) = if seated {
        let seat = calibration
            .seat_anchor_px
            .expect("seated avatar must have a calibrated cushion contact");
        let head_above_seat = calibration
            .head_above_seat
            .expect("seated avatar must have a shared cushion-to-crown height");
        let support_to_crown = seat[1] - calibration.alpha_bounds[1] as f32;
        (seat, head_above_seat * height / support_to_crown)
    } else {
        (
            calibration.floor_anchor_px,
            calibration.visible_height * height / visible_pixels,
        )
    };
    let anchor_x = anchor[0] / width - 0.5;
    // Seated pelvis contacts the cushion and crown height stays stable across
    // views. Source proportions retain a bounded boot-floor error instead of
    // changing head size to force every painted boot onto the floor.
    // The backend scales world quad X by texture aspect, independently of UV sign.
    let local_anchor = Vec3::new(
        (if mirrored { -anchor_x } else { anchor_x }) * width / height,
        0.5 - anchor[1] / height,
        0.0,
    ) * scale;
    let yaw = Quat::from_rotation_y(to_viewer.x.atan2(to_viewer.z));
    let rock = if !seated && rock.is_finite() {
        rock.clamp(-MAX_ROCK, MAX_ROCK)
    } else {
        0.0
    };
    let rotation = yaw * Quat::from_rotation_z(rock);
    let floor = eye - Vec3::Y * if seated { 1.25 } else { 1.7 };
    let world_anchor = floor + Vec3::Y * if seated { SEAT_HEIGHT } else { 0.0 };
    SpriteInstance {
        sprite: handle,
        transform: RenderTransform {
            // World sprite pivot is ignored by WGPU. Standing rolls around the
            // grounded foot; seated pelvis remains centered on the cushion.
            translation: world_anchor - rotation * local_anchor,
            rotation,
            scale,
        },
        uv_rect: if mirrored {
            [1.0, 0.0, -1.0, 1.0]
        } else {
            [0.0, 0.0, 1.0, 1.0]
        },
        space: SpriteSpace::World,
        ..SpriteInstance::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_handles() -> AvatarTextures {
        AvatarTextures {
            standing: std::array::from_fn(|_| SpriteHandle::new()),
            seated: std::array::from_fn(|_| SpriteHandle::new()),
        }
    }

    #[test]
    fn selection_is_stable_bounded_and_uses_all_nine_avatars() {
        let selections: std::collections::BTreeSet<_> = (1..=1000)
            .map(|id| {
                let index = avatar_index(id);
                assert_eq!(index, avatar_index(id));
                assert!(index < AVATAR_COUNT);
                index
            })
            .collect();
        assert_eq!(selections.len(), AVATAR_COUNT);
        assert_eq!(avatar_index(1), 5);
        assert_eq!(avatar_index(2), 4);
        assert!(avatar_index(u64::MAX) < AVATAR_COUNT);
    }

    #[test]
    fn authored_avatar_ids_are_exact_and_bounded_to_the_nine_portraits() {
        for index in 0..AVATAR_COUNT {
            assert_eq!(
                index_from_avatar_id(&format!("avatar_{:02}", index + 1)),
                Some(index)
            );
        }
        for invalid in [
            "",
            "avatar_00",
            "avatar_10",
            "avatar_1",
            "avatar_001",
            "avatar_09\n",
            "npc_concession_robot",
            "guest_01",
        ] {
            assert_eq!(index_from_avatar_id(invalid), None);
        }
    }

    // Only this test clones the full pool. Geometry/selection tests use synthetic
    // handles and metadata, so parallel tests do not clone giant decoded pools.
    #[test]
    fn complete_runtime_inventory_has_transparency_calibration_and_ordered_handles() {
        let textures = texture_assets();
        let handles = handles(&textures);
        let inventory: serde_json::Value =
            serde_json::from_str(include_str!("../assets/avatars/metadata/inventory.json"))
                .unwrap();
        let entries = inventory["assets"].as_array().unwrap();
        let views: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../assets/avatars/metadata/views.json")).unwrap();
        assert_eq!(textures.len(), TEXTURE_COUNT);
        assert_eq!(entries.len(), TEXTURE_COUNT);
        for (index, texture) in textures.iter().enumerate() {
            let calibration = &metadata()[index];
            assert_eq!([texture.width, texture.height], calibration.dimensions);
            assert_eq!(
                texture.rgba.len(),
                (texture.width * texture.height * 4) as usize
            );
            assert_eq!(entries[index]["path"], views[index]["path"]);
            assert_eq!(
                entries[index]["bytes"].as_u64().unwrap(),
                AVATAR_PNGS[index / 6][index % 6].len() as u64
            );
            assert_eq!(
                views[index]["avatar_id"],
                format!("avatar_{:02}", index / 6 + 1)
            );
            assert_eq!(
                views[index]["view"],
                [
                    "standing_front",
                    "standing_back",
                    "standing_left",
                    "seated_front",
                    "seated_back",
                    "seated_left"
                ][index % 6]
            );

            assert!(
                textures[..index]
                    .iter()
                    .all(|other| other.handle != texture.handle)
            );
            let avatar = handles[index / 6];
            assert_eq!(
                [avatar.standing, avatar.seated][index % 6 / 3][index % 3],
                texture.handle
            );
            let mut transparent = false;
            let mut solid = false;
            let mut bounds = [texture.width, texture.height, 0, 0];
            let mut alpha_range = [u8::MAX, u8::MIN];
            for (pixel_index, pixel) in texture.rgba.as_chunks::<4>().0.iter().enumerate() {
                transparent |= pixel[3] == 0;
                solid |= pixel[3] >= 128;
                alpha_range[0] = alpha_range[0].min(pixel[3]);
                alpha_range[1] = alpha_range[1].max(pixel[3]);
                if pixel[3] >= 16 {
                    let x = pixel_index as u32 % texture.width;
                    let y = pixel_index as u32 / texture.width;
                    bounds = [
                        bounds[0].min(x),
                        bounds[1].min(y),
                        bounds[2].max(x + 1),
                        bounds[3].max(y + 1),
                    ];
                }
            }
            assert!(transparent && solid);
            assert_eq!(
                entries[index]["alpha_range"],
                serde_json::json!(alpha_range)
            );
            assert_eq!(entries[index]["alpha_bounds"], serde_json::json!(bounds));
            // Keep source-pixel calibration fixed despite palette quantization at fringes.
            assert_eq!(
                entries[index]["source_alpha_bounds"],
                serde_json::json!(calibration.alpha_bounds)
            );
            let source_bounds = calibration.alpha_bounds;
            for (actual, calibrated) in bounds.into_iter().zip(source_bounds) {
                assert!(actual.abs_diff(calibrated) <= 1);
            }
            assert!(calibration.floor_anchor_px[1] <= source_bounds[3] as f32);
            assert!(calibration.floor_anchor_px[1] > source_bounds[1] as f32);
            assert_eq!(calibration.seat_anchor_px.is_some(), index % 6 >= 3);
            assert!(
                (source_bounds[0] as f32..=source_bounds[2] as f32)
                    .contains(&calibration.floor_anchor_px[0])
            );
            let selected = sprite(avatar, Vec3::Y * 1.7, Vec3::NEG_Z, Vec3::NEG_Z, false, 0.0);
            let expected = &metadata()[index / 6 * 6];
            assert!(
                (selected.transform.scale
                    - expected.visible_height * expected.dimensions[1] as f32
                        / (expected.alpha_bounds[3] - expected.alpha_bounds[1]) as f32)
                    .abs()
                    < 1e-5
            );
        }
    }

    #[test]
    fn inventory_preserves_source_provenance_without_claiming_lossless_pixels() {
        let inventory: serde_json::Value =
            serde_json::from_str(include_str!("../assets/avatars/metadata/inventory.json"))
                .unwrap();
        let standing: serde_json::Value = serde_json::from_str(include_str!(
            "../assets/avatars/docs/standing_left_back_all_manifest.json"
        ))
        .unwrap();
        let seated: serde_json::Value = serde_json::from_str(include_str!(
            "../assets/avatars/docs/seated_all_manifest.json"
        ))
        .unwrap();
        let entries = inventory["assets"].as_array().unwrap();
        let fronts = inventory["front_reference_comparison"].as_array().unwrap();
        assert_eq!(entries.len(), TEXTURE_COUNT);
        assert_eq!(fronts.len(), AVATAR_COUNT);
        assert_eq!(inventory["optimization"]["lossless"], false);
        let mut source_bytes = 0;
        let mut runtime_bytes = 0;
        for (index, entry) in entries.iter().enumerate() {
            for field in [
                "sha256",
                "source_sha256",
                "rgba_sha256",
                "source_rgba_sha256",
            ] {
                let hash = entry[field].as_str().unwrap();
                assert_eq!(hash.len(), 64);
                assert!(
                    hash.bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                );
            }
            // Encoding hashes alone cannot establish whether a PNG optimization was lossless.
            assert_ne!(entry["sha256"], entry["source_sha256"]);
            assert_ne!(entry["rgba_sha256"], entry["source_rgba_sha256"]);
            assert!(entry["source_bytes"].as_u64().unwrap() > entry["bytes"].as_u64().unwrap());
            source_bytes += entry["source_bytes"].as_u64().unwrap();
            runtime_bytes += entry["bytes"].as_u64().unwrap();
            if index % 6 == 0 {
                let reference = &fronts[index / 6];
                assert_eq!(entry["source_sha256"], reference["repository_sha256"]);
                let path = reference["repository_path"].as_str().unwrap();
                assert!(
                    entry["path"]
                        .as_str()
                        .unwrap()
                        .ends_with(&format!("/{path}"))
                );
                assert_eq!(
                    entry["source"],
                    format!(
                        "git:{}:examples/first-person-lab/assets/avatars/{path}",
                        inventory["source_repository_revision"].as_str().unwrap()
                    )
                );
                let upstream = seated["assets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|asset| {
                        asset["standing_reference"] == reference["upstream_standing_reference"]
                    })
                    .unwrap();
                assert_eq!(reference["matches"], false);
                assert_ne!(
                    reference["repository_sha256"],
                    reference["upstream_standing_reference_sha256"]
                );
                assert_eq!(
                    reference["upstream_standing_reference_sha256"],
                    upstream["standing_reference_sha256"]
                );
            } else {
                let manifest = if index % 6 < 3 { &standing } else { &seated };
                let source = manifest["assets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|asset| asset["filename"] == entry["path"])
                    .unwrap();
                assert_eq!(entry["source_sha256"], source["sha256"]);
                assert_eq!(entry["dimensions"], source["dimensions"]);
                assert!(
                    entry["source"]
                        .as_str()
                        .unwrap()
                        .ends_with(entry["path"].as_str().unwrap())
                );
                if index % 6 >= 3 {
                    assert_eq!(entry["source_alpha_range"], source["alpha_range"]);
                    assert_eq!(
                        entry["source_alpha_bounds"],
                        source["content_bbox_alpha_ge_16"]
                    );
                }
            }
        }
        assert_eq!(inventory["optimization"]["source_png_bytes"], source_bytes);
        assert_eq!(
            inventory["optimization"]["runtime_png_bytes"],
            runtime_bytes
        );
    }

    #[test]
    fn four_directions_follow_horizontal_body_forward_with_deterministic_fallbacks() {
        let t = fake_handles();
        for forward in [Vec3::NEG_Z, Vec3::X, Vec3::Z, Vec3::NEG_X] {
            let left = Vec3::Y.cross(forward);
            for (viewer, view, mirrored) in [
                (forward, 0, false),
                (-forward, 1, false),
                (left, 2, false),
                (-left, 2, true),
            ] {
                let s = sprite(
                    t,
                    Vec3::ZERO,
                    viewer * 4.0 + Vec3::Y * 20.0,
                    forward,
                    false,
                    0.0,
                );
                assert_eq!(s.sprite, t.standing[view]);
                assert_eq!(
                    s.uv_rect,
                    if mirrored {
                        [1.0, 0.0, -1.0, 1.0]
                    } else {
                        [0.0, 0.0, 1.0, 1.0]
                    }
                );
                assert!((s.transform.rotation * Vec3::Z - viewer).length() < 1e-5);
            }
        }
        for forward in [
            Vec3::ZERO,
            Vec3::Y,
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
        ] {
            assert_eq!(direction(Vec3::ZERO, Vec3::NEG_Z, forward, false).0, 0);
        }
        for viewer in [Vec3::ZERO, Vec3::Y, Vec3::splat(f32::NAN)] {
            assert_eq!(direction(Vec3::ZERO, viewer, Vec3::NEG_Z, false).0, 0);
        }
        assert_eq!(
            direction(Vec3::ZERO, Vec3::new(-1.0, 0.0, -1.0), Vec3::NEG_Z, false).0,
            0
        );
        assert_eq!(
            direction(Vec3::ZERO, Vec3::new(-1.0, 0.0, 1.0), Vec3::NEG_Z, false).0,
            1
        );
    }

    #[test]
    fn seated_body_is_independent_of_look_and_uses_full_art() {
        let t = fake_handles();
        for viewer in [Vec3::NEG_Z, Vec3::Z, Vec3::NEG_X, Vec3::X] {
            let expected = sprite(t, Vec3::Y * 1.25, viewer, Vec3::NEG_Z, true, 0.0);
            for look in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO, Vec3::splat(f32::NAN)] {
                for rock in [-100.0, -0.08, 0.0, 0.08, 100.0, f32::NAN, f32::INFINITY] {
                    let actual = sprite(t, Vec3::Y * 1.25, viewer, look, true, rock);
                    assert_eq!(actual, expected);
                    assert_eq!(actual.uv_rect[1], 0.0);
                    assert_eq!(actual.uv_rect[3], 1.0);
                }
            }
        }
    }

    #[test]
    fn all_views_preserve_aspect_height_and_grounded_foot_under_local_roll() {
        for (index, calibration) in metadata().iter().enumerate() {
            let seated = index % 6 >= 3;
            assert!(
                (if seated { 1.25..=1.80 } else { 2.16..=2.21 })
                    .contains(&calibration.visible_height)
            );
            let eye = Vec3::new(4.0, if seated { 2.25 } else { 2.7 }, -3.0);
            let floor = Vec3::new(4.0, 1.0, -3.0);
            let normal = Vec3::new(0.6, 0.0, 0.8);
            let yaw = Quat::from_rotation_y(normal.x.atan2(normal.z));
            for mirrored in [false, true] {
                for rock in [-100.0, -0.08, 0.0, 0.08, 100.0, f32::NAN, f32::INFINITY] {
                    let s = calibrated_sprite(
                        SpriteHandle::new(),
                        calibration,
                        eye,
                        normal,
                        seated,
                        mirrored,
                        rock,
                    );
                    let width = calibration.dimensions[0] as f32;
                    let height = calibration.dimensions[1] as f32;
                    let x = calibration.floor_anchor_px[0] / width - 0.5;
                    let foot = Vec3::new(
                        (if mirrored { -x } else { x }) * width / height,
                        0.5 - calibration.floor_anchor_px[1] / height,
                        0.0,
                    );
                    let world_foot =
                        s.transform.translation + s.transform.rotation * (foot * s.transform.scale);
                    if seated {
                        assert!((world_foot.y - floor.y).abs() <= SEATED_BOOT_FLOOR_TOLERANCE);
                    } else {
                        assert!(world_foot.distance(floor) < 1e-5);
                    }
                    let visible_height =
                        (calibration.alpha_bounds[3] - calibration.alpha_bounds[1]) as f32 / height
                            * s.transform.scale;
                    assert!((visible_height - calibration.visible_height).abs() < 1e-5);
                    let clamped = if !seated && rock.is_finite() {
                        rock.clamp(-MAX_ROCK, MAX_ROCK)
                    } else {
                        0.0
                    };
                    assert!(
                        s.transform
                            .rotation
                            .dot(yaw * Quat::from_rotation_z(clamped))
                            .abs()
                            > 0.99999
                    );
                    assert!(s.transform.translation.is_finite());
                    assert!(s.transform.rotation.is_finite());
                    assert!(s.transform.scale.is_finite());
                }
            }
        }
    }

    #[test]
    fn all_seated_pelves_center_on_cushion_with_stable_crowns_and_bounded_boot_error() {
        for calibration in metadata()
            .iter()
            .filter(|view| view.seat_anchor_px.is_some())
        {
            let seat = calibration.seat_anchor_px.unwrap();
            let [width, height] = calibration.dimensions.map(|value| value as f32);
            let [left, top, right, bottom] = calibration.alpha_bounds.map(|value| value as f32);
            assert!((left..right).contains(&seat[0]));
            assert!((top..bottom).contains(&seat[1]));
            // Contact is the lower thigh/pelvis surface, not a mid-torso pivot.
            assert!(((seat[1] - top) / (bottom - top)) > 0.60);
            let floor = Vec3::new(4.0, 2.0, -3.0);
            for normal in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
                for mirrored in [false, true] {
                    let s = calibrated_sprite(
                        SpriteHandle::new(),
                        calibration,
                        floor + Vec3::Y * 1.25,
                        normal,
                        true,
                        mirrored,
                        100.0,
                    );
                    let world_pixel = |pixel: [f32; 2]| {
                        let x = pixel[0] / width - 0.5;
                        let local = Vec3::new(
                            (if mirrored { -x } else { x }) * width / height,
                            0.5 - pixel[1] / height,
                            0.0,
                        );
                        s.transform.translation + s.transform.rotation * (local * s.transform.scale)
                    };
                    assert!(world_pixel(seat).distance(floor + Vec3::Y * SEAT_HEIGHT) < 1e-5);
                    let boot = world_pixel(calibration.floor_anchor_px);
                    assert!((boot.y - floor.y).abs() <= SEATED_BOOT_FLOOR_TOLERANCE);
                    let expected_offset = (calibration.floor_anchor_px[0] - seat[0]) / height
                        * s.transform.scale
                        * if mirrored { -1.0 } else { 1.0 };
                    let right = s.transform.rotation * Vec3::X;
                    assert!(((boot - floor).dot(right) - expected_offset).abs() < 1e-5);
                    let top_y = world_pixel([seat[0], top]).y;
                    let crown = floor.y + SEAT_HEIGHT + calibration.head_above_seat.unwrap();
                    assert!((top_y - crown).abs() < 1e-5);
                    assert!(top_y < floor.y + 1.25 + 0.56);
                    assert!(top_y > floor.y + 1.25);
                }
            }
        }
    }

    #[test]
    fn seated_identity_shares_minimax_crown_height_across_all_three_views() {
        for avatar in metadata().as_chunks::<6>().0 {
            let seated = &avatar[3..];
            let exact_fit_torsos: Vec<_> = seated
                .iter()
                .map(|view| {
                    let seat_y = view.seat_anchor_px.unwrap()[1];
                    SEAT_HEIGHT * (seat_y - view.alpha_bounds[1] as f32)
                        / (view.floor_anchor_px[1] - seat_y)
                })
                .collect();
            let min = exact_fit_torsos
                .iter()
                .copied()
                .fold(f32::INFINITY, f32::min);
            let max = exact_fit_torsos
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            let target = 2.0 * min * max / (min + max);
            let shared = seated[0].head_above_seat.unwrap();
            assert!((shared - target).abs() < 1e-6);
            let mut crown_heights = Vec::new();
            for view in seated {
                assert_eq!(view.head_above_seat.unwrap(), shared);
                let sprite = calibrated_sprite(
                    SpriteHandle::new(),
                    view,
                    Vec3::Y * 1.25,
                    Vec3::Z,
                    true,
                    false,
                    0.0,
                );
                let top_local = 0.5 - view.alpha_bounds[1] as f32 / view.dimensions[1] as f32;
                crown_heights
                    .push(sprite.transform.translation.y + top_local * sprite.transform.scale);
            }
            for crown in crown_heights {
                assert!((crown - (SEAT_HEIGHT + shared)).abs() < 1e-5);
            }
            let min_error = SEAT_HEIGHT * (1.0 - shared / min);
            let max_error = SEAT_HEIGHT * (1.0 - shared / max);
            assert!((min_error + max_error).abs() < 1e-6);
            assert!(min_error.abs() <= SEATED_BOOT_FLOOR_TOLERANCE);
            assert!(max_error.abs() <= SEATED_BOOT_FLOOR_TOLERANCE);
        }
    }

    #[test]
    fn malformed_pose_inputs_still_produce_finite_transforms() {
        let t = fake_handles();
        for value in [
            Vec3::ZERO,
            Vec3::Y,
            Vec3::splat(f32::NAN),
            Vec3::splat(f32::INFINITY),
            Vec3::splat(f32::MAX),
        ] {
            let s = sprite(t, value, value, value, false, f32::NAN);
            assert!(s.transform.translation.is_finite());
            assert!(s.transform.rotation.is_finite());
            assert!(s.transform.scale.is_finite());
        }
    }
}
