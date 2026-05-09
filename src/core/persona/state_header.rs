//! Persona state header: the canonical snapshot of the agent's
//! identity principles, safety posture, objectives, open loops,
//! commitments, and recent context. Validated on every transition.

use anyhow::{Result, bail};
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::config::PersonaConfig;

/// Canonical snapshot of persona identity and context state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateHeader {
    /// Hash of the immutable identity principles.
    pub identity_principles_hash: String,
    /// Safety posture label (e.g. "strict").
    pub safety_posture: String,
    /// Currently active objective.
    pub current_objective: String,
    /// Unresolved work items.
    pub open_loops: Vec<String>,
    /// Planned next actions.
    pub next_actions: Vec<String>,
    /// Standing commitments.
    pub commitments: Vec<String>,
    /// Brief summary of the most recent context.
    pub recent_context_summary: String,
    /// RFC 3339 timestamp of the last update.
    pub last_updated_at: String,
}

impl StateHeader {
    /// # Errors
    /// Returns an error if required fields are empty, lengths exceed limits, or timestamp is invalid.
    pub fn validate(&self, persona: &PersonaConfig) -> Result<()> {
        validate_non_empty("identity_principles_hash", &self.identity_principles_hash)?;
        validate_non_empty("safety_posture", &self.safety_posture)?;
        validate_text_len(
            "current_objective",
            &self.current_objective,
            persona.max_current_objective_chars,
        )?;
        validate_text_len(
            "recent_context_summary",
            &self.recent_context_summary,
            persona.max_recent_context_summary_chars,
        )?;

        validate_items(
            "open_loops",
            &self.open_loops,
            persona.max_open_loops,
            persona.max_list_item_chars,
        )?;
        validate_items(
            "next_actions",
            &self.next_actions,
            persona.max_next_actions,
            persona.max_list_item_chars,
        )?;
        validate_items(
            "commitments",
            &self.commitments,
            persona.max_commitments,
            persona.max_list_item_chars,
        )?;

        if DateTime::parse_from_rfc3339(&self.last_updated_at).is_err() {
            bail!("last_updated_at must be RFC3339");
        }

        Ok(())
    }

    /// # Errors
    /// Returns an error if candidate validation fails or immutable fields are mutated.
    pub fn validate_writeback_candidate(
        previous: &Self,
        candidate: &Self,
        persona: &PersonaConfig,
    ) -> Result<()> {
        candidate.validate(persona)?;

        if candidate.identity_principles_hash != previous.identity_principles_hash {
            bail!("immutable field changed: identity_principles_hash");
        }
        if candidate.safety_posture != previous.safety_posture {
            bail!("immutable field changed: safety_posture");
        }

        let previous_last_updated_at = DateTime::parse_from_rfc3339(&previous.last_updated_at)
            .map_err(|_| anyhow::anyhow!("previous.last_updated_at must be RFC3339"))?;
        let candidate_last_updated_at = DateTime::parse_from_rfc3339(&candidate.last_updated_at)
            .map_err(|_| anyhow::anyhow!("candidate.last_updated_at must be RFC3339"))?;
        if candidate_last_updated_at <= previous_last_updated_at {
            bail!("last_updated_at must be strictly later than previous state");
        }

        Ok(())
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(())
}

fn validate_text_len(field: &str, value: &str, max_len: usize) -> Result<()> {
    validate_non_empty(field, value)?;
    if value.len() > max_len && value.chars().count() > max_len {
        bail!("{field} exceeds max length of {max_len}");
    }
    Ok(())
}

fn validate_items(
    field: &str,
    items: &[String],
    max_items: usize,
    max_item_len: usize,
) -> Result<()> {
    if items.len() > max_items {
        bail!("{field} exceeds max items of {max_items}");
    }

    for item in items {
        if item.trim().is_empty() {
            bail!("{field} contains empty item");
        }
        if item.len() > max_item_len && item.chars().count() > max_item_len {
            bail!("{field} item exceeds max length of {max_item_len}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state_header() -> StateHeader {
        StateHeader {
            identity_principles_hash: "abc123".into(),
            safety_posture: "strict".into(),
            current_objective: "Ship contracts for persona loop".into(),
            open_loops: vec!["Add strict schema".into()],
            next_actions: vec!["Implement validation".into()],
            commitments: vec!["Keep main-session scope".into()],
            recent_context_summary: "Task 1 focuses on config/schema contracts only.".into(),
            last_updated_at: "2026-02-16T10:00:00Z".into(),
        }
    }

    #[test]
    fn state_header_valid_v1_payload_parse() {
        let payload = r#"
{
  "identity_principles_hash": "abc123",
  "safety_posture": "strict",
  "current_objective": "Ship contracts for persona loop",
  "open_loops": ["Add strict schema"],
  "next_actions": ["Implement validation"],
  "commitments": ["Keep main-session scope"],
  "recent_context_summary": "Task 1 focuses on config/schema contracts only.",
  "last_updated_at": "2026-02-16T10:00:00Z"
}
"#;

        let parsed: StateHeader = serde_json::from_str(payload).unwrap();
        parsed.validate(&PersonaConfig::default()).unwrap();
    }

    #[test]
    fn state_header_rejects_invalid() {
        let missing_required = r#"
{
  "identity_principles_hash": "abc123",
  "safety_posture": "strict",
  "open_loops": [],
  "next_actions": [],
  "commitments": [],
  "recent_context_summary": "ok",
  "last_updated_at": "2026-02-16T10:00:00Z"
}
"#;

        let err = serde_json::from_str::<StateHeader>(missing_required).unwrap_err();
        assert!(
            err.to_string()
                .contains("missing field `current_objective`"),
            "unexpected serde error: {err}"
        );
    }

    #[test]
    fn state_header_rejects_immutable_field_mutation_boundary() {
        let previous = sample_state_header();
        let mut candidate = previous.clone();
        candidate.identity_principles_hash = "changed".into();

        let err = StateHeader::validate_writeback_candidate(
            &previous,
            &candidate,
            &PersonaConfig::default(),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "immutable field changed: identity_principles_hash"
        );
    }

    #[test]
    fn state_header_rejects_non_advancing_last_updated_at() {
        let previous = sample_state_header();
        let mut candidate = previous.clone();
        candidate.current_objective = "Keep going".into();

        let err = StateHeader::validate_writeback_candidate(
            &previous,
            &candidate,
            &PersonaConfig::default(),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "last_updated_at must be strictly later than previous state"
        );
    }
}
