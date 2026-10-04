//! Bounded display-name generation and local user identifiers.
//!
//! These helpers are shared by native and browser callers. Neither a generated
//! name nor a UUID establishes an authenticated identity or a Woven entity ID.

use thiserror::Error;
use uuid::Uuid;

const MAX_ADJECTIVES: usize = 8;
const MAX_ADJECTIVE_BYTES: usize = 8;
const MAX_AFFIX_BYTES: usize = 64;

const ADJECTIVES: [&str; 64] = [
    "Able", "Agile", "Alert", "Amiable", "Bold", "Brave", "Breezy", "Bright", "Brisk", "Bubbly",
    "Calm", "Capable", "Careful", "Cheerful", "Clever", "Cool", "Cozy", "Curious", "Daring",
    "Eager", "Earnest", "Fair", "Fancy", "Fearless", "Festive", "Friendly", "Gentle", "Glad",
    "Gleeful", "Graceful", "Happy", "Helpful", "Honest", "Hopeful", "Humble", "Jolly", "Joyful",
    "Keen", "Kind", "Lively", "Loyal", "Lucky", "Mellow", "Merry", "Mindful", "Modest", "Nimble",
    "Noble", "Patient", "Playful", "Polite", "Radiant", "Relaxed", "Serene", "Sincere", "Smart",
    "Steady", "Stellar", "Sunny", "Swift", "Upbeat", "Vibrant", "Warm", "Witty",
];

/// Invalid options supplied to [`generate_user_name`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum UserNameError {
    /// The adjective count is outside the supported range of 1 through 8.
    #[error("adjective count must be between 1 and 8 (received {count})")]
    InvalidAdjectiveCount {
        /// The requested adjective count.
        count: usize,
    },
    /// The prefix exceeds its 64-byte limit.
    #[error("user-name prefix must be at most 64 bytes (received {bytes})")]
    PrefixTooLong {
        /// The prefix's UTF-8 byte length.
        bytes: usize,
    },
    /// The postfix exceeds its 64-byte limit.
    #[error("user-name postfix must be at most 64 bytes (received {bytes})")]
    PostfixTooLong {
        /// The postfix's UTF-8 byte length.
        bytes: usize,
    },
    /// The prefix contains a Unicode control character.
    #[error("user-name prefix must not contain control characters")]
    PrefixContainsControl,
    /// The postfix contains a Unicode control character.
    #[error("user-name postfix must not contain control characters")]
    PostfixContainsControl,
}

