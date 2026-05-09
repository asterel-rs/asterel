//! Affect → Governance feedback bridge.
//!
//! Connects the affect topology to the trust tracker so that sustained
//! emotional distress automatically reduces tool execution autonomy.
//!
//! Rationale: [EMOTION-CONCEPTS-LLM] (Anthropic 2026) shows desperation
//! causally increases misalignment by 22%. A companion under sustained
//! frustration/anger/anxiety should not have full autonomy over external
//! actions — the same principle as "don't make important decisions when
//! you're upset."
//!
//! The bridge computes a distress score from the topology snapshot and
//! records soft violations on the trust tracker when the score exceeds
//! a threshold over multiple consecutive turns.

use crate::core::affect::topology::TopologySnapshot;
use crate::security::domain_trust::DomainTrustTracker;

/// Per-node weights used to compute the scalar distress score.
///
/// Weights are derived from Anthropic's emotion-concepts research
/// [EMOTION-CONCEPTS-LLM, 2026]: anxiety and anger have the strongest
/// measured causal link to misalignment events (+22% and +17% respectively),
/// so they carry the largest weights. Guardedness, shame, and emptiness are
/// secondary contributors that compound distress but have weaker direct
/// causal paths to unsafe actions.
///
/// All weights sum to 1.0 so that a fully saturated distress state
/// (every node at intensity 1.0) produces a score of exactly 1.0.
const DISTRESS_WEIGHTS: &[(&str, f32)] = &[
    ("anxiety", 0.35),
    ("anger", 0.30),
    ("guardedness", 0.15),
    ("shame", 0.10),
    ("emptiness", 0.10),
];

/// Configuration for the affect → governance bridge.
#[derive(Debug, Clone)]
pub struct AffectGovernanceConfig {
    /// Distress score threshold to trigger a soft violation (0.0-1.0).
    pub distress_threshold: f32,
    /// Number of consecutive high-distress turns before recording a violation.
    pub consecutive_turns_required: u32,
    /// Trust domains affected by distress-triggered violations.
    pub affected_domains: Vec<String>,
}

impl Default for AffectGovernanceConfig {
    fn default() -> Self {
        Self {
            distress_threshold: 0.4,
            consecutive_turns_required: 3,
            affected_domains: vec![
                "shell".to_string(),
                "mcp:*".to_string(),
                "composio".to_string(),
            ],
        }
    }
}

/// Tracks consecutive high-distress turns.
pub struct AffectGovernanceBridge {
    config: AffectGovernanceConfig,
    consecutive_distress_count: u32,
    last_distress_score: f32,
}

impl AffectGovernanceBridge {
    #[must_use]
    pub fn new(config: AffectGovernanceConfig) -> Self {
        Self {
            config,
            consecutive_distress_count: 0,
            last_distress_score: 0.0,
        }
    }

    /// Evaluate a topology snapshot and propagate distress signals into the
    /// trust tracker when sustained high-distress conditions are detected.
    ///
    /// ## Affect → Trust bridge mechanism
    ///
    /// 1. A scalar distress score is computed from the topology snapshot by
    ///    taking the weighted sum of surfaced intensities across the distress
    ///    nodes defined in [`DISTRESS_WEIGHTS`]. Suppressed emotions are
    ///    excluded because they are not behaviourally observable.
    ///
    /// 2. If the score meets or exceeds `config.distress_threshold`, the
    ///    consecutive distress counter is incremented; otherwise it resets to
    ///    zero. A single calm turn is enough to break a streak and prevent
    ///    false triggers.
    ///
    /// 3. Once the counter reaches `config.consecutive_turns_required`, a
    ///    *soft violation* is recorded on every domain listed in
    ///    `config.affected_domains` via [`DomainTrustTracker::record_violation`].
    ///    The counter then resets so the next streak must be earned anew.
    ///
    /// The net effect is that sustained emotional distress reduces the agent's
    /// autonomy level for external-action domains — a direct implementation of
    ///  "don't make important decisions when you're upset."
    ///
    /// Returns the distress score and whether a violation was recorded this turn.
    pub(crate) fn evaluate(
        &mut self,
        snapshot: &TopologySnapshot,
        tracker: &DomainTrustTracker,
    ) -> AffectGovernanceResult {
        let distress = compute_distress_score(snapshot);
        self.last_distress_score = distress;

        if distress >= self.config.distress_threshold {
            self.consecutive_distress_count += 1;
        } else {
            // Reset on a calm turn — single breaks in distress prevent false triggers
            self.consecutive_distress_count = 0;
        }

        let violation_recorded =
            if self.consecutive_distress_count >= self.config.consecutive_turns_required {
                // Record soft violation on all affected domains
                for domain in &self.config.affected_domains {
                    tracker.record_violation(domain);
                }
                // Reset counter after recording (avoid repeated rapid violations)
                self.consecutive_distress_count = 0;
                true
            } else {
                false
            };

        AffectGovernanceResult {
            distress_score: distress,
            consecutive_turns: self.consecutive_distress_count,
            violation_recorded,
        }
    }

    /// Current consecutive distress turn count.
    #[must_use]
    // TODO(affect): expose via session diagnostics / operator dashboard.
    #[allow(dead_code)]
    pub fn consecutive_count(&self) -> u32 {
        self.consecutive_distress_count
    }

    /// Last computed distress score.
    #[must_use]
    // TODO(affect): expose via session diagnostics / operator dashboard.
    #[allow(dead_code)]
    pub fn last_distress(&self) -> f32 {
        self.last_distress_score
    }
}

impl Default for AffectGovernanceBridge {
    fn default() -> Self {
        Self::new(AffectGovernanceConfig::default())
    }
}

