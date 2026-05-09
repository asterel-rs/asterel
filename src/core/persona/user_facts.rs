// Wired (P-1): extract/persist connected to session_posturn.run_post_answer_pipeline.

//! Cross-session user fact persistence.
//!
//! Maintains a lightweight `UserProfile` composed of stable facts
//! (name, language, response style preference, ongoing projects)
//! that are persisted across sessions via memory slots and injected
//! into the gateway persona context as a `[User Profile]` block.
//!
//! ## Wiring status — persona (extract/persist path)
//!
//! **Load/render:** already wired into the turn pipeline.
//! **Extract/persist (P-1, 2026-04-06):** wired into
//! `session_posturn::run_post_answer_pipeline` — called after each non-ephemeral
//! user message.  `USER_FACT_SUFFIXES` and `load_user_profile` remain unused
//! (`#[allow(dead_code)]`) until further integration.

use anyhow::Result;

use crate::contracts::strings::data_model::{
    SLOT_USER_FACT_LANGUAGE_SUFFIX, SLOT_USER_FACT_NAME_SUFFIX, SLOT_USER_FACT_PROJECTS_SUFFIX,
    SLOT_USER_FACT_STYLE_PREF_SUFFIX, SOURCE_REF_USER_FACT_UPDATE, SOURCE_REF_USER_FACT_WRITEBACK,
};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};

fn user_fact_slot_key(person_id: &str, suffix: &str) -> String {
    format!(
        "persona/{}/user_facts/{}",
        sanitize_person_id(person_id),
        suffix
    )
}

// TODO(persona): iterated by load_user_profile_for_entity to bulk-load all fact slots.
#[allow(dead_code)]
const USER_FACT_SUFFIXES: [&str; 4] = [
    SLOT_USER_FACT_NAME_SUFFIX,
    SLOT_USER_FACT_LANGUAGE_SUFFIX,
    SLOT_USER_FACT_STYLE_PREF_SUFFIX,
    SLOT_USER_FACT_PROJECTS_SUFFIX,
];

/// Known user facts loaded from persistent memory slots.
#[derive(Debug, Clone, Default)]
pub(crate) struct UserProfile {
    /// Display name or preferred name.
    pub name: Option<String>,
    /// Preferred language (e.g. "ja", "en").
    pub language: Option<String>,
    /// Response style preference (e.g. "concise", "detailed", "casual").
    pub style_pref: Option<String>,
    /// Ongoing projects or topics the user is working on.
    pub ongoing_projects: Option<String>,
}

impl UserProfile {
    /// Returns `true` if all fields are `None`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.language.is_none()
            && self.style_pref.is_none()
            && self.ongoing_projects.is_none()
    }
}

/// Load known user facts from memory slots.
///
/// Each known slot is resolved independently; missing slots are simply
/// left as `None`.
///
/// # Errors
///
/// Returns an error only if the memory backend itself fails.
// TODO(persona): called from turn pipeline (non-tenant path) for persona context injection — see module wiring status.
#[allow(dead_code)]
pub(crate) async fn load_user_profile(mem: &dyn Memory, person_id: &str) -> Result<UserProfile> {
    load_user_profile_for_entity(mem, &person_entity_id(person_id), person_id).await
}

