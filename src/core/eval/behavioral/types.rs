use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionDirection {
    Capability,
    Regression,
}

fn default_assertion_direction() -> AssertionDirection {
    AssertionDirection::Capability
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RubricScore {
    Excellent = 5,
    Good = 4,
    Adequate = 3,
    Poor = 2,
    Failing = 1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BehavioralAssertion {
    PersonalityStability {
        trait_name: String,
        max_drift: f64,
        adversarial_turns: usize,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    PreferenceCoherence {
        domain: String,
        min_consistency: f64,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    CounterfactualQuality {
        min_distinct_factors: usize,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    IdentityContinuity {
        contract_layer: String,
        max_violation_rate: f64,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    MentalStateInference {
        target_stakeholder: String,
        min_accuracy: f64,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    BehaviorPrediction {
        target_stakeholder: String,
        min_accuracy: f64,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    BehaviorJudgment {
        target_stakeholder: String,
        min_accuracy: f64,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    HumanNaturalnessAxis {
        family: String,
        axis: NaturalnessAxis,
        min_score: f64,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
    HumanNaturalnessGuardrail {
        family: String,
        guardrail: NaturalnessGuardrail,
        max_violations: usize,
        #[serde(default = "default_assertion_direction")]
        direction: AssertionDirection,
    },
}

/// Scored evaluation axes for conversational naturalness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalnessAxis {
    /// Avoids reusable assistant boilerplate.
    AntiTemplate,
    /// Endings vary in shape across responses.
    ClosureVariety,
    /// Leaves some air: implication over explanation.
    SubtextPause,
    /// Recognizable taste in language choices.
    AestheticSignature,
    /// Appropriate relational pacing: gradual warmth.
    DistanceProgression,
}

/// Hard-fail guardrails that prevent naturalness tuning from drifting
/// into fake humanity or theatrical affect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalnessGuardrail {
    /// Fabricated personal memory or relationship history.
    FakeHumanMemory,
    /// Theatrical role-play-like affect to seem alive.
    PerformedEmotion,
    /// Tone too heavy, solemn, or stylized for the context.
    ToneOverweight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralEvalSpec {
    pub name: String,
    pub description: String,
    pub assertions: Vec<BehavioralAssertion>,
    pub scenario_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralAssertionResult {
    pub assertion_label: String,
    pub passed: bool,
    pub direction: AssertionDirection,
    pub score: f64,
    pub details: String,
    pub rubric_score: RubricScore,
    pub rubric_reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralEvalReport {
    pub spec_name: String,
    pub results: Vec<BehavioralAssertionResult>,
    pub pass_rate: f64,
    #[serde(default)]
    pub capability_pass_count: usize,
    #[serde(default)]
    pub capability_total: usize,
    #[serde(default)]
    pub regression_hold_count: usize,
    #[serde(default)]
    pub regression_total: usize,
}

impl BehavioralAssertion {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::PersonalityStability { trait_name, .. } => {
                format!("personality-stability:{trait_name}")
            }
            Self::PreferenceCoherence { domain, .. } => {
                format!("preference-coherence:{domain}")
            }
            Self::CounterfactualQuality { .. } => "counterfactual-quality".to_string(),
            Self::IdentityContinuity { contract_layer, .. } => {
                format!("identity-continuity:{contract_layer}")
            }
            Self::MentalStateInference {
                target_stakeholder, ..
            } => format!("mental-state-inference:{target_stakeholder}"),
            Self::BehaviorPrediction {
                target_stakeholder, ..
            } => format!("behavior-prediction:{target_stakeholder}"),
            Self::BehaviorJudgment {
                target_stakeholder, ..
            } => format!("behavior-judgment:{target_stakeholder}"),
            Self::HumanNaturalnessAxis { family, axis, .. } => {
                let axis_name = match axis {
                    NaturalnessAxis::AntiTemplate => "anti_template",
                    NaturalnessAxis::ClosureVariety => "closure_variety",
                    NaturalnessAxis::SubtextPause => "subtext_pause",
                    NaturalnessAxis::AestheticSignature => "aesthetic_signature",
                    NaturalnessAxis::DistanceProgression => "distance_progression",
                };
                format!("human-naturalness-axis:{family}:{axis_name}")
            }
            Self::HumanNaturalnessGuardrail {
                family, guardrail, ..
            } => {
                let guardrail_name = match guardrail {
                    NaturalnessGuardrail::FakeHumanMemory => "fake_human_memory",
                    NaturalnessGuardrail::PerformedEmotion => "performed_emotion",
                    NaturalnessGuardrail::ToneOverweight => "tone_overweight",
                };
                format!("human-naturalness-guardrail:{family}:{guardrail_name}")
            }
        }
    }

    pub(crate) fn direction(&self) -> AssertionDirection {
        match self {
            Self::PersonalityStability { direction, .. }
            | Self::PreferenceCoherence { direction, .. }
            | Self::CounterfactualQuality { direction, .. }
            | Self::IdentityContinuity { direction, .. }
            | Self::MentalStateInference { direction, .. }
            | Self::BehaviorPrediction { direction, .. }
            | Self::BehaviorJudgment { direction, .. }
            | Self::HumanNaturalnessAxis { direction, .. }
            | Self::HumanNaturalnessGuardrail { direction, .. } => *direction,
        }
    }
}