/// Generate a display name from distinct, friendly CamelCase adjectives.
///
/// For the usual two-adjective default, call `generate_user_name(2, "", "")`.
/// This produces names such as `BrightCalm`, with no noun and at most 16 ASCII
/// characters, comfortably below the labs' 24-character display-name limit.
/// Names are not guaranteed to be unique across calls or users.
///
/// Counts from 1 through 8 are supported. Each dictionary word is at most 8
/// ASCII bytes. The prefix and postfix are preserved verbatim, may each contain
/// at most 64 UTF-8 bytes, and must not contain Unicode control characters.
/// Valid output is therefore bounded to 192 bytes; callers with tighter limits
/// must account for their chosen count and affixes.
///
/// Selection uses secure platform randomness through the UUID crate, without
/// retries. At most eight selections are made, with no repeated adjective in
/// one name.
///
/// # Errors
///
/// Returns [`UserNameError`] for an unsupported adjective count, an oversized
/// affix, or an affix containing a control character. Validation precedes
/// allocation and random generation.
///
/// # Panics
///
/// Panics if the platform's secure random source is unavailable, rather than
/// falling back to an insecure random source.
///
/// # Examples
///
/// ```
/// let name = weaver_core::generate_user_name(2, "", "")?;
/// assert!(name.len() <= 16);
/// # Ok::<(), weaver_core::UserNameError>(())
/// ```
pub fn generate_user_name(
    adjectives: usize,
    prefix: &str,
    postfix: &str,
) -> Result<String, UserNameError> {
    if !(1..=MAX_ADJECTIVES).contains(&adjectives) {
        return Err(UserNameError::InvalidAdjectiveCount { count: adjectives });
    }
    if prefix.len() > MAX_AFFIX_BYTES {
        return Err(UserNameError::PrefixTooLong {
            bytes: prefix.len(),
        });
    }
    if postfix.len() > MAX_AFFIX_BYTES {
        return Err(UserNameError::PostfixTooLong {
            bytes: postfix.len(),
        });
    }
    if prefix.chars().any(char::is_control) {
        return Err(UserNameError::PrefixContainsControl);
    }
    if postfix.chars().any(char::is_control) {
        return Err(UserNameError::PostfixContainsControl);
    }

    let mut name =
        String::with_capacity(prefix.len() + adjectives * MAX_ADJECTIVE_BYTES + postfix.len());
    name.push_str(prefix);
    let mut choices = ADJECTIVES;
    for selected in 0..adjectives {
        // Partial Fisher-Yates sampling avoids an unbounded duplicate-retry loop.
        let remaining = choices.len() - selected;
        let offset = (Uuid::new_v4().as_u128() % remaining as u128) as usize;
        choices.swap(selected, selected + offset);
        name.push_str(choices[selected]);
    }
    name.push_str(postfix);
    Ok(name)
}

