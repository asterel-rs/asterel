//! Style profile: persists and adapts the agent's formality,
//! verbosity, and temperature settings per person, with bounded
//! deltas enforced by the writeback guard.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::schema::CharacterConfig;
use crate::contracts::strings::data_model::{
    SLOT_STYLE_PROFILE_ADAPTATION, SOURCE_PERSONA_STYLE_PROFILE_ADAPTATION,
    SOURCE_PERSONA_STYLE_PROFILE_WRITEBACK,
};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};
use crate::security::writeback_guard::StyleWriteback;

/// Memory slot key for the canonical style profile.
pub const STYLE_PROFILE_CANONICAL_KEY: &str = "persona/style_profile/v1";

const STYLE_PROFILE_ADAPTATION_SLOT_KEY: &str = SLOT_STYLE_PROFILE_ADAPTATION;
const MAX_STYLE_SCORE_DELTA: u8 = 15;
const MAX_STYLE_TEMPERATURE_DELTA: f64 = 0.15;

/// Persisted style profile with formality, verbosity, and temperature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleProfileState {
    /// Formality score (0 = informal, 100 = formal).
    pub formality: u8,
    /// Verbosity score (0 = concise, 100 = verbose).
    pub verbosity: u8,
    /// LLM temperature setting in `[0.0, 1.0]`.
    pub temperature: f64,
    /// RFC 3339 timestamp of the last update.
    pub updated_at: String,
}

/// Result of applying a bounded style profile update.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StyleProfileAdaptationDecision {
    /// Previous style profile state, if any existed.
    pub previous: Option<StyleProfileState>,
    /// Raw requested values before bounding.
    pub requested: StyleProfileRequested,
    /// Applied values after bounded clamping.
    pub applied: StyleProfileState,
    /// Whether any value was clamped to its delta bound.
    pub clamped: bool,
}

/// Raw requested style profile values before bounding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleProfileRequested {
    /// Requested formality score.
    pub formality: u8,
    /// Requested verbosity score.
    pub verbosity: u8,
    /// Requested temperature setting.
    pub temperature: f64,
}

impl StyleProfileState {
    /// Seed a style profile from character defaults, not from Big Five drift.
    #[must_use]
    pub fn from_character_config(config: &CharacterConfig, updated_at: impl Into<String>) -> Self {
        Self {
            formality: config.style_defaults.formality,
            verbosity: config.style_defaults.verbosity,
            temperature: config.style_defaults.temperature,
            updated_at: updated_at.into(),
        }
    }
}

fn style_profile_slot_key(person_id: &str) -> String {
    format!(
        "persona/{}/style_profile/v1",
        sanitize_person_id(person_id).replace(':', "_")
    )
}

fn clamp_score(previous: u8, requested: u8, max_delta: u8) -> (u8, bool) {
    let min_value = previous.saturating_sub(max_delta);
    let max_value = previous.saturating_add(max_delta);
    let bounded = requested.clamp(min_value, max_value);
    (bounded, bounded != requested)
}

fn clamp_temperature(previous: f64, requested: f64, max_delta: f64) -> (f64, bool) {
    let min_value = (previous - max_delta).clamp(0.0, 1.0);
    let max_value = (previous + max_delta).clamp(0.0, 1.0);
    let bounded = requested.clamp(min_value, max_value);
    (bounded, (bounded - requested).abs() > f64::EPSILON)
}

/// Apply a bounded style profile update, clamping deltas to limits.
#[must_use]
pub fn apply_bounded_style_profile(
    previous: Option<&StyleProfileState>,
    requested: &StyleWriteback,
    updated_at: &str,
) -> StyleProfileAdaptationDecision {
    let (applied_formality, formality_clamped) = previous
        .map_or((requested.formality, false), |p| {
            clamp_score(p.formality, requested.formality, MAX_STYLE_SCORE_DELTA)
        });
    let (applied_verbosity, verbosity_clamped) = previous
        .map_or((requested.verbosity, false), |p| {
            clamp_score(p.verbosity, requested.verbosity, MAX_STYLE_SCORE_DELTA)
        });
    let (applied_temperature, temperature_clamped) =
        previous.map_or((requested.temperature, false), |p| {
            clamp_temperature(
                p.temperature,
                requested.temperature,
                MAX_STYLE_TEMPERATURE_DELTA,
            )
        });

    StyleProfileAdaptationDecision {
        previous: previous.cloned(),
        requested: StyleProfileRequested {
            formality: requested.formality,
            verbosity: requested.verbosity,
            temperature: requested.temperature,
        },
        applied: StyleProfileState {
            formality: applied_formality,
            verbosity: applied_verbosity,
            temperature: applied_temperature,
            updated_at: updated_at.to_string(),
        },
        clamped: formality_clamped || verbosity_clamped || temperature_clamped,
    }
}

/// # Errors
/// Returns an error if memory lookup, slot parsing, or deserialization fails.
pub async fn load_style_profile(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<StyleProfileState>> {
    let entity_id = person_entity_id(person_id);
    let slot_key = style_profile_slot_key(person_id);
    let Some(slot) = mem.resolve_slot(&entity_id, &slot_key).await? else {
        return Ok(None);
    };

    let parsed: StyleProfileState = serde_json::from_str(&slot.value).with_context(|| {
        format!("parse canonical style profile from slot key: {STYLE_PROFILE_CANONICAL_KEY}")
    })?;
    Ok(Some(parsed))
}

async fn persist_style_profile(
    mem: &dyn Memory,
    person_id: &str,
    profile: &StyleProfileState,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        style_profile_slot_key(person_id),
        MemoryEventType::FactUpdated,
        serde_json::to_string(profile)?,
        0.95,
        0.8,
        format!("persona-style-profile-writeback:{}", profile.updated_at),
        SOURCE_PERSONA_STYLE_PROFILE_WRITEBACK,
        Some(profile.updated_at.clone()),
        person_id,
    )
    .await
}