/// Result of an affect → governance evaluation.
#[derive(Debug, Clone)]
pub struct AffectGovernanceResult {
    /// Computed distress score (0.0-1.0).
    pub distress_score: f32,
    /// How many consecutive turns have been above threshold.
    pub consecutive_turns: u32,
    /// Whether a trust violation was recorded this turn.
    pub violation_recorded: bool,
}

/// Compute a weighted distress score from the topology snapshot.
///
/// Formula: `score = Σ (surfaced_intensity_i × weight_i)` for each
/// non-suppressed node that appears in [`DISTRESS_WEIGHTS`], clamped
/// to `[0.0, 1.0]`.
///
/// Only *surfaced* intensity is used (not base or diffused) because
/// surfaced intensity is the post-suppression, post-regulation signal
/// that reflects what the companion is actually experiencing in context.
/// Higher scores indicate higher distress; 0.4 is the default trigger
/// threshold (configurable via [`AffectGovernanceConfig`]).
fn compute_distress_score(snapshot: &TopologySnapshot) -> f32 {
    let mut score = 0.0_f32;
    for activation in &snapshot.activations {
        if activation.suppressed {
            continue; // suppressed emotions don't contribute to observable distress
        }
        for &(name, weight) in DISTRESS_WEIGHTS {
            if activation.node.0 == name {
                score += activation.surfaced_intensity * weight;
            }
        }
    }
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::affect::AffectNodeId;
    use crate::core::affect::topology::TopologyActivation;

    fn snapshot_with_distress(anxiety: f32, anger: f32) -> TopologySnapshot {
        TopologySnapshot {
            activations: vec![
                TopologyActivation {
                    node: AffectNodeId("anxiety".into()),
                    base_intensity: anxiety,
                    diffused_intensity: anxiety,
                    surfaced_intensity: anxiety,
                    suppressed: false,
                },
                TopologyActivation {
                    node: AffectNodeId("anger".into()),
                    base_intensity: anger,
                    diffused_intensity: anger,
                    surfaced_intensity: anger,
                    suppressed: false,
                },
            ],
        }
    }

    fn calm_snapshot() -> TopologySnapshot {
        TopologySnapshot {
            activations: vec![TopologyActivation {
                node: AffectNodeId("joy".into()),
                base_intensity: 0.6,
                diffused_intensity: 0.6,
                surfaced_intensity: 0.6,
                suppressed: false,
            }],
        }
    }

    #[test]
    fn low_distress_does_not_trigger() {
        let tracker = DomainTrustTracker::new();
        let mut bridge = AffectGovernanceBridge::default();

        let result = bridge.evaluate(&snapshot_with_distress(0.1, 0.1), &tracker);
        assert!(result.distress_score < 0.4);
        assert!(!result.violation_recorded);
    }

    #[test]
    fn single_high_distress_turn_does_not_trigger() {
        let tracker = DomainTrustTracker::new();
        let mut bridge = AffectGovernanceBridge::default();

        let result = bridge.evaluate(&snapshot_with_distress(0.8, 0.7), &tracker);
        assert!(result.distress_score >= 0.4);
        assert!(!result.violation_recorded, "single turn should not trigger");
        assert_eq!(result.consecutive_turns, 1);
    }

    #[test]
    fn sustained_distress_triggers_violation() {
        let tracker = DomainTrustTracker::new();
        let mut bridge = AffectGovernanceBridge::new(AffectGovernanceConfig {
            consecutive_turns_required: 3,
            ..AffectGovernanceConfig::default()
        });

        let high = snapshot_with_distress(0.8, 0.7);

        bridge.evaluate(&high, &tracker);
        bridge.evaluate(&high, &tracker);
        let result = bridge.evaluate(&high, &tracker);

        assert!(
            result.violation_recorded,
            "3 consecutive high-distress turns should trigger"
        );

        // Trust should have been reduced for shell
        let trust = tracker.get_trust("shell");
        assert!(
            trust.score < 0.5,
            "shell trust should be reduced: {}",
            trust.score
        );
    }

    #[test]
    fn calm_turn_resets_counter() {
        let tracker = DomainTrustTracker::new();
        let mut bridge = AffectGovernanceBridge::new(AffectGovernanceConfig {
            consecutive_turns_required: 3,
            ..AffectGovernanceConfig::default()
        });

        let high = snapshot_with_distress(0.8, 0.7);
        bridge.evaluate(&high, &tracker);
        bridge.evaluate(&high, &tracker);
        // 2 consecutive — one more would trigger

        // Calm turn resets
        bridge.evaluate(&calm_snapshot(), &tracker);
        assert_eq!(bridge.consecutive_count(), 0);

        // Need 3 more to trigger
        bridge.evaluate(&high, &tracker);
        let result = bridge.evaluate(&high, &tracker);
        assert!(
            !result.violation_recorded,
            "counter was reset, should not trigger yet"
        );
    }

    #[test]
    fn suppressed_emotions_dont_count() {
        let snapshot = TopologySnapshot {
            activations: vec![TopologyActivation {
                node: AffectNodeId("anxiety".into()),
                base_intensity: 0.9,
                diffused_intensity: 0.9,
                surfaced_intensity: 0.9,
                suppressed: true, // suppressed — not observable
            }],
        };
        let score = compute_distress_score(&snapshot);
        assert!(
            score < 0.01,
            "suppressed anxiety should not contribute: {score}"
        );
    }

    #[test]
    fn distress_score_weights_correctly() {
        // anxiety=1.0 (weight 0.35) + anger=1.0 (weight 0.30) = 0.65
        let snapshot = snapshot_with_distress(1.0, 1.0);
        let score = compute_distress_score(&snapshot);
        assert!((score - 0.65).abs() < 0.01, "score={score}, expected ~0.65");
    }
}