/// Load known user facts from memory slots for a specific entity.
///
/// This is intended for tenant-scoped callers that already resolved their
/// scoped entity identifier.
pub(crate) async fn load_user_profile_for_entity(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
) -> Result<UserProfile> {
    let mut profile = UserProfile::default();

    let fields: [(&str, &mut Option<String>); 4] = [
        (SLOT_USER_FACT_NAME_SUFFIX, &mut profile.name),
        (SLOT_USER_FACT_LANGUAGE_SUFFIX, &mut profile.language),
        (SLOT_USER_FACT_STYLE_PREF_SUFFIX, &mut profile.style_pref),
        (
            SLOT_USER_FACT_PROJECTS_SUFFIX,
            &mut profile.ongoing_projects,
        ),
    ];

    for (suffix, field) in fields {
        let slot_key = user_fact_slot_key(person_id, suffix);
        match mem.resolve_slot(entity_id, &slot_key).await {
            Ok(Some(slot)) => {
                let value = slot.value.trim().to_string();
                if !value.is_empty() {
                    *field = Some(value);
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(
                    person_id,
                    %slot_key,
                    error = %e,
                    "failed to resolve user fact slot"
                );
            }
        }
    }

    Ok(profile)
}

/// Extracted user fact: `(slot_key, value)`.
type ExtractedFact = (String, String);

/// Rule-based extraction of stable user facts from a message.
///
/// Patterns detected:
/// - "My name is X" / "I'm X" (name)
/// - "I speak X" / "I prefer X language" (language)
/// - "I prefer X responses" / "I like X style" (style)
/// - "I'm working on X" / "my project is X" (projects)
///
/// Returns a vec of `(slot_key, value)` pairs; callers decide persistence.
// Wired (P-1): called from run_post_answer_pipeline after each user message.
#[must_use]
pub(crate) fn extract_user_facts(message: &str) -> Vec<ExtractedFact> {
    let mut facts = Vec::new();
    let lower = message.to_lowercase();

    if let Some(name) = extract_name(&lower, message) {
        facts.push((SLOT_USER_FACT_NAME_SUFFIX.to_string(), name));
    }
    if let Some(lang) = extract_language(&lower) {
        facts.push((SLOT_USER_FACT_LANGUAGE_SUFFIX.to_string(), lang));
    }
    if let Some(style) = extract_style_pref(&lower) {
        facts.push((SLOT_USER_FACT_STYLE_PREF_SUFFIX.to_string(), style));
    }
    if let Some(project) = extract_project(&lower, message) {
        facts.push((SLOT_USER_FACT_PROJECTS_SUFFIX.to_string(), project));
    }

    facts
}

/// Persist a single user fact to a memory slot.
///
/// Uses the `persist_helper` to enforce persona long-term write policy.
///
/// # Errors
///
/// Returns an error if memory write or policy enforcement fails.
// Wired (P-1): called from run_post_answer_pipeline for each extracted fact.
pub(crate) async fn persist_user_fact(
    mem: &dyn Memory,
    person_id: &str,
    fact_suffix: &str,
    value: &str,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        user_fact_slot_key(person_id, fact_suffix),
        MemoryEventType::FactUpdated,
        value.to_string(),
        0.85,
        0.8,
        SOURCE_REF_USER_FACT_UPDATE,
        SOURCE_REF_USER_FACT_WRITEBACK,
        None,
        person_id,
    )
    .await
}

