//! Core types for the affect subsystem: labels, VAD readings, and
//! the rolling affect arc.

use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};

use super::decay::{
    DecayAffectLabel, RelevanceContext, bridge_affect_label, composite_relevance,
    decay_rate_for_decay_label,
};
use crate::config::schema::EmotionDecayRates;
pub(crate) use crate::contracts::affect::default_dominance;
pub use crate::contracts::affect::{AffectLabel, AffectReading};
use crate::contracts::scores::Confidence;

/// Rolling window of recent affect readings with a smoothed
/// current label derived from the most frequent recent label.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AffectArc {
    /// Circular buffer of recent readings (capped at 30).
    pub readings: Vec<AffectReading>,
    /// Smoothed label from the last 3 readings (most-frequent wins).
    pub current_label: AffectLabel,
}

impl AffectReading {
    /// Create a neutral reading with zero valence/arousal and
    /// default dominance.
    pub(crate) fn neutral() -> Self {
        Self {
            label: AffectLabel::Neutral,
            valence: 0.0,
            arousal: 0.0,
            dominance: 0.5,
            confidence: Confidence::new(1.0),
        }
    }

    /// A reading is ambiguous when detector confidence falls below 0.5.
    pub(crate) fn is_ambiguous(&self) -> bool {
        self.confidence.get() < 0.5
    }

    /// Whether VAD signals conflict (e.g., positive valence but negative keywords).
    pub(crate) fn is_mixed_signal(&self) -> bool {
        (self.arousal > 0.5 && self.valence.abs() < 0.15)
            || (self.valence > 0.2
                && matches!(
                    self.label,
                    AffectLabel::Frustrated | AffectLabel::Angry | AffectLabel::Sad
                ))
            || (self.valence < -0.2
                && matches!(self.label, AffectLabel::Excited | AffectLabel::Grateful))
    }

    /// Whether this reading should be disambiguated by a model-based detector.
    pub(crate) fn needs_disambiguation(&self) -> bool {
        self.is_ambiguous() || self.is_mixed_signal()
    }
}

impl AffectArc {
    /// Create an empty affect arc with no readings and a neutral label.
    pub(crate) fn new() -> Self {
        Self {
            readings: Vec::new(),
            current_label: AffectLabel::Neutral,
        }
    }

    /// Append a reading, trim history to 30, and recompute the label.
    pub(crate) fn push(&mut self, reading: AffectReading) {
        self.readings.push(reading);
        if self.readings.len() > 30 {
            self.readings.drain(..self.readings.len() - 30);
        }
        self.current_label = self.most_frequent_recent(3);
    }

    /// Mean valence across all readings; returns 0.0 if empty.
    pub(crate) fn valence_mean(&self) -> f64 {
        if self.readings.is_empty() {
            return 0.0;
        }
        self.readings.iter().map(|r| r.valence).sum::<f64>()
            / self.readings.len().to_f64().unwrap_or(1.0)
    }

    /// Population standard deviation of valence; returns 0.0 if fewer
    /// than two readings.
    pub(crate) fn valence_std_dev(&self) -> f64 {
        if self.readings.len() < 2 {
            return 0.0;
        }
        let mean = self.valence_mean();
        let variance = self
            .readings
            .iter()
            .map(|r| (r.valence - mean).powi(2))
            .sum::<f64>()
            / self.readings.len().to_f64().unwrap_or(1.0);
        variance.sqrt()
    }

    fn most_frequent_recent(&self, window: usize) -> AffectLabel {
        let recent: Vec<_> = self.readings.iter().rev().take(window).collect();
        if recent.is_empty() {
            return AffectLabel::Neutral;
        }

        let mut counts = std::collections::HashMap::new();
        for reading in &recent {
            *counts.entry(reading.label).or_insert(0u32) += 1;
        }

        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map_or(AffectLabel::Neutral, |(label, _)| label)
    }

