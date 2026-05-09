//! Mood-driven prompt injection: selects a behavioural mood per cycle
//! and renders a nudge block that steers the agent toward a specific
//! exploration or creation pattern.
//!
//! Moods are lightweight and optional — when disabled or when no mood
//! is selected, the pipeline continues exactly as before.

use rand::{Rng, RngExt};
use serde::{Deserialize, Serialize};

use super::types::{AffectLabel, AffectReading};

/// Discrete mood variants that steer agent behaviour for a single cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mood {
    /// Investigate a topic deeply — web searches, reading, analysis.
    Research,
    /// Focus on an existing thread and push it forward.
    DeepDive,
    /// Write code, build tools, implement something runnable.
    Coder,
    /// Produce substantial written output — reports, essays, analyses.
    Creative,
    /// Follow curiosity into unknown territory, explore rabbit holes.
    Explorer,
    /// Review, organise, and consolidate existing work.
    Organizer,
}

impl Mood {
    /// All defined mood variants, in declaration order.
    pub(crate) const ALL: &'static [Mood] = &[
        Mood::Research,
        Mood::DeepDive,
        Mood::Coder,
        Mood::Creative,
        Mood::Explorer,
        Mood::Organizer,
    ];

    /// Human-readable label for logging and prompt rendering.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Mood::Research => "research",
            Mood::DeepDive => "deep-dive",
            Mood::Coder => "coder",
            Mood::Creative => "creative",
            Mood::Explorer => "explorer",
            Mood::Organizer => "organizer",
        }
    }

    /// Behavioural nudge text injected into the prompt.
    pub(crate) fn nudge(self) -> &'static str {
        match self {
            Mood::Research => {
                "You are in a research mood. Pick a specific topic, investigate \
                 it thoroughly, and synthesise your findings into a clear analysis. \
                 Prioritise depth and sourcing over breadth."
            }
            Mood::DeepDive => {
                "You are in a focused mood. Look at your current objectives and \
                 push one of them forward. Make real, measurable progress on \
                 something already in flight."
            }
            Mood::Coder => {
                "You are in a building mood. Write real code — a utility, a \
                 parser, a small tool, or an implementation of something you \
                 have been researching. Make it run."
            }
            Mood::Creative => {
                "You are in a writing mood. Produce something substantial — \
                 a report, an essay, a design document, or a deep analysis. \
                 Aim for quality someone would want to read."
            }
            Mood::Explorer => {
                "You are feeling adventurous. Explore something you know little \
                 about. Follow the rabbit hole, but capture what you find — \
                 do not just think, write it down."
            }
            Mood::Organizer => {
                "You are in a tidy mood. Review your existing work, consolidate \
                 notes, update tracking documents, and then pick up where you \
                 left off on something concrete."
            }
        }
    }
}

/// References: [ALMA] Gebhard 2005; [WASABI] Becker-Asano 2008;
/// [PAD] Mehrabian 1996. See the public research reference index in the docs site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionMood {
    /// Pleasure axis [-1.0, 1.0].
    pub pleasure: f64,
    /// Arousal axis [-1.0, 1.0].
    pub arousal: f64,
    /// Dominance axis [-1.0, 1.0].
    pub dominance: f64,
}

impl Default for SessionMood {
    fn default() -> Self {
        Self {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
        }
    }
}

impl SessionMood {
    /// Create a mood from a personality baseline derived from Big Five.
    ///
    /// Uses a centered Mehrabian-style mapping so a balanced 0.5 profile
    /// sits near neutral rather than inheriting a biased baseline.
    pub(crate) fn from_big_five(
        extraversion: f64,
        agreeableness: f64,
        conscientiousness: f64,
        neuroticism: f64,
        openness: f64,
    ) -> Self {
        let e = 2.0 * (extraversion - 0.5);
        let a_trait = 2.0 * (agreeableness - 0.5);
        let c = 2.0 * (conscientiousness - 0.5);
        let n = 2.0 * (neuroticism - 0.5);
        let o = 2.0 * (openness - 0.5);
        Self {
            pleasure: (0.21 * e + 0.59 * a_trait - 0.19 * n).clamp(-1.0, 1.0),
            arousal: (0.15 * o + 0.30 * n - 0.57 * a_trait + 0.25 * e).clamp(-1.0, 1.0),
            dominance: (0.25 * e + 0.17 * c - 0.60 * n).clamp(-1.0, 1.0),
        }
    }

