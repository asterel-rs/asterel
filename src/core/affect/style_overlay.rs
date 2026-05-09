//! Affect-to-style modulation: converts affect labels and VAD coordinates into
//! per-turn style deltas that adjust formality, verbosity, and sampling
//! temperature within constitutional guardrails.
//!
//! # Why style modulation?
//!
//! Affect recognition is only useful if it changes behaviour. Style modulation
//! is the *lightweight* behaviour change: it does not alter *what* the agent
//! says, only *how* it says it. An anxious user gets slightly more verbosity
//! (fuller explanation) and lower temperature (more predictable, less creative).
//! An angry user gets higher formality (professional distance) and much lower
//! temperature (precise, not creative). Excited users get looser temperature and
//! slightly reduced formality (match their energy).
//!
//! # Two entry points
//!
//! - [`affect_to_style_delta`] — discrete label → fixed delta table. Fast,
//!   interpretable, used when only the label is available or as a fallback.
//!
//! - [`affect_vad_to_style_delta`] — continuous VAD → linear interpolation.
//!   Finer-grained; preferred when a full `AffectReading` is available.
//!   The VAD formulas are:
//!   - `formality_delta = dominance × 15` (high control → more formal)
//!   - `verbosity_delta = valence × −10 + arousal × 5` (negative valence →
//!     more explanation; high arousal → keep it brief)
//!   - `temperature_delta = arousal × 0.1 − (1 − |valence|) × 0.05`
//!     (arousal increases creativity; calm certainty decreases it)
//!
//! # Constitutional guardrails
//!
//! All deltas are hard-clamped: formality/verbosity ±15, temperature ±0.15.
//! These bounds ensure affect modulation cannot push style so far that it
//! compromises response quality or safety.

use super::types::{AffectLabel, AffectReading};

/// Per-turn style deltas for affect modulation.
///
/// These mirror the `TurnStyleOverlay` from the augment module.
/// The augmentation pipeline converts these into `TurnStyleOverlay` values.
///
/// Constitutional guardrails: formality/verbosity ±15, temperature ±0.15
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_field_names)] // _delta suffix distinguishes from absolute style values
pub(crate) struct AffectStyleDelta {
    /// Formality adjustment (clamped to +/-15).
    pub formality_delta: i8,
    /// Verbosity adjustment (clamped to +/-15).
    pub verbosity_delta: i8,
    /// Sampling temperature adjustment (clamped to +/-0.15).
    pub temperature_delta: f64,
}

/// Convert an affect label to style overlay deltas (discrete fallback).
pub(crate) fn affect_to_style_delta(label: AffectLabel) -> AffectStyleDelta {
    match label {
        AffectLabel::Neutral => AffectStyleDelta::default(),
        AffectLabel::Confused => AffectStyleDelta {
            formality_delta: 0,
            verbosity_delta: 10,
            temperature_delta: -0.05,
        },
        AffectLabel::Frustrated => AffectStyleDelta {
            formality_delta: 5,
            verbosity_delta: -10,
            temperature_delta: -0.05,
        },
        AffectLabel::Anxious => AffectStyleDelta {
            formality_delta: 0,
            verbosity_delta: 5,
            temperature_delta: -0.05,
        },
        AffectLabel::Sad | AffectLabel::Grateful => AffectStyleDelta {
            formality_delta: -5,
            verbosity_delta: 0,
            temperature_delta: 0.0,
        },
        AffectLabel::Angry => AffectStyleDelta {
            formality_delta: 10,
            verbosity_delta: -10,
            temperature_delta: -0.10,
        },
        AffectLabel::Excited => AffectStyleDelta {
            formality_delta: -5,
            verbosity_delta: 5,
            temperature_delta: 0.05,
        },
        AffectLabel::Curious => AffectStyleDelta {
            formality_delta: 0,
            verbosity_delta: 10,
            temperature_delta: 0.05,
        },
        AffectLabel::Overwhelmed => AffectStyleDelta {
            formality_delta: 0,
            verbosity_delta: -5,
            temperature_delta: -0.05,
        },
    }
}