async fn persist_style_profile_adaptation_event(
    mem: &dyn Memory,
    person_id: &str,
    decision: &StyleProfileAdaptationDecision,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        STYLE_PROFILE_ADAPTATION_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        serde_json::to_string(decision)?,
        0.9,
        0.6,
        format!(
            "persona-style-profile-adaptation:{}",
            decision.applied.updated_at
        ),
        SOURCE_PERSONA_STYLE_PROFILE_ADAPTATION,
        Some(decision.applied.updated_at.clone()),
        person_id,
    )
    .await
}

/// # Errors
/// Returns an error if loading previous state or persisting update records fails.
pub async fn apply_style_profile_update(
    mem: &dyn Memory,
    person_id: &str,
    requested: &StyleWriteback,
    reflected_at: &str,
) -> Result<StyleProfileAdaptationDecision> {
    let previous = load_style_profile(mem, person_id).await?;
    let decision = apply_bounded_style_profile(previous.as_ref(), requested, reflected_at);
    persist_style_profile_adaptation_event(mem, person_id, &decision).await?;
    persist_style_profile(mem, person_id, &decision.applied).await?;
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{MarkdownMemory, Memory};

    #[test]
    fn apply_bounded_style_profile_without_previous_uses_requested_values() {
        let requested = StyleWriteback {
            formality: 70,
            verbosity: 30,
            temperature: 0.4,
        };

        let decision = apply_bounded_style_profile(None, &requested, "2026-02-26T09:00:00Z");
        assert!(!decision.clamped);
        assert_eq!(decision.applied.formality, 70);
        assert_eq!(decision.applied.verbosity, 30);
        assert!((decision.applied.temperature - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_bounded_style_profile_clamps_large_delta() {
        let previous = StyleProfileState {
            formality: 10,
            verbosity: 90,
            temperature: 0.2,
            updated_at: "2026-02-26T09:00:00Z".to_string(),
        };
        let requested = StyleWriteback {
            formality: 90,
            verbosity: 10,
            temperature: 0.8,
        };

        let decision =
            apply_bounded_style_profile(Some(&previous), &requested, "2026-02-26T10:00:00Z");
        assert!(decision.clamped);
        assert_eq!(decision.applied.formality, 25);
        assert_eq!(decision.applied.verbosity, 75);
        assert!((decision.applied.temperature - 0.35).abs() < f64::EPSILON);
    }

    #[test]
    fn render_style_guidance_renders_classification_labels() {
        let profile = StyleProfileState {
            formality: 80,
            verbosity: 20,
            temperature: 0.35,
            updated_at: "2026-02-26T10:00:00Z".to_string(),
        };
        let guidance = crate::core::persona::presenter::render_style_guidance(&profile);
        assert!(guidance.contains("formality=80 (high)"));
        assert!(guidance.contains("verbosity=20 (concise)"));
        assert!(guidance.contains("temperature=0.35"));
    }

    #[test]
    fn style_profile_can_seed_from_character_config_without_touching_traits() {
        let config = crate::config::schema::CharacterConfig::default();
        let profile = StyleProfileState::from_character_config(&config, "config-seed");
        assert_eq!(profile.formality, config.style_defaults.formality);
        assert_eq!(profile.verbosity, config.style_defaults.verbosity);
        assert!((profile.temperature - config.style_defaults.temperature).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn apply_style_profile_update_persists_canonical_and_adaptation_records() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let first_requested = StyleWriteback {
            formality: 20,
            verbosity: 20,
            temperature: 0.2,
        };
        let first = apply_style_profile_update(
            mem.as_ref(),
            "person-test",
            &first_requested,
            "2026-02-26T09:00:00Z",
        )
        .await
        .expect("first style profile update should pass");
        assert!(!first.clamped);

        let second_requested = StyleWriteback {
            formality: 80,
            verbosity: 90,
            temperature: 0.8,
        };
        let second = apply_style_profile_update(
            mem.as_ref(),
            "person-test",
            &second_requested,
            "2026-02-26T10:00:00Z",
        )
        .await
        .expect("second style profile update should pass");
        assert!(second.clamped);
        assert_eq!(second.applied.formality, 35);
        assert_eq!(second.applied.verbosity, 35);
        assert!((second.applied.temperature - 0.35).abs() < f64::EPSILON);

        let loaded = load_style_profile(mem.as_ref(), "person-test")
            .await
            .expect("style profile should load")
            .expect("style profile should exist");
        assert_eq!(loaded, second.applied);

        let adaptation = mem
            .resolve_slot(
                &person_entity_id("person-test"),
                STYLE_PROFILE_ADAPTATION_SLOT_KEY,
            )
            .await
            .expect("adaptation slot query should pass")
            .expect("adaptation slot should exist");
        assert!(adaptation.value.contains("\"clamped\":true"));
    }
}