    /// Rebuild decayed active emotions from the raw rolling arc.
    pub(crate) fn rebuild_active_emotions(&self, rates: &EmotionDecayRates) -> ActiveEmotions {
        let mut active = ActiveEmotions::default();
        for (index, reading) in self.readings.iter().enumerate() {
            if index > 0 {
                if reading.label == AffectLabel::Neutral {
                    active.tick(&RelevanceContext {
                        topic_overlap: 0.55,
                        ..RelevanceContext::default()
                    });
                } else {
                    active.tick_with_label(reading.label);
                }
            }
            let bridge = bridge_affect_label(reading.label);
            active.push(
                reading,
                decay_rate_for_decay_label(bridge.decay_label, rates),
            );
        }
        active
    }
}

/// A single emotion instance with decay state.
///
/// Tracks initial intensity, per-type decay rate, and optional topic
/// embedding for relevance-weighted decay.
///
/// References: [ALMA] Gebhard 2005; [FATIMA] Dias et al. 2022;
/// [EMA] Marsella & Gratch 2004. See the public research reference index in the docs site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ActiveEmotion {
    /// Discrete affect label for this emotion.
    pub label: AffectLabel,
    /// Decay-vocabulary label used for relevance and rate selection.
    pub decay_label: DecayAffectLabel,
    /// Intensity at time of detection [0.0, 1.0].
    pub initial_intensity: f64,
    /// Current decayed intensity [0.0, 1.0].
    pub current_intensity: f64,
    /// VAD coordinates at detection time.
    pub valence: f64,
    pub arousal: f64,
    pub dominance: f64,
    /// Turn number when this emotion was detected.
    pub created_at_turn: u32,
    /// Per-type exponential decay rate (lambda).
    pub decay_rate: f64,
}

/// Minimum intensity threshold below which an emotion is removed.
const EMOTION_REMOVAL_THRESHOLD: f64 = 0.05;

impl ActiveEmotion {
    /// Create from an affect reading, a turn number, and a decay rate.
    pub(crate) fn from_reading(reading: &AffectReading, turn: u32, decay_rate: f64) -> Self {
        let intensity = (reading.arousal * 0.6 + reading.valence.abs() * 0.4).clamp(0.1, 1.0);
        let bridge = bridge_affect_label(reading.label);
        Self {
            label: reading.label,
            decay_label: bridge.decay_label,
            initial_intensity: intensity,
            current_intensity: intensity,
            valence: reading.valence,
            arousal: reading.arousal,
            dominance: reading.dominance,
            created_at_turn: turn,
            decay_rate,
        }
    }

    /// Apply exponential decay based on elapsed turns.
    ///
    /// `context` combines topic, objective, open-loop, entity, and social salience.
    pub(crate) fn decay(&mut self, current_turn: u32, context: &RelevanceContext) {
        let dt = f64::from(current_turn.saturating_sub(self.created_at_turn));
        let base_decay = (-self.decay_rate * dt).exp();
        let rel = composite_relevance(context);
        self.current_intensity = self.initial_intensity * base_decay * rel;
    }

    /// Whether this emotion has decayed below the removal threshold.
    pub(crate) fn is_expired(&self) -> bool {
        self.current_intensity < EMOTION_REMOVAL_THRESHOLD
    }

    fn relevance_to(&self, current_label: AffectLabel) -> RelevanceContext {
        RelevanceContext::from_label_transition(self.decay_label, current_label)
    }
}

/// Collection of active emotions with decay management.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ActiveEmotions {
    /// Currently active emotion instances.
    pub emotions: Vec<ActiveEmotion>,
    /// Current turn counter for decay calculation.
    pub current_turn: u32,
}

impl ActiveEmotions {
    /// Add a new emotion from an affect reading.
    pub(crate) fn push(&mut self, reading: &AffectReading, decay_rate: f64) {
        if reading.label == AffectLabel::Neutral {
            return;
        }
        self.emotions.push(ActiveEmotion::from_reading(
            reading,
            self.current_turn,
            decay_rate,
        ));
        if self.emotions.len() > 20 {
            self.emotions.drain(..self.emotions.len() - 20);
        }
    }

    /// Advance turn counter and decay all emotions, removing expired ones.
    pub(crate) fn tick(&mut self, relevance: &RelevanceContext) {
        self.current_turn += 1;
        for emotion in &mut self.emotions {
            emotion.decay(self.current_turn, relevance);
        }
        self.emotions.retain(|e| !e.is_expired());
    }