/// Generate a fresh lowercase, hyphenated `UUIDv4` user identifier.
///
/// Uses the standard UUID crate's secure OS randomness on native platforms and
/// secure browser randomness on WebAssembly via its `js` feature. Every call
/// generates a new random identifier. Callers needing a stable local user ID
/// should generate it once, persist it, and reuse the stored value.
///
/// This identifier is not an authenticated identity, credential, or Woven
/// entity ID. Random UUIDs have negligible collision probability, not a
/// mathematical guarantee of uniqueness.
///
/// # Panics
///
/// Panics if the platform's secure random source is unavailable, rather than
/// falling back to an insecure random source.
///
/// # Examples
///
/// ```
/// let user_id = weaver_core::generate_user_id();
/// assert_eq!(user_id.len(), 36);
/// ```
#[must_use]
pub fn generate_user_id() -> String {
    Uuid::new_v4().hyphenated().to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::{Variant, Version};

    use super::*;

    fn words(name: &str) -> Vec<&str> {
        let mut words = Vec::new();
        let mut start = 0;
        for (index, character) in name.char_indices().skip(1) {
            if character.is_ascii_uppercase() {
                words.push(&name[start..index]);
                start = index;
            }
        }
        words.push(&name[start..]);
        words
    }

    #[test]
    fn dictionary_is_unique_friendly_camel_case_and_bounded() {
        assert!(ADJECTIVES.len() >= MAX_ADJECTIVES);
        let mut seen = HashSet::new();
        for word in ADJECTIVES {
            assert!(!word.is_empty());
            assert!(word.len() <= MAX_ADJECTIVE_BYTES, "oversized word: {word}");
            assert!(word.as_bytes()[0].is_ascii_uppercase());
            assert!(word.as_bytes()[1..].iter().all(u8::is_ascii_lowercase));
            assert!(seen.insert(word), "duplicate word: {word}");
        }
    }

    #[test]
    fn generates_exact_adjective_counts_without_repeats_or_nouns() {
        for count in 1..=MAX_ADJECTIVES {
            for _ in 0..32 {
                let name = generate_user_name(count, "", "").unwrap();
                let words = words(&name);
                assert_eq!(words.len(), count);
                assert!(words.iter().all(|word| ADJECTIVES.contains(word)));
                assert_eq!(words.iter().copied().collect::<HashSet<_>>().len(), count);
                assert!(name.len() <= count * MAX_ADJECTIVE_BYTES);
            }
        }
    }

    #[test]
    fn two_adjective_default_fits_lab_display_name_limit() {
        let name = generate_user_name(2, "", "").unwrap();
        assert_eq!(words(&name).len(), 2);
        assert!(name.len() <= 16);
        assert!(name.len() < 24);
    }

    #[test]
    fn preserves_affixes_verbatim() {
        let prefix = "Guest-友 ";
        let postfix = " #42✨";
        let name = generate_user_name(3, prefix, postfix).unwrap();
        let adjectives = name
            .strip_prefix(prefix)
            .unwrap()
            .strip_suffix(postfix)
            .unwrap();
        assert_eq!(words(adjectives).len(), 3);
    }

    #[test]
    fn rejects_invalid_adjective_counts() {
        for count in [0, MAX_ADJECTIVES + 1, usize::MAX] {
            assert_eq!(
                generate_user_name(count, "", ""),
                Err(UserNameError::InvalidAdjectiveCount { count })
            );
        }
    }

    #[test]
    fn affix_byte_limits_accept_boundaries_and_reject_oversize() {
        for boundary in ["a".repeat(64), "é".repeat(32)] {
            let name = generate_user_name(8, &boundary, &boundary).unwrap();
            assert!(name.starts_with(&boundary));
            assert!(name.ends_with(&boundary));
            assert!(name.len() <= 192);
        }
        for oversized in ["a".repeat(65), "é".repeat(33)] {
            assert_eq!(
                generate_user_name(2, &oversized, ""),
                Err(UserNameError::PrefixTooLong {
                    bytes: oversized.len()
                })
            );
            assert_eq!(
                generate_user_name(2, "", &oversized),
                Err(UserNameError::PostfixTooLong {
                    bytes: oversized.len()
                })
            );
        }
    }

    #[test]
    fn rejects_ascii_and_unicode_controls_in_either_affix() {
        for control in ['\0', '\n', '\r', '\t', '\u{7f}', '\u{85}', '\u{9f}'] {
            let affix = format!("before{control}after");
            assert_eq!(
                generate_user_name(2, &affix, ""),
                Err(UserNameError::PrefixContainsControl)
            );
            assert_eq!(
                generate_user_name(2, "", &affix),
                Err(UserNameError::PostfixContainsControl)
            );
        }
    }

    #[test]
    fn validation_errors_are_descriptive() {
        let error = generate_user_name(0, "", "").unwrap_err().to_string();
        assert!(error.contains("between 1 and 8"));
        assert!(error.contains("received 0"));
        let error = generate_user_name(2, &"a".repeat(65), "")
            .unwrap_err()
            .to_string();
        assert!(error.contains("prefix"));
        assert!(error.contains("64 bytes"));
        assert!(error.contains("received 65"));
        let error = generate_user_name(2, "", "\n").unwrap_err().to_string();
        assert!(error.contains("postfix"));
        assert!(error.contains("control characters"));
    }

    #[test]
    fn user_ids_are_lowercase_hyphenated_rfc4122_uuid_v4() {
        let id = generate_user_id();
        assert_eq!(id.len(), 36);
        for (index, byte) in id.bytes().enumerate() {
            if [8, 13, 18, 23].contains(&index) {
                assert_eq!(byte, b'-');
            } else {
                assert!(byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            }
        }
        assert_eq!(id.as_bytes()[14], b'4');
        assert!(b"89ab".contains(&id.as_bytes()[19]));
        let parsed = Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version(), Some(Version::Random));
        assert_eq!(parsed.get_variant(), Variant::RFC4122);
        assert_eq!(parsed.hyphenated().to_string(), id);
    }

    #[test]
    fn repeated_user_id_generation_produces_fresh_ids() {
        let mut seen = HashSet::new();
        for _ in 0..1024 {
            let id = generate_user_id();
            assert!(seen.insert(id), "unexpected duplicate UUID in sample");
        }
    }
}
