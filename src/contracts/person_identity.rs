//! Shared person/user identity formatting helpers.
//!
//! These helpers define stable string formatting at the contract boundary so
//! labs-owned modules do not need to depend on one another just to construct
//! canonical entity identifiers.

use crate::contracts::strings::data_model::{ENTITY_PREFIX_PERSON, ENTITY_PREFIX_USER};
use sha2::{Digest, Sha256};

const DEFAULT_PERSON_ID: &str = "local-default";

/// Build a `person:`-prefixed entity ID from a raw person identifier.
#[must_use]
pub fn person_entity_id(person_id: &str) -> String {
    let normalized = sanitize_person_id(person_id);
    let effective = if normalized.is_empty() {
        DEFAULT_PERSON_ID.to_string()
    } else {
        normalized
    };
    format!("{ENTITY_PREFIX_PERSON}{effective}")
}

/// Build a `person:`-prefixed entity ID from channel and sender.
#[must_use]
pub fn channel_entity_id(channel: &str, sender: &str) -> String {
    person_entity_id(&format!("{channel}.{sender}"))
}

/// Construct a user-scoped entity ID from channel + sender.
#[must_use]
pub fn user_entity_id(channel: &str, sender: &str) -> String {
    format!(
        "{ENTITY_PREFIX_USER}{}",
        sanitize_person_id(&format!("{channel}.{sender}"))
    )
}

/// Sanitize a raw identifier to safe ASCII (alphanumeric, `-`, `_`, `.`).
///
/// Legacy-safe identifiers are preserved verbatim. Identifiers that require
/// trimming or character replacement receive a deterministic hash suffix so
/// common lossy forms such as `alice/bob` and `alice_bob` do not collapse to
/// the same person/entity scope.
#[must_use]
pub fn sanitize_person_id(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut out = String::with_capacity(trimmed.len());
    let mut changed = trimmed.len() != raw.len();
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
            changed = true;
        }
    }
    let without_edge_separators = out.trim_matches('_');
    if without_edge_separators.len() != out.len() {
        changed = true;
    }
    if without_edge_separators.is_empty() {
        return String::new();
    }
    if !changed {
        return without_edge_separators.to_string();
    }

    format!("{without_edge_separators}__h{}", stable_person_id_hash(raw))
}

fn stable_person_id_hash(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    hex::encode(&digest[..6])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_person_id_normalizes_unsafe_characters() {
        let sanitized = sanitize_person_id(" user/a b:c ");
        assert!(sanitized.starts_with("user_a_b_c__h"));
        assert_eq!(sanitized.len(), "user_a_b_c__h".len() + 12);
    }

    #[test]
    fn sanitize_person_id_keeps_safe_ascii_tokens() {
        assert_eq!(sanitize_person_id("alice-01.dev"), "alice-01.dev");
    }

    #[test]
    fn user_entity_id_formats_correctly() {
        assert_eq!(user_entity_id("discord", "alice"), "user:discord.alice");
    }

    #[test]
    fn sanitized_person_ids_distinguish_lossy_collisions() {
        assert_ne!(
            sanitize_person_id("alice/bob"),
            sanitize_person_id("alice_bob")
        );
        assert_ne!(person_entity_id("alice/bob"), person_entity_id("alice_bob"));
    }
}