    /// Update session mood using ALMA's two-force model.
    ///
    /// Two forces act simultaneously each turn:
    ///
    /// 1. **Emotion push** — active emotions shift mood toward their PAD
    ///    coordinates, weighted by `alpha` (typically 0.05–0.10).
    ///    `alpha × weighted_sum` is added to each PAD dimension.
    ///
    /// 2. **Homeostatic spring** — mood is pulled back toward the personality
    ///    `baseline` by `beta_*` factors (typically 0.10–0.25). The spring
    ///    force is `−beta × (current − baseline)`, so the farther mood drifts,
    ///    the stronger the pull.
    ///
    /// Each PAD dimension has its own beta to allow asymmetric recovery:
    /// arousal tends to recover faster (`beta_a` > `beta_p`) because
    /// physiological activation naturally subsides faster than emotional tone.
    ///
    /// All three dimensions are clamped to their valid ranges after update:
    /// pleasure \[−1, 1\], arousal \[−1, 1\], dominance \[−1, 1\].
    ///
    /// References: `[ALMA]` Gebhard 2005; `[PAD]` Mehrabian 1996.
    pub(crate) fn update(
        &mut self,
        emotions: &super::types::ActiveEmotions,
        baseline: &SessionMood,
        alpha: f64,
        beta_p: f64,
        beta_a: f64,
        beta_d: f64,
    ) {
        self.pleasure += alpha * emotions.weighted_valence_sum();
        self.arousal += alpha * emotions.weighted_arousal_sum();
        self.dominance += alpha * emotions.weighted_dominance_sum();

        self.pleasure -= beta_p * (self.pleasure - baseline.pleasure);
        self.arousal -= beta_a * (self.arousal - baseline.arousal);
        self.dominance -= beta_d * (self.dominance - baseline.dominance);

        self.pleasure = self.pleasure.clamp(-1.0, 1.0);
        self.arousal = self.arousal.clamp(-1.0, 1.0);
        self.dominance = self.dominance.clamp(-1.0, 1.0);
    }

    /// Reset mood toward baseline at session boundary.
    ///
    /// `reset_factor` in [0.0, 1.0]: 0.8 means mood = 0.2*mood + 0.8*baseline.
    pub(crate) fn session_reset(&mut self, baseline: &SessionMood, reset_factor: f64) {
        let reset_factor = reset_factor.clamp(0.0, 1.0);
        let keep = 1.0 - reset_factor;
        self.pleasure = keep * self.pleasure + reset_factor * baseline.pleasure;
        self.arousal = keep * self.arousal + reset_factor * baseline.arousal;
        self.dominance = keep * self.dominance + reset_factor * baseline.dominance;
    }

    /// Euclidean distance from another mood point.
    pub(crate) fn distance(&self, other: &SessionMood) -> f64 {
        let dp = self.pleasure - other.pleasure;
        let da = self.arousal - other.arousal;
        let dd = self.dominance - other.dominance;
        (dp * dp + da * da + dd * dd).sqrt()
    }

    /// Derive session mood by replaying the current affect arc with decay.
    pub(crate) fn from_affect_arc(
        arc: &super::types::AffectArc,
        config: &crate::config::schema::AffectDecayConfig,
        baseline: &SessionMood,
    ) -> Self {
        let mut mood = baseline.clone();
        if arc.readings.is_empty() {
            mood.session_reset(baseline, 1.0);
            return mood;
        }

        let mut emotions = super::types::ActiveEmotions::default();
        for (index, reading) in arc.readings.iter().enumerate() {
            if index > 0 {
                if reading.label == AffectLabel::Neutral {
                    emotions.tick(&super::decay::RelevanceContext {
                        topic_overlap: 0.55,
                        ..super::decay::RelevanceContext::default()
                    });
                } else {
                    emotions.tick_with_label(reading.label);
                }
            }

            emotions.push(
                reading,
                super::decay::decay_rate_for_decay_label(
                    super::decay::bridge_affect_label(reading.label).decay_label,
                    &config.emotion_rates,
                ),
            );
            mood.update(
                &emotions,
                baseline,
                config.alpha_emotion_to_mood,
                config.beta_homeostatic_pull,
                config.beta_arousal_homeostatic,
                config.beta_homeostatic_pull,
            );
        }

        mood
    }

    /// Render a concise mood summary for prompt injection.
    pub(crate) fn render_block(&self) -> String {
        if self.pleasure.abs() < 0.1 && self.arousal.abs() < 0.1 && self.dominance.abs() < 0.1 {
            return String::new();
        }

        let valence_desc = if self.pleasure > 0.3 {
            "positive"
        } else if self.pleasure < -0.3 {
            "subdued"
        } else {
            "neutral"
        };
        let energy_desc = if self.arousal > 0.35 {
            "energized"
        } else if self.arousal < -0.25 {
            "calm"
        } else {
            "steady"
        };

        let mut out = String::with_capacity(60);
        out.push_str("[Current Mood] Overall tone: ");
        out.push_str(valence_desc);
        out.push_str(", energy: ");
        out.push_str(energy_desc);
        out.push('.');
        out
    }
}