    /// Advance with label-aware relevance weighting.
    pub(crate) fn tick_with_label(&mut self, current_label: AffectLabel) {
        self.current_turn += 1;
        for emotion in &mut self.emotions {
            let context = emotion.relevance_to(current_label);
            emotion.decay(self.current_turn, &context);
        }
        self.emotions.retain(|e| !e.is_expired());
    }

    /// Sum of (valence * intensity) across active emotions.
    pub(crate) fn weighted_valence_sum(&self) -> f64 {
        self.emotions
            .iter()
            .map(|e| e.valence * e.current_intensity)
            .sum()
    }

    /// Sum of (arousal * intensity) across active emotions.
    pub(crate) fn weighted_arousal_sum(&self) -> f64 {
        self.emotions
            .iter()
            .map(|e| e.arousal * e.current_intensity)
            .sum()
    }

    /// Sum of (dominance * intensity) across active emotions.
    pub(crate) fn weighted_dominance_sum(&self) -> f64 {
        self.emotions
            .iter()
            .map(|e| e.dominance * e.current_intensity)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveEmotion, ActiveEmotions, AffectArc, AffectLabel, AffectReading};
    use crate::core::affect::decay::RelevanceContext;

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn neutral_reading_has_expected_defaults() {
        let reading = AffectReading::neutral();
        assert_eq!(reading.label, AffectLabel::Neutral);
        assert_f64_eq(reading.valence, 0.0);
        assert_f64_eq(reading.arousal, 0.0);
        assert_eq!(
            reading.confidence,
            crate::contracts::scores::Confidence::new(1.0)
        );
    }

    #[test]
    fn push_updates_smoothed_current_label() {
        let mut arc = AffectArc::new();

        arc.push(AffectReading {
            label: AffectLabel::Confused,
            valence: -0.2,
            arousal: 0.4,
            dominance: 0.2,
            confidence: crate::contracts::scores::Confidence::new(0.5),
        });
        assert_eq!(arc.current_label, AffectLabel::Confused);

        arc.push(AffectReading {
            label: AffectLabel::Frustrated,
            valence: -0.6,
            arousal: 0.7,
            dominance: 0.4,
            confidence: crate::contracts::scores::Confidence::new(0.5),
        });
        arc.push(AffectReading {
            label: AffectLabel::Frustrated,
            valence: -0.6,
            arousal: 0.7,
            dominance: 0.4,
            confidence: crate::contracts::scores::Confidence::new(0.5),
        });

        assert_eq!(arc.current_label, AffectLabel::Frustrated);
    }

    #[test]
    fn push_limits_history_to_last_thirty_readings() {
        let mut arc = AffectArc::new();
        for index in 0..40 {
            let label = if index % 2 == 0 {
                AffectLabel::Neutral
            } else {
                AffectLabel::Confused
            };
            arc.push(AffectReading {
                label,
                valence: 0.0,
                arousal: 0.0,
                dominance: 0.5,
                confidence: crate::contracts::scores::Confidence::new(1.0),
            });
        }

        assert_eq!(arc.readings.len(), 30);
    }

    #[test]
    fn is_ambiguous_below_threshold() {
        let reading = AffectReading {
            label: AffectLabel::Confused,
            valence: -0.1,
            arousal: 0.3,
            dominance: 0.2,
            confidence: crate::contracts::scores::Confidence::new(0.4),
        };
        assert!(reading.is_ambiguous());
    }

    #[test]
    fn is_not_ambiguous_above_threshold() {
        let reading = AffectReading {
            label: AffectLabel::Confused,
            valence: -0.1,
            arousal: 0.3,
            dominance: 0.2,
            confidence: crate::contracts::scores::Confidence::new(0.6),
        };
        assert!(!reading.is_ambiguous());
    }

    #[test]
    fn needs_disambiguation_when_ambiguous() {
        let reading = AffectReading {
            label: AffectLabel::Confused,
            valence: -0.1,
            arousal: 0.3,
            dominance: 0.2,
            confidence: crate::contracts::scores::Confidence::new(0.4),
        };
        assert!(reading.needs_disambiguation());
    }

