//! Event appraisal: converts an affect reading and user context into meaning
//! dimensions that drive topology activation.
//!
//! # Why appraisal?
//!
//! Raw VAD coordinates tell us *how intense* an emotion is, but the topology
//! needs to know *what kind of meaning* the event carries so it can light up the
//! right nodes. Appraisal theory (Lazarus, 1991; Ortony et al., 1988) provides
//! the bridge: every emotion arises from a cognitive evaluation of an event
//! against personal goals and norms.
//!
//! This module answers "what does this event *mean* to this character?" rather
//! than "which canned feeling does it deserve?" The six dimensions are a
//! deliberate simplification of the full appraisal framework — enough to
//! differentiate topology activation without requiring an LLM call.
//!
//! # Pipeline position
//!
//! ```text
//! AffectReading (label + VAD)
//!     │
//!     ▼  appraise_event()
//! EventAppraisal (reward, responsibility, loss_risk, …)
//!     │
//!     ▼  activate_from_appraisal()   [topology.rs]
//! base activation Vec<f32>
//! ```

use serde::{Deserialize, Serialize};

use crate::contracts::affect::{AffectLabel, AffectReading};

/// Multi-dimensional appraisal of an event's meaning to the character.
///
/// All dimensions are in \[0.0, 1.0\] and feed into
/// `activate_from_appraisal()` to set the base intensity for topology nodes.
///
/// The six dimensions were chosen to cover the emotional patterns most relevant
/// to a companion context (reward/punishment, social belonging, relational risk,
/// and normative evaluation) while keeping the mapping computationally cheap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EventAppraisal {
    /// How rewarding or positive this event is for the character.
    /// High reward activates joy, pride, and relief.
    pub reward: f32,
    /// How much the character feels responsible or implicated in the event.
    /// Increases with direct address and high arousal.
    pub responsibility: f32,
    /// Perceived risk of loss, failure, or negative outcome.
    /// High `loss_risk` activates anxiety and guardedness.
    pub loss_risk: f32,
    /// How much the event involves social recognition or belonging.
    /// High `social_validation` activates attachment and (at lower magnitude) envy.
    pub social_validation: f32,
    /// How salient the attachment/relationship dimension is.
    /// Increases when the topic is personal or the user expresses vulnerability.
    pub attachment_salience: f32,
    /// How much the event violates the character's norms or values.
    /// High `norm_violation` activates shame and anger (anger via inverted-U).
    pub norm_violation: f32,
}