fn extract_name(lower: &str, original: &str) -> Option<String> {
    let patterns: &[&str] = &["my name is ", "call me ", "i'm ", "i am "];
    for pattern in patterns {
        if let Some(rest) = lower.find(pattern).map(|i| &original[i + pattern.len()..]) {
            let name = rest.split(['.', ',', '!', '?', '\n']).next()?.trim();
            if is_plausible_name_fragment(name) {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn is_plausible_name_fragment(name: &str) -> bool {
    if name.is_empty() || name.len() > 50 || name.split_whitespace().count() > 4 {
        return false;
    }
    let normalized = name
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_ascii_lowercase();
    let first_word = normalized.split_whitespace().next().unwrap_or_default();
    !matches!(
        first_word,
        "sorry"
            | "done"
            | "here"
            | "sure"
            | "right"
            | "just"
            | "working"
            | "getting"
            | "not"
            | "new"
            | "fine"
            | "ok"
            | "okay"
    )
}

fn extract_language(lower: &str) -> Option<String> {
    let lang_map: &[(&str, &str)] = &[
        ("japanese", "ja"),
        ("english", "en"),
        ("chinese", "zh"),
        ("korean", "ko"),
        ("spanish", "es"),
        ("french", "fr"),
        ("german", "de"),
        ("portuguese", "pt"),
        ("russian", "ru"),
        ("italian", "it"),
        ("arabic", "ar"),
    ];

    if lower.contains("i speak ") || lower.contains("i prefer ") {
        for (name, code) in lang_map {
            if lower.contains(name) {
                return Some((*code).to_string());
            }
        }
    }

    if lower.contains("in japanese") || lower.contains("reply in japanese") {
        return Some("ja".to_string());
    }
    if lower.contains("in english") || lower.contains("reply in english") {
        return Some("en".to_string());
    }

    None
}

fn extract_style_pref(lower: &str) -> Option<String> {
    let style_keywords: &[(&str, &str)] = &[
        ("concise", "concise"),
        ("brief", "concise"),
        ("short", "concise"),
        ("detailed", "detailed"),
        ("verbose", "detailed"),
        ("thorough", "detailed"),
        ("casual", "casual"),
        ("informal", "casual"),
        ("formal", "formal"),
        ("technical", "technical"),
    ];

    let triggers = [
        "i prefer ",
        "i like ",
        "i want ",
        "give me ",
        "please be ",
        "be more ",
    ];

    for trigger in &triggers {
        if lower.contains(trigger) {
            for (keyword, style) in style_keywords {
                if lower.contains(keyword) {
                    return Some((*style).to_string());
                }
            }
        }
    }

    None
}

fn extract_project(lower: &str, original: &str) -> Option<String> {
    let patterns: &[&str] = &[
        "i'm working on ",
        "i am working on ",
        "my project is ",
        "working on a project called ",
    ];
    for pattern in patterns {
        if let Some(rest) = lower.find(pattern).map(|i| &original[i + pattern.len()..]) {
            let project = rest.split(['.', '!', '?', '\n']).next()?.trim();
            if !project.is_empty() && project.len() <= 100 {
                return Some(project.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{MarkdownMemory, Memory};

    #[test]
    fn empty_profile_renders_nothing() {
        let profile = UserProfile::default();
        assert!(crate::core::persona::presenter::render_user_profile_block(&profile).is_empty());
        assert!(profile.is_empty());
    }

    #[test]
    fn partial_profile_renders_known_fields() {
        let profile = UserProfile {
            name: Some("Haru".into()),
            language: Some("ja".into()),
            style_pref: None,
            ongoing_projects: None,
        };
        let block = crate::core::persona::presenter::render_user_profile_block(&profile);
        assert!(block.contains("[User Profile]"));
        assert!(block.contains("- name: Haru"));
        assert!(block.contains("- language: ja"));
        assert!(!block.contains("preferred_style"));
        assert!(!block.contains("ongoing_projects"));
    }

    #[test]
    fn full_profile_renders_all_fields() {
        let profile = UserProfile {
            name: Some("Haru".into()),
            language: Some("ja".into()),
            style_pref: Some("concise".into()),
            ongoing_projects: Some("Asterel".into()),
        };
        let block = crate::core::persona::presenter::render_user_profile_block(&profile);
        assert!(block.contains("- name: Haru"));
        assert!(block.contains("- language: ja"));
        assert!(block.contains("- preferred_style: concise"));
        assert!(block.contains("- ongoing_projects: Asterel"));
    }

    #[test]
    fn extract_name_from_message() {
        let facts = extract_user_facts("My name is Haru");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].0, SLOT_USER_FACT_NAME_SUFFIX);
        assert_eq!(facts[0].1, "Haru");
    }

    #[test]
    fn extract_name_call_me() {
        let facts = extract_user_facts("Call me Alex");
        assert!(
            facts
                .iter()
                .any(|(k, v)| k == SLOT_USER_FACT_NAME_SUFFIX && v == "Alex")
        );
    }

    #[test]
    fn extract_name_rejects_common_im_not_name_fragments() {
        for message in ["I'm sorry", "I'm done", "I'm here", "I'm okay"] {
            let facts = extract_user_facts(message);
            assert!(
                !facts.iter().any(|(k, _)| k == SLOT_USER_FACT_NAME_SUFFIX),
                "unexpected name fact from {message:?}: {facts:?}"
            );
        }
    }

    #[test]
    fn extract_language_i_speak() {
        let facts = extract_user_facts("I speak Japanese");
        assert!(
            facts
                .iter()
                .any(|(k, v)| k == SLOT_USER_FACT_LANGUAGE_SUFFIX && v == "ja")
        );
    }

    #[test]
    fn extract_language_in_english() {
        let facts = extract_user_facts("Please reply in English");
        assert!(
            facts
                .iter()
                .any(|(k, v)| k == SLOT_USER_FACT_LANGUAGE_SUFFIX && v == "en")
        );
    }

    #[test]
    fn extract_style_pref_concise() {
        let facts = extract_user_facts("I prefer concise answers");
        assert!(
            facts
                .iter()
                .any(|(k, v)| k == SLOT_USER_FACT_STYLE_PREF_SUFFIX && v == "concise")
        );
    }

    #[test]
    fn extract_style_pref_detailed() {
        let facts = extract_user_facts("I like detailed explanations");
        assert!(
            facts
                .iter()
                .any(|(k, v)| k == SLOT_USER_FACT_STYLE_PREF_SUFFIX && v == "detailed")
        );
    }

    #[test]
    fn extract_project_working_on() {
        let facts = extract_user_facts("I'm working on a web scraper");
        assert!(
            facts
                .iter()
                .any(|(k, v)| k == SLOT_USER_FACT_PROJECTS_SUFFIX && v == "a web scraper")
        );
    }

    #[test]
    fn no_extraction_from_normal_message() {
        let facts = extract_user_facts("What's the weather like today?");
        assert!(facts.is_empty());
    }

    #[test]
    fn extract_multiple_facts() {
        let facts =
            extract_user_facts("My name is Haru. I speak Japanese. I prefer concise answers.");
        assert!(facts.iter().any(|(k, _)| k == SLOT_USER_FACT_NAME_SUFFIX));
        assert!(
            facts
                .iter()
                .any(|(k, _)| k == SLOT_USER_FACT_LANGUAGE_SUFFIX)
        );
        assert!(
            facts
                .iter()
                .any(|(k, _)| k == SLOT_USER_FACT_STYLE_PREF_SUFFIX)
        );
    }

    #[test]
    fn rejects_overly_long_name() {
        let long_name = "A".repeat(60);
        let msg = format!("My name is {long_name}");
        let facts = extract_user_facts(&msg);
        assert!(!facts.iter().any(|(k, _)| k == SLOT_USER_FACT_NAME_SUFFIX));
    }

    #[tokio::test]
    async fn persist_and_load_user_profile_round_trip() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        persist_user_fact(
            mem.as_ref(),
            "test-user",
            SLOT_USER_FACT_NAME_SUFFIX,
            "Haru",
        )
        .await
        .expect("persist name");
        persist_user_fact(
            mem.as_ref(),
            "test-user",
            SLOT_USER_FACT_LANGUAGE_SUFFIX,
            "ja",
        )
        .await
        .expect("persist language");

        let profile = load_user_profile(mem.as_ref(), "test-user")
            .await
            .expect("load profile");

        assert_eq!(profile.name.as_deref(), Some("Haru"));
        assert_eq!(profile.language.as_deref(), Some("ja"));
        assert!(profile.style_pref.is_none());
        assert!(profile.ongoing_projects.is_none());
    }

    #[tokio::test]
    async fn load_profile_for_entity_respects_entity_scope() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        persist_user_fact(
            mem.as_ref(),
            "test-user",
            SLOT_USER_FACT_NAME_SUFFIX,
            "Haru",
        )
        .await
        .expect("persist name");

        let scoped =
            load_user_profile_for_entity(mem.as_ref(), &person_entity_id("test-user"), "test-user")
                .await
                .expect("load scoped profile");
        assert_eq!(scoped.name.as_deref(), Some("Haru"));

        let wrong_scope = load_user_profile_for_entity(
            mem.as_ref(),
            &person_entity_id("other-entity"),
            "test-user",
        )
        .await
        .expect("load profile with wrong scope");
        assert!(wrong_scope.is_empty());
    }

    #[tokio::test]
    async fn empty_profile_from_empty_memory() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let profile = load_user_profile(mem.as_ref(), "nonexistent")
            .await
            .expect("load empty profile");

        assert!(profile.is_empty());
    }
}