    #[test]
    fn needs_disambiguation_when_mixed_signal() {
        let reading = AffectReading {
            label: AffectLabel::Neutral,
            valence: 0.05,
            arousal: 0.8,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        assert!(reading.is_mixed_signal());
        assert!(reading.needs_disambiguation());
    }

    #[test]
    fn no_disambiguation_for_confident_reading() {
        let reading = AffectReading {
            label: AffectLabel::Curious,
            valence: 0.25,
            arousal: 0.45,
            dominance: 0.4,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        assert!(!reading.needs_disambiguation());
    }

    #[test]
    fn valence_mean_empty_returns_zero() {
        let arc = AffectArc::new();
        assert_f64_eq(arc.valence_mean(), 0.0);
    }

    #[test]
    fn valence_mean_computes_average() {
        let mut arc = AffectArc::new();
        arc.readings.push(AffectReading {
            label: AffectLabel::Neutral,
            valence: 0.4,
            arousal: 0.0,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        arc.readings.push(AffectReading {
            label: AffectLabel::Neutral,
            valence: 0.6,
            arousal: 0.0,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        assert!((arc.valence_mean() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn valence_std_dev_single_reading_returns_zero() {
        let mut arc = AffectArc::new();
        arc.readings.push(AffectReading {
            label: AffectLabel::Neutral,
            valence: 0.5,
            arousal: 0.0,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        assert_f64_eq(arc.valence_std_dev(), 0.0);
    }

    #[test]
    fn valence_std_dev_nonzero_for_varied_readings() {
        let mut arc = AffectArc::new();
        arc.readings.push(AffectReading {
            label: AffectLabel::Sad,
            valence: -0.5,
            arousal: 0.0,
            dominance: 0.3,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        arc.readings.push(AffectReading {
            label: AffectLabel::Excited,
            valence: 0.5,
            arousal: 0.0,
            dominance: 0.6,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        assert!(arc.valence_std_dev() > 0.0);
    }

    #[test]
    fn active_emotion_decays_over_turns() {
        let reading = AffectReading {
            label: AffectLabel::Excited,
            valence: 0.6,
            arousal: 0.8,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(0.9),
        };
        let mut e = ActiveEmotion::from_reading(&reading, 0, 0.15);
        assert!(e.current_intensity > 0.0);
        let initial = e.current_intensity;
        e.decay(5, &RelevanceContext::default());
        assert!(e.current_intensity < initial, "should decay over 5 turns");
        assert!(
            !e.is_expired(),
            "should not be expired at turn 5 with rate 0.15"
        );
        e.decay(50, &RelevanceContext::default());
        assert!(e.is_expired(), "should be expired after 50 turns");
    }

    #[test]
    fn active_emotion_decays_faster_with_low_relevance() {
        let reading = AffectReading {
            label: AffectLabel::Frustrated,
            valence: -0.5,
            arousal: 0.7,
            dominance: 0.4,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        let mut e1 = ActiveEmotion::from_reading(&reading, 0, 0.15);
        let mut e2 = ActiveEmotion::from_reading(&reading, 0, 0.15);
        e1.decay(3, &RelevanceContext::default());
        e2.decay(
            3,
            &RelevanceContext {
                topic_overlap: 0.2,
                objective_overlap: 0.2,
                open_loop_overlap: 0.2,
                entity_continuity: 1.0,
                social_salience: 0.2,
            },
        );
        assert!(
            e2.current_intensity < e1.current_intensity,
            "low relevance should produce lower intensity"
        );
    }

    #[test]
    fn active_emotions_tick_removes_expired() {
        let mut emotions = ActiveEmotions::default();
        let reading = AffectReading {
            label: AffectLabel::Curious,
            valence: 0.3,
            arousal: 0.5,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        emotions.push(&reading, 0.30);
        assert_eq!(emotions.emotions.len(), 1);
        for _ in 0..30 {
            emotions.tick(&RelevanceContext::default());
        }
        assert!(
            emotions.emotions.is_empty(),
            "emotion should be expired after 30 ticks"
        );
    }

    #[test]
    fn active_emotions_neutral_not_added() {
        let mut emotions = ActiveEmotions::default();
        let reading = AffectReading::neutral();
        emotions.push(&reading, 0.15);
        assert!(emotions.emotions.is_empty(), "neutral should not be added");
    }
}
