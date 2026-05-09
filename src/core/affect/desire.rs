//! Desire-driven objective modulation: transforms the agent's
//! active goal based on the detected emotional state, so that
//! affect reshapes *what the agent prioritizes*, not just
//! *how it speaks*.
//!
//! References: [DESIRE-DRIVEN] Ma et al., 2025 — emotional
//!   cognitive modeling with desire-driven objective optimization.
//! See the public research reference index in the docs site.
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};

use super::types::AffectLabel;

/// The type of objective the agent should prioritise given the user's affect.
///
/// Each variant represents a distinct motivational stance. The variant selected
/// by [`derive_desire`] reshapes *what* the agent focuses on, not just how it
/// speaks — this is the desire-driven objective modulation mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesireKind {
    /// User needs reassurance or a sense of control (anxious, overwhelmed).
    Safety,
    /// User needs clarity before any solution (confused).
    Understanding,
    /// User needs their feelings acknowledged before problem-solving (sad).
    Validation,
    /// No strong emotional signal — optimise for task completion (neutral).
    Efficiency,
    /// User is curious and wants to go deeper (curious).
    Exploration,
    /// User is positive and wants shared engagement (excited, grateful).
    Connection,
    /// User is frustrated or angry and wants a concrete fix (frustrated, angry).
    Resolution,
}

/// The resolved desire state for a single turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DesireState {
    /// Which objective type was derived from the detected affect.
    pub primary: DesireKind,
    /// Strength of the desire in \[0.0, 1.0\]. Low intensity (< 0.3) means the
    /// desire is too weak to inject into the prompt.
    pub intensity: f64,
    /// One-sentence prefix injected into the agent's objective block.
    /// Empty when the desire is `Efficiency` or intensity is too low.
    ///
    /// Stored as `Cow<'static, str>` so the common case of a fixed template
    /// literal (every variant uses a `&'static str`) stays allocation-free.
    pub objective_prefix: std::borrow::Cow<'static, str>,
}

#[must_use]
pub(crate) fn derive_desire(
    affect: AffectLabel,
    valence: f64,
    arousal: f64,
    confidence: f64,
) -> DesireState {
    if confidence < 0.3 {
        return DesireState {
            primary: DesireKind::Efficiency,
            intensity: 0.3,
            objective_prefix: std::borrow::Cow::Borrowed(""),
        };
    }

    let (kind, prefix) = match affect {
        AffectLabel::Anxious => (
            DesireKind::Safety,
            "Reassure first, then address the request",
        ),
        AffectLabel::Confused => (
            DesireKind::Understanding,
            "Clarify the confusion before solving",
        ),
        AffectLabel::Frustrated => (
            DesireKind::Resolution,
            "Acknowledge frustration, provide direct solution",
        ),
        AffectLabel::Sad => (
            DesireKind::Validation,
            "Acknowledge feelings before problem-solving",
        ),
        AffectLabel::Angry => (
            DesireKind::Resolution,
            "Stay calm, acknowledge the issue, solve concretely",
        ),
        AffectLabel::Overwhelmed => (
            DesireKind::Safety,
            "Simplify and prioritize; one step at a time",
        ),
        AffectLabel::Curious => (DesireKind::Exploration, "Encourage deeper exploration"),
        AffectLabel::Excited => (
            DesireKind::Connection,
            "Match energy and build on enthusiasm",
        ),
        AffectLabel::Grateful => (
            DesireKind::Connection,
            "Acknowledge warmly, continue being helpful",
        ),
        AffectLabel::Neutral => (DesireKind::Efficiency, ""),
    };

    let intensity = compute_desire_intensity(valence, arousal, confidence);

    DesireState {
        primary: kind,
        intensity,
        objective_prefix: std::borrow::Cow::Borrowed(prefix),
    }
}

/// Compute desire intensity from VAD coordinates and detection confidence.
///
/// `emotional_strength = max(|valence|, arousal)` captures how strongly the
/// emotion is felt regardless of direction. Multiplied by `confidence` to
/// down-weight intensities derived from uncertain readings.
fn compute_desire_intensity(valence: f64, arousal: f64, confidence: f64) -> f64 {
    let emotional_strength = valence.abs().max(arousal);
    (emotional_strength * confidence).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use crate::core::affect::presenter::render_desire_block;

    use super::*;

    #[test]
    fn anxious_user_triggers_safety_desire() {
        let desire = derive_desire(AffectLabel::Anxious, -0.3, 0.7, 0.8);
        assert_eq!(desire.primary, DesireKind::Safety);
        assert!(desire.intensity > 0.0);
        assert!(desire.objective_prefix.contains("Reassure"));
    }

    #[test]
    fn neutral_affect_produces_efficiency() {
        let desire = derive_desire(AffectLabel::Neutral, 0.0, 0.2, 0.9);
        assert_eq!(desire.primary, DesireKind::Efficiency);
        assert!(desire.objective_prefix.is_empty());
    }

    #[test]
    fn low_confidence_defaults_to_efficiency() {
        let desire = derive_desire(AffectLabel::Angry, -0.8, 0.9, 0.2);
        assert_eq!(desire.primary, DesireKind::Efficiency);
        assert!(desire.objective_prefix.is_empty());
    }

    #[test]
    fn render_empty_for_neutral() {
        let desire = derive_desire(AffectLabel::Neutral, 0.0, 0.1, 0.5);
        let block = render_desire_block(&desire);
        assert!(block.is_empty());
    }

    #[test]
    fn render_includes_prefix_for_emotional_state() {
        let desire = derive_desire(AffectLabel::Frustrated, -0.5, 0.8, 0.9);
        let block = render_desire_block(&desire);
        assert!(block.contains("[Desire Objective"));
        assert!(block.contains("frustration"));
    }

    #[test]
    fn intensity_is_bounded() {
        for label in [
            AffectLabel::Anxious,
            AffectLabel::Angry,
            AffectLabel::Excited,
            AffectLabel::Sad,
        ] {
            let desire = derive_desire(label, -1.0, 1.0, 1.0);
            assert!(
                (0.0..=1.0).contains(&desire.intensity),
                "{label:?} produced unbounded intensity: {}",
                desire.intensity
            );
        }
    }
}
