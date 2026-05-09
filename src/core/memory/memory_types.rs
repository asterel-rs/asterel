//! Canonical memory domain types shared across backends.
//!
//! Policy-relevant classification types (`MemorySource`, `MemoryEventType`,
//! `PrivacyLevel`, `SourceKind`, `MemoryProvenance`, `MemoryLayer`,
//! `SignalTier`, `MemoryEventInput`) are defined in
//! `crate::contracts::memory` (L0) and re-exported here so that all existing
//! consumers continue to import from `crate::core::memory`.

use super::emotional_context::EmotionalContext;

mod ingress;

pub use crate::contracts::memory_domain::{
    BeliefSlot, CapabilitySupport, GraphEdge, GraphEntity, GraphEntityType, GraphRelationType,
    MemoryCapMatrix, MemoryCategory, MemoryEntry, MemoryEvent, MemoryInferenceEvent,
    MemoryIntegrityIssue, MemoryIntegrityReport, MemoryRecallEntry, NodeTier, RecallQuery,
};
pub use crate::contracts::memory_forget::{
    ForgetArtifact, ForgetArtifactCheck, ForgetMode, ForgetObservation, ForgetOutcome,
    ForgetRequirement, ForgetStatus,
};

pub use crate::contracts::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource, PrivacyLevel,
    SignalTier, SourceKind,
};

impl MemoryEventInput {
    /// # Errors
    /// Returns an error if ingress normalization fails validation.
    pub fn normalize_for_ingress(mut self) -> crate::contracts::memory_error::MemoryResult<Self> {
        ingress::normalize_memory_event_input(&mut self)?;
        Ok(self)
    }

    #[must_use]
    pub fn with_emotion(mut self, ctx: EmotionalContext) -> Self {
        self.emotion_label = Some(ctx.label);
        self.emotion_valence = Some(clamp_finite(ctx.valence, -1.0, 1.0));
        self.emotion_arousal = Some(clamp_finite(ctx.arousal, 0.0, 1.0));
        self.emotion_confidence = Some(clamp_finite(ctx.confidence, 0.0, 1.0));
        self
    }
}

fn clamp_finite(value: f64, min: f64, max: f64) -> f64 {
    if value.is_nan() {
        min.max(0.0).min(max)
    } else {
        value.clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_for_ingress_sanitizes_entity_and_slot() {
        let input = MemoryEventInput::new(
            " person:User / A ",
            "external.channel.discord.user/1?x",
            MemoryEventType::FactAdded,
            "v",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        );

        let normalized = input.normalize_for_ingress().unwrap();
        assert_eq!(normalized.entity_id.as_str(), "person:User_A");
        assert_eq!(
            normalized.slot_key.as_str(),
            "external.channel.discord.user/1_x"
        );
    }

    #[test]
    fn normalize_for_ingress_rejects_empty_identifiers() {
        let input = MemoryEventInput::new(
            "   ",
            "slot",
            MemoryEventType::FactAdded,
            "v",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        );
        let err = input.normalize_for_ingress().unwrap_err().to_string();
        assert!(err.contains("entity_id"));

        let input = MemoryEventInput::new(
            "entity",
            "   ",
            MemoryEventType::FactAdded,
            "v",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        );
        let err = input.normalize_for_ingress().unwrap_err().to_string();
        assert!(err.contains("slot_key"));
    }

    #[test]
    fn normalize_for_ingress_rejects_invalid_slot_key_pattern() {
        let input = MemoryEventInput::new(
            "entity",
            ".invalid-slot",
            MemoryEventType::FactAdded,
            "v",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        );
        let err = input.normalize_for_ingress().unwrap_err().to_string();
        assert!(err.contains("slot_key must match taxonomy pattern"));
    }

    #[test]
    fn with_emotion_clamps_non_finite_values() {
        let input = MemoryEventInput::new(
            "entity",
            "emotion.slot",
            MemoryEventType::FactAdded,
            "v",
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_emotion(EmotionalContext {
            label: "joy".to_string(),
            valence: f64::NAN,
            arousal: f64::INFINITY,
            confidence: f64::NAN,
        });

        assert_eq!(input.emotion_valence, Some(0.0));
        assert_eq!(input.emotion_arousal, Some(1.0));
        assert_eq!(input.emotion_confidence, Some(0.0));
    }
}
