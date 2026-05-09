//! Person and user entity ID construction and sanitization.
//! Produces deterministic `person:` and `user:` prefixed IDs
//! from channel and sender identifiers.

use crate::config::Config;
pub use crate::contracts::person_identity::{
    channel_entity_id, person_entity_id, sanitize_person_id, user_entity_id,
};

/// Resolve the person ID from env, config, or fallback to default.
#[must_use]
pub fn resolve_person_id(config: &Config) -> String {
    if let Ok(from_env) = std::env::var("ASTEREL_PERSON_ID") {
        let sanitized = sanitize_person_id(&from_env);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }

    if let Some(from_config) = config.identity.person_id.as_deref() {
        let sanitized = sanitize_person_id(from_config);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }

    DEFAULT_PERSON_ID.to_string()
}

const DEFAULT_PERSON_ID: &str = "local-default";

/// Build the canonical persona state-header memory slot key for a raw person id.
#[must_use]
pub(crate) fn canonical_state_header_slot_key(person_id: &str) -> String {
    let sanitized = sanitize_person_id(person_id);
    let effective = if sanitized.is_empty() {
        DEFAULT_PERSON_ID
    } else {
        sanitized.as_str()
    };
    format!("persona/{effective}/state_header/v1")
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
    fn canonical_state_header_slot_key_sanitizes_like_person_entity() {
        let slot_key = canonical_state_header_slot_key(" user/a b:c ");
        assert!(slot_key.starts_with("persona/user_a_b_c__h"));
        assert!(slot_key.ends_with("/state_header/v1"));
    }

    #[test]
    fn person_entity_id_distinguishes_lossy_sanitization_collisions() {
        assert_ne!(person_entity_id("alice/bob"), person_entity_id("alice_bob"));
        assert_ne!(
            canonical_state_header_slot_key("alice/bob"),
            canonical_state_header_slot_key("alice_bob")
        );
    }

    #[test]
    fn canonical_state_header_slot_key_uses_default_for_empty_ids() {
        assert_eq!(
            canonical_state_header_slot_key(" : "),
            "persona/local-default/state_header/v1"
        );
    }
}
