//! Stable entity-derived avatar selection, sharing the existing pose identity.

use std::sync::OnceLock;
use weaver_app_core::TextureAsset;
use weaver_render::SpriteHandle;

pub(crate) const AVATAR_COUNT: usize = 9;

const AVATAR_PNGS: [&[u8]; AVATAR_COUNT] = [
    include_bytes!("../assets/avatars/avatar_01_rust_drifter.png"),
    include_bytes!("../assets/avatars/avatar_02_chrome_usher.png"),
    include_bytes!("../assets/avatars/avatar_03_moth_mechanic.png"),
    include_bytes!("../assets/avatars/avatar_04_velvet_bones.png"),
    include_bytes!("../assets/avatars/avatar_05_spore_scout.png"),
    include_bytes!("../assets/avatars/avatar_06_canid_courier.png"),
    include_bytes!("../assets/avatars/avatar_07_puddle_kid.png"),
    include_bytes!("../assets/avatars/avatar_08_aviary_pilot.png"),
    include_bytes!("../assets/avatars/avatar_09_tortoise_miner.png"),
];

pub(crate) fn texture_assets() -> Vec<TextureAsset> {
    // Decode once per runtime, not per player or frame (or repeated test app).
    static TEXTURES: OnceLock<Vec<TextureAsset>> = OnceLock::new();
    TEXTURES
        .get_or_init(|| {
            AVATAR_PNGS
                .iter()
                .map(|png| {
                    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                        .expect("bundled avatar PNG must decode")
                        .into_rgba8();
                    TextureAsset {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn bundled_pool_contains_only_nine_transparent_portrait_textures() {
        let textures = texture_assets();
        assert_eq!(textures.len(), AVATAR_COUNT);
        for texture in textures {
            assert_eq!((texture.width, texture.height), (1086, 1448));
            assert_eq!(texture.rgba.len(), 1086 * 1448 * 4);
            let (pixels, remainder) = texture.rgba.as_chunks::<4>();
            assert!(remainder.is_empty());
            assert!(pixels.iter().any(|pixel| pixel[3] == 0));
            assert!(pixels.iter().any(|pixel| pixel[3] == 255));
        }
    }
}