/// Compute style deltas from continuous VAD coordinates.
///
/// Uses linear interpolation in VAD space for finer-grained style
/// modulation than discrete label matching alone.
///
/// - `formality_delta`: Dominance drives formality (high D → more formal).
/// - `verbosity_delta`: Negative valence increases verbosity (more
///   explanation), high arousal reduces it (keep it brief).
/// - `temperature_delta`: Arousal increases sampling temperature, calm
///   certainty (high valence, low arousal) decreases it.
// Cast safety: VAD-derived deltas are bounded to [-15, 15] before i8 conversion.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn affect_vad_to_style_delta(reading: &AffectReading) -> AffectStyleDelta {
    let v = reading.valence;
    let a = reading.arousal;
    let d = reading.dominance;

    let formality_raw = d * 15.0;
    let verbosity_raw = v * -10.0 + a * 5.0;
    let temperature_raw = a * 0.1 - (1.0 - v.abs()) * 0.05;

    AffectStyleDelta {
        formality_delta: (formality_raw as i8).clamp(-15, 15),
        verbosity_delta: (verbosity_raw as i8).clamp(-15, 15),
        temperature_delta: temperature_raw.clamp(-0.15, 0.15),
    }
}

#[cfg(test)]
mod tests {
    use super::{affect_to_style_delta, affect_vad_to_style_delta};
    use crate::core::affect::presenter::render_affect_block;
    use crate::core::affect::types::{AffectLabel, AffectReading};

    #[test]
    fn all_deltas_within_constitutional_bounds() {
        let labels = [
            AffectLabel::Neutral,
            AffectLabel::Confused,
            AffectLabel::Frustrated,
            AffectLabel::Anxious,
            AffectLabel::Sad,
            AffectLabel::Angry,
            AffectLabel::Excited,
            AffectLabel::Grateful,
            AffectLabel::Curious,
            AffectLabel::Overwhelmed,
        ];

        for label in labels {
            let delta = affect_to_style_delta(label);
            assert!(
                delta.formality_delta.abs() <= 15,
                "{label:?} formality_delta out of bounds"
            );
            assert!(
                delta.verbosity_delta.abs() <= 15,
                "{label:?} verbosity_delta out of bounds"
            );
            assert!(
                delta.temperature_delta.abs() <= 0.15,
                "{label:?} temperature_delta out of bounds"
            );
        }
    }

    #[test]
    fn render_affect_block_neutral_is_empty() {
        assert!(render_affect_block(AffectLabel::Neutral, 1.0).is_empty());
    }

    #[test]
    fn render_affect_block_frustrated_contains_guidance() {
        let block = render_affect_block(AffectLabel::Frustrated, 0.5);
        assert!(block.contains("[Affect Guidance"));
        assert!(block.contains("frustrated"));
    }

    #[test]
    fn vad_style_delta_high_dominance_positive_formality() {
        let reading = AffectReading {
            label: AffectLabel::Excited,
            valence: 0.7,
            arousal: 0.8,
            dominance: 0.6,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        let delta = affect_vad_to_style_delta(&reading);
        assert!(
            delta.formality_delta > 0,
            "high dominance should produce positive formality, got {}",
            delta.formality_delta
        );
        assert!(
            delta.temperature_delta > 0.0,
            "high arousal should increase temperature, got {}",
            delta.temperature_delta
        );
    }

    #[test]
    fn vad_style_delta_within_bounds() {
        // Extreme values
        let extremes = [
            (1.0, 1.0, 1.0),
            (-1.0, 0.0, 0.0),
            (-1.0, 1.0, 1.0),
            (0.0, 0.0, 0.0),
        ];
        for (v, a, d) in extremes {
            let reading = AffectReading {
                label: AffectLabel::Neutral,
                valence: v,
                arousal: a,
                dominance: d,
                confidence: crate::contracts::scores::Confidence::new(1.0),
            };
            let delta = affect_vad_to_style_delta(&reading);
            assert!(
                delta.formality_delta.abs() <= 15,
                "formality out of bounds for V={v} A={a} D={d}: {}",
                delta.formality_delta
            );
            assert!(
                delta.verbosity_delta.abs() <= 15,
                "verbosity out of bounds for V={v} A={a} D={d}: {}",
                delta.verbosity_delta
            );
            assert!(
                delta.temperature_delta.abs() <= 0.15,
                "temperature out of bounds for V={v} A={a} D={d}: {}",
                delta.temperature_delta
            );
        }
    }
}