/// Appraise an event from the affect reading and conversational context.
///
/// Maps the detected affect label and VAD coordinates to meaning dimensions using
/// a lightweight heuristic — no LLM call required. The heuristics encode
/// commonsense correlations (e.g., `Anxious` + low dominance → high `loss_risk`;
/// `Grateful` + direct address → high `social_validation`) rather than trying to
/// learn them from data.
///
/// `is_direct_address` is true when the user is speaking *to* the companion
/// (rather than narrating about something external). It raises responsibility and
/// `social_validation` because the companion is directly implicated.
///
/// `topic_is_personal` raises `attachment_salience` — vulnerability or personal
/// disclosure shifts the interaction into relational territory.
pub(crate) fn appraise_event(
    reading: &AffectReading,
    is_direct_address: bool,
    topic_is_personal: bool,
) -> EventAppraisal {
    #[allow(clippy::cast_possible_truncation)]
    let valence = reading.valence as f32;
    #[allow(clippy::cast_possible_truncation)]
    let arousal = reading.arousal as f32;
    #[allow(clippy::cast_possible_truncation)]
    let dominance = reading.dominance as f32;

    let reward = match reading.label {
        AffectLabel::Excited | AffectLabel::Grateful => {
            (valence * 0.7 + arousal * 0.3).clamp(0.0, 1.0)
        }
        AffectLabel::Curious => (valence.max(0.0) * 0.5 + 0.3).clamp(0.0, 1.0),
        _ => valence.max(0.0) * 0.5,
    };

    let responsibility = if is_direct_address {
        0.4 + arousal * 0.3
    } else {
        arousal * 0.2
    };

    let loss_risk = match reading.label {
        AffectLabel::Anxious => (arousal * 0.6 + (1.0 - dominance) * 0.4).clamp(0.0, 1.0),
        AffectLabel::Sad => ((1.0 - valence.abs()) * 0.5 + 0.3).clamp(0.0, 1.0),
        AffectLabel::Overwhelmed => (arousal * 0.7).clamp(0.0, 1.0),
        AffectLabel::Frustrated => (arousal * 0.4).clamp(0.0, 1.0),
        _ => (1.0 - dominance).max(0.0) * 0.2,
    };

    let social_validation = if is_direct_address {
        match reading.label {
            AffectLabel::Grateful | AffectLabel::Excited => 0.7,
            AffectLabel::Curious => 0.4,
            _ => 0.2,
        }
    } else {
        0.1
    };

    let attachment_salience = if topic_is_personal {
        match reading.label {
            AffectLabel::Sad | AffectLabel::Anxious => 0.7,
            AffectLabel::Grateful => 0.6,
            AffectLabel::Overwhelmed => 0.5,
            _ => 0.3,
        }
    } else {
        match reading.label {
            AffectLabel::Sad | AffectLabel::Anxious => 0.3,
            _ => 0.1,
        }
    };

    let norm_violation = match reading.label {
        AffectLabel::Angry => (arousal * 0.6 + (1.0 - dominance) * 0.3).clamp(0.0, 1.0),
        AffectLabel::Frustrated => (arousal * 0.4).clamp(0.0, 1.0),
        _ => 0.0,
    };

    EventAppraisal {
        reward,
        responsibility: responsibility.clamp(0.0, 1.0),
        loss_risk,
        social_validation,
        attachment_salience,
        norm_violation,
    }
}

#[must_use]
pub(crate) fn topic_is_personal_text_cue(user_message: &str) -> bool {
    let lower = user_message.to_lowercase();
    [
        "i feel",
        "i'm feeling",
        "i am feeling",
        "i'm scared",
        "i am scared",
        "i'm sad",
        "i am sad",
        "i'm anxious",
        "i am anxious",
        "thank you",
        "thanks",
        "つらい",
        "怖い",
        "悲しい",
        "不安",
        "ありがとう",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::scores::Confidence;

    fn reading(label: AffectLabel, valence: f64, arousal: f64) -> AffectReading {
        AffectReading {
            label,
            valence,
            arousal,
            dominance: 0.5,
            confidence: Confidence::new(0.8),
        }
    }

    #[test]
    fn grateful_direct_address_yields_high_reward_and_social() {
        let r = reading(AffectLabel::Grateful, 0.7, 0.5);
        let a = appraise_event(&r, true, false);
        assert!(a.reward > 0.4, "reward={}", a.reward);
        assert!(a.social_validation > 0.5, "social={}", a.social_validation);
    }

    #[test]
    fn anxious_personal_topic_yields_high_loss_and_attachment() {
        let r = reading(AffectLabel::Anxious, -0.3, 0.7);
        let a = appraise_event(&r, false, true);
        assert!(a.loss_risk > 0.4, "loss_risk={}", a.loss_risk);
        assert!(
            a.attachment_salience > 0.5,
            "attachment={}",
            a.attachment_salience
        );
    }

    #[test]
    fn angry_high_arousal_yields_norm_violation() {
        let r = reading(AffectLabel::Angry, -0.6, 0.8);
        let a = appraise_event(&r, true, false);
        assert!(
            a.norm_violation > 0.3,
            "norm_violation={}",
            a.norm_violation
        );
    }

    #[test]
    fn neutral_indirect_yields_low_everything() {
        let r = reading(AffectLabel::Neutral, 0.0, 0.2);
        let a = appraise_event(&r, false, false);
        assert!(a.reward < 0.1);
        assert!(a.loss_risk < 0.2);
        assert!(a.social_validation < 0.2);
        assert!(a.attachment_salience < 0.2);
    }
}
