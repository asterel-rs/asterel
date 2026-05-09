//! Data types for the agent's self-narrative (growth history,
//! milestones, and consistent values). Populated by the persona
//! rebuild cycle.

use serde::{Deserialize, Serialize};

/// A self-narrative summarising the agent's growth, learned lessons, and values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SelfNarrative {
    /// High-level narrative arc: a brief story of how the agent has developed.
    pub narrative_arc: String,
    /// Key experiences that shaped the agent.
    pub key_experiences: Vec<String>,
    /// Areas where the agent has grown or improved.
    pub growth_areas: Vec<String>,
    /// Values that have remained consistent.
    pub consistent_values: Vec<String>,
    /// Open questions the agent is still exploring.
    pub open_questions: Vec<String>,
    /// Significant milestones in the agent's journey.
    #[serde(default)]
    pub milestones: Vec<String>,
    /// Values that are emerging (strengthening over time).
    #[serde(default)]
    pub emerging_values: Vec<String>,
    /// The agent's current chapter in its development.
    #[serde(default)]
    pub current_chapter: String,
    /// ISO 8601 timestamp of last rebuild.
    pub rebuilt_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_narrative_is_empty() {
        let n = SelfNarrative::default();
        assert!(n.narrative_arc.is_empty());
        assert!(n.key_experiences.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let n = SelfNarrative {
            narrative_arc: "I started as a basic assistant and grew.".into(),
            key_experiences: vec!["First companion turn recovery".into()],
            growth_areas: vec!["Tool usage".into()],
            consistent_values: vec!["Accuracy".into()],
            open_questions: vec!["How to handle ambiguity better?".into()],
            rebuilt_at: "2026-03-01T00:00:00Z".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&n).unwrap();
        let back: SelfNarrative = serde_json::from_str(&json).unwrap();
        assert_eq!(back.narrative_arc, n.narrative_arc);
    }
}
