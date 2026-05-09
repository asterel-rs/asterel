//! Cognitive scaffolding snapshot: captures the agent's situational
//! framing for a single turn — affect, domain, reasoning mode, tone
//! register, and style profile values — and serialises it into a
//! `[Cognitive Scaffolding]` prompt block via the presenter.
//!
//! The `RelationshipDepth` threshold ladder (New → Developing →
//! Established → Deep) is driven solely by the cumulative
//! `interaction_count` on the `RelationshipState`.  Values such as
//! `affect_confidence` and `complexity` are clamped to `[0.0, 1.0]`
//! before storage; non-finite floats are replaced with `0.0`.

use serde::{Deserialize, Serialize};

use crate::contracts::affect::AffectLabel;
use crate::contracts::policy::{DomainTag, ReasoningStrategy};
use crate::core::persona::relationship::RelationshipState;
use crate::core::persona::style_profile::StyleProfileState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RelationshipDepth {
    New,
    Developing,
    Established,
    Deep,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScaffoldingState {
    #[serde(rename = "affect")]
    pub affect_label: String,
    #[serde(rename = "confidence")]
    pub affect_confidence: f64,
    #[serde(rename = "relationship")]
    pub relationship_depth: RelationshipDepth,
    pub domain: String,
    pub complexity: f64,
    #[serde(rename = "reasoning")]
    pub reasoning_mode: String,
    #[serde(rename = "tone")]
    pub tone_register: String,
    pub formality: u8,
    pub verbosity: u8,
}

#[must_use]
pub(crate) fn build_scaffolding_state(
    affect_label: AffectLabel,
    affect_confidence: f64,
    relationship: &RelationshipState,
    domain: DomainTag,
    complexity: f64,
    reasoning_mode: ReasoningStrategy,
    tone_register: &str,
    style_profile: &StyleProfileState,
) -> ScaffoldingState {
    ScaffoldingState {
        affect_label: affect_label.as_snake_case().to_string(),
        affect_confidence: clamp_unit_interval(affect_confidence),
        relationship_depth: RelationshipDepth::from_interaction_count(
            relationship.interaction_count,
        ),
        domain: domain.as_snake_case().to_string(),
        complexity: clamp_unit_interval(complexity),
        reasoning_mode: reasoning_mode.as_snake_case().to_string(),
        tone_register: normalize_tone_register(tone_register),
        formality: style_profile.formality,
        verbosity: style_profile.verbosity,
    }
}

impl RelationshipDepth {
    fn from_interaction_count(interaction_count: u32) -> Self {
        match interaction_count {
            0..=4 => Self::New,
            5..=20 => Self::Developing,
            21..=100 => Self::Established,
            _ => Self::Deep,
        }
    }
}

fn clamp_unit_interval(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn normalize_tone_register(tone_register: &str) -> String {
    let trimmed = tone_register.trim();
    if trimmed.is_empty() {
        "balanced".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relationship_with_interactions(interaction_count: u32) -> RelationshipState {
        RelationshipState {
            interaction_count,
            ..RelationshipState::default()
        }
    }

    fn style_profile() -> StyleProfileState {
        StyleProfileState {
            formality: 35,
            verbosity: 45,
            temperature: 0.4,
            updated_at: "2026-03-15T09:00:00Z".to_string(),
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn build_scaffolding_state_maps_signals_into_snapshot() {
        let state = build_scaffolding_state(
            AffectLabel::Anxious,
            0.72,
            &relationship_with_interactions(47),
            DomainTag::Personal,
            0.6,
            ReasoningStrategy::Stepwise,
            "supportive",
            &style_profile(),
        );

        assert_eq!(state.affect_label, "anxious");
        assert_eq!(state.affect_confidence, 0.72);
        assert_eq!(state.relationship_depth, RelationshipDepth::Established);
        assert_eq!(state.domain, "personal");
        assert_eq!(state.complexity, 0.6);
        assert_eq!(state.reasoning_mode, "stepwise");
        assert_eq!(state.tone_register, "supportive");
        assert_eq!(state.formality, 35);
        assert_eq!(state.verbosity, 45);
    }

    #[test]
    fn build_scaffolding_state_maps_relationship_depth_thresholds() {
        assert_eq!(
            RelationshipDepth::from_interaction_count(4),
            RelationshipDepth::New
        );
        assert_eq!(
            RelationshipDepth::from_interaction_count(5),
            RelationshipDepth::Developing
        );
        assert_eq!(
            RelationshipDepth::from_interaction_count(20),
            RelationshipDepth::Developing
        );
        assert_eq!(
            RelationshipDepth::from_interaction_count(21),
            RelationshipDepth::Established
        );
        assert_eq!(
            RelationshipDepth::from_interaction_count(100),
            RelationshipDepth::Established
        );
        assert_eq!(
            RelationshipDepth::from_interaction_count(101),
            RelationshipDepth::Deep
        );
    }

    #[test]
    fn render_scaffolding_block_renders_compact_json() {
        let block =
            crate::core::persona::presenter::render_scaffolding_block(&build_scaffolding_state(
                AffectLabel::Anxious,
                0.72,
                &relationship_with_interactions(47),
                DomainTag::Personal,
                0.6,
                ReasoningStrategy::Stepwise,
                "supportive",
                &style_profile(),
            ));

        assert_eq!(
            block,
            concat!(
                "[Cognitive Scaffolding]\n",
                "{\"affect\":\"anxious\",\"confidence\":0.72,",
                "\"relationship\":\"established\",\"domain\":\"personal\",",
                "\"complexity\":0.6,\"reasoning\":\"stepwise\",",
                "\"tone\":\"supportive\",\"formality\":35,\"verbosity\":45}"
            )
        );
    }

    #[test]
    fn render_scaffolding_block_returns_empty_for_incomplete_state() {
        let state = ScaffoldingState {
            affect_label: String::new(),
            affect_confidence: 0.5,
            relationship_depth: RelationshipDepth::New,
            domain: "general".to_string(),
            complexity: 0.4,
            reasoning_mode: "standard".to_string(),
            tone_register: "balanced".to_string(),
            formality: 50,
            verbosity: 50,
        };

        assert!(crate::core::persona::presenter::render_scaffolding_block(&state).is_empty());
    }
}
