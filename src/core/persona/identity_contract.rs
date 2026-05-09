//! Identity contract: enforces layered identity invariants
//! (stable, adaptive, volatile) by validating state transitions
//! against the persona configuration.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::PersonaConfig;
use crate::core::persona::state_header::StateHeader;

/// Version tag for the identity contract schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityContractVersion {
    /// Initial contract schema version.
    V1,
}

/// Immutable identity layer: principles hash and safety posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StableIdentityLayer {
    /// Hash of the agent's core identity principles.
    pub identity_principles_hash: String,
    /// Safety posture label (e.g. "strict").
    pub safety_posture: String,
}

/// Mutable-but-audited layer: objectives, open loops, commitments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptiveIdentityLayer {
    /// Currently active objective.
    pub current_objective: String,
    /// Unresolved work items.
    pub open_loops: Vec<String>,
    /// Standing commitments the agent must honour.
    pub commitments: Vec<String>,
}

/// Ephemeral layer: next actions, context summary, timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolatileIdentityLayer {
    /// Planned next actions for the current turn.
    pub next_actions: Vec<String>,
    /// Brief summary of the most recent context.
    pub recent_context_summary: String,
    /// RFC 3339 timestamp of the last update.
    pub last_updated_at: String,
}

/// V1 identity contract decomposing state into three layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityContractV1 {
    /// Schema version tag.
    pub version: IdentityContractVersion,
    /// Immutable identity layer (must not change between transitions).
    pub stable: StableIdentityLayer,
    /// Auditable layer (objectives, loops, commitments).
    pub adaptive: AdaptiveIdentityLayer,
    /// Ephemeral layer (actions, summary, timestamp).
    pub volatile: VolatileIdentityLayer,
}

impl IdentityContractV1 {
    /// Construct a contract by decomposing a `StateHeader` into layers.
    #[must_use]
    pub fn from_state_header(state: &StateHeader) -> Self {
        Self {
            version: IdentityContractVersion::V1,
            stable: StableIdentityLayer {
                identity_principles_hash: state.identity_principles_hash.clone(),
                safety_posture: state.safety_posture.clone(),
            },
            adaptive: AdaptiveIdentityLayer {
                current_objective: state.current_objective.clone(),
                open_loops: state.open_loops.clone(),
                commitments: state.commitments.clone(),
            },
            volatile: VolatileIdentityLayer {
                next_actions: state.next_actions.clone(),
                recent_context_summary: state.recent_context_summary.clone(),
                last_updated_at: state.last_updated_at.clone(),
            },
        }
    }

    /// Reassemble the three layers back into a `StateHeader`.
    #[must_use]
    pub fn to_state_header(&self) -> StateHeader {
        StateHeader {
            identity_principles_hash: self.stable.identity_principles_hash.clone(),
            safety_posture: self.stable.safety_posture.clone(),
            current_objective: self.adaptive.current_objective.clone(),
            open_loops: self.adaptive.open_loops.clone(),
            next_actions: self.volatile.next_actions.clone(),
            commitments: self.adaptive.commitments.clone(),
            recent_context_summary: self.volatile.recent_context_summary.clone(),
            last_updated_at: self.volatile.last_updated_at.clone(),
        }
    }

    /// # Errors
    /// Returns an error if the projected state header fails persona validation.
    pub fn validate(&self, persona: &PersonaConfig) -> Result<()> {
        self.to_state_header().validate(persona)
    }

    /// # Errors
    /// Returns an error if candidate validation fails or stable layer mutation is detected.
    pub fn validate_mutation(
        previous: &Self,
        candidate: &Self,
        persona: &PersonaConfig,
    ) -> Result<()> {
        candidate.validate(persona)?;
        if candidate.stable != previous.stable {
            bail!("identity contract stable layer is immutable");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state_header() -> StateHeader {
        StateHeader {
            identity_principles_hash: "identity-v1-abcd1234".to_string(),
            safety_posture: "strict".to_string(),
            current_objective: "Deliver identity contract layer".to_string(),
            open_loops: vec!["Finalize WP-101".to_string()],
            next_actions: vec!["Run schema tests".to_string()],
            commitments: vec!["Keep stable layer immutable".to_string()],
            recent_context_summary: "Building stable/adaptive/volatile schema baseline".to_string(),
            last_updated_at: "2026-02-26T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn contract_roundtrip_preserves_state_header() {
        let state = sample_state_header();
        let contract = IdentityContractV1::from_state_header(&state);
        let roundtrip = contract.to_state_header();
        assert_eq!(roundtrip, state);
    }

    #[test]
    fn validate_mutation_rejects_stable_changes() {
        let previous = IdentityContractV1::from_state_header(&sample_state_header());
        let mut candidate = previous.clone();
        candidate.stable.identity_principles_hash = "changed".to_string();

        let err =
            IdentityContractV1::validate_mutation(&previous, &candidate, &PersonaConfig::default())
                .unwrap_err();
        assert_eq!(
            err.to_string(),
            "identity contract stable layer is immutable"
        );
    }

    #[test]
    fn validate_mutation_accepts_adaptive_and_volatile_changes() {
        let previous = IdentityContractV1::from_state_header(&sample_state_header());
        let mut candidate = previous.clone();
        candidate.adaptive.current_objective = "Refine identity schema tests".to_string();
        candidate.volatile.next_actions = vec!["Execute targeted tests".to_string()];
        candidate.volatile.last_updated_at = "2026-02-26T01:00:00Z".to_string();

        IdentityContractV1::validate_mutation(&previous, &candidate, &PersonaConfig::default())
            .unwrap();
    }

    #[test]
    fn serde_rejects_unknown_layer_fields() {
        let payload = r#"
{
  "version": "v1",
  "stable": {
    "identity_principles_hash": "identity-v1-abcd1234",
    "safety_posture": "strict",
    "unknown": true
  },
  "adaptive": {
    "current_objective": "Deliver identity contract layer",
    "open_loops": ["Finalize WP-101"],
    "commitments": ["Keep stable layer immutable"]
  },
  "volatile": {
    "next_actions": ["Run schema tests"],
    "recent_context_summary": "Building stable/adaptive/volatile schema baseline",
    "last_updated_at": "2026-02-26T00:00:00Z"
  }
}
"#;

        let err = serde_json::from_str::<IdentityContractV1>(payload).unwrap_err();
        assert!(
            err.to_string().contains("unknown field `unknown`"),
            "unexpected serde error: {err}"
        );
    }
}