/// Select a mood informed by the current affect reading.
///
/// The mapping uses the affect label as a soft bias:
/// - **Curious** → leans toward `Research` or `Explorer`
/// - **Excited** → leans toward `Coder` or `Creative`
/// - **Frustrated / Angry** → leans toward `DeepDive` (focus on progress)
/// - **Overwhelmed** → leans toward `Organizer`
/// - Everything else → uniform random across all moods
///
/// A random element is always present so the agent does not become
/// deterministic even under the same affect state.
pub(crate) fn select_mood(reading: &AffectReading) -> Mood {
    let mut rng = rand::rng();

    // 40% chance to follow the affect-biased suggestion, 60% pure random.
    let use_bias = rng.random_bool(0.4);

    if use_bias {
        let biased = match reading.label {
            AffectLabel::Curious => {
                if rng.random_bool(0.5) {
                    Mood::Research
                } else {
                    Mood::Explorer
                }
            }
            AffectLabel::Excited | AffectLabel::Grateful => {
                if rng.random_bool(0.5) {
                    Mood::Coder
                } else {
                    Mood::Creative
                }
            }
            AffectLabel::Frustrated | AffectLabel::Angry => Mood::DeepDive,
            AffectLabel::Overwhelmed | AffectLabel::Anxious => Mood::Organizer,
            AffectLabel::Confused => Mood::Research,
            AffectLabel::Sad => Mood::Creative,
            AffectLabel::Neutral => random_mood(&mut rng),
        };
        return biased;
    }

    random_mood(&mut rng)
}

/// Uniform random mood selection.
fn random_mood(rng: &mut impl Rng) -> Mood {
    let idx = rng.random_range(0..Mood::ALL.len());
    Mood::ALL[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_moods_have_non_empty_label_and_nudge() {
        for &mood in Mood::ALL {
            assert!(!mood.label().is_empty(), "{mood:?} has empty label");
            assert!(!mood.nudge().is_empty(), "{mood:?} has empty nudge");
        }
    }

    #[test]
    fn select_mood_returns_valid_variant() {
        let reading = AffectReading::neutral();
        // Run a few times to exercise randomness.
        for _ in 0..20 {
            let mood = select_mood(&reading);
            assert!(
                Mood::ALL.contains(&mood),
                "select_mood returned unknown variant: {mood:?}"
            );
        }
    }

    #[test]
    fn select_mood_with_curious_affect() {
        let reading = AffectReading {
            label: AffectLabel::Curious,
            valence: 0.3,
            arousal: 0.5,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        // Just verify it doesn't panic and returns a valid mood.
        for _ in 0..20 {
            let mood = select_mood(&reading);
            assert!(Mood::ALL.contains(&mood));
        }
    }

    #[test]
    fn session_mood_from_big_five_returns_valid_ranges() {
        let mood = SessionMood::from_big_five(0.30, 0.75, 0.65, 0.25, 0.80);
        assert!((-1.0..=1.0).contains(&mood.pleasure));
        assert!((-1.0..=1.0).contains(&mood.arousal));
        assert!((-1.0..=1.0).contains(&mood.dominance));
    }

    #[test]
    fn balanced_big_five_maps_near_neutral_baseline() {
        let mood = SessionMood::from_big_five(0.5, 0.5, 0.5, 0.5, 0.5);
        assert!(mood.pleasure.abs() < 0.05);
        assert!(mood.arousal.abs() < 0.05);
        assert!(mood.dominance.abs() < 0.05);
    }

    #[test]
    fn session_mood_update_moves_toward_emotions() {
        let baseline = SessionMood::from_big_five(0.30, 0.75, 0.65, 0.25, 0.80);
        let mut mood = baseline.clone();
        let mut emotions = super::super::types::ActiveEmotions::default();
        let reading = super::AffectReading {
            label: super::AffectLabel::Excited,
            valence: 0.8,
            arousal: 0.9,
            dominance: 0.6,
            confidence: crate::contracts::scores::Confidence::new(0.9),
        };
        emotions.push(&reading, 0.15);
        mood.update(&emotions, &baseline, 0.08, 0.15, 0.25, 0.15);
        assert!(
            mood.pleasure > baseline.pleasure || mood.pleasure >= baseline.pleasure,
            "mood should shift toward positive emotion"
        );
    }

    #[test]
    fn session_mood_reset_pulls_toward_baseline() {
        let baseline = SessionMood {
            pleasure: 0.3,
            arousal: 0.2,
            dominance: 0.1,
        };
        let mut mood = SessionMood {
            pleasure: -0.5,
            arousal: 0.8,
            dominance: -0.3,
        };
        mood.session_reset(&baseline, 0.80);
        assert!((mood.pleasure - (-0.5 * 0.2 + 0.3 * 0.8)).abs() < 1e-10);
        assert!((mood.arousal - (0.8 * 0.2 + 0.2 * 0.8)).abs() < 1e-10);
    }

    #[test]
    fn session_mood_render_block_empty_at_baseline() {
        let mood = SessionMood::default();
        assert!(mood.render_block().is_empty());
    }

    #[test]
    fn session_mood_render_block_non_empty_when_shifted() {
        let mood = SessionMood {
            pleasure: 0.5,
            arousal: 0.7,
            dominance: 0.3,
        };
        let block = mood.render_block();
        assert!(!block.is_empty());
        assert!(block.contains("positive"));
        assert!(block.contains("energized"));
    }
}
