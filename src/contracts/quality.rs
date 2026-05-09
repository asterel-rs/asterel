use serde::{Deserialize, Serialize};

/// Weights for combining quality vector dimensions into a composite.
///
/// Weights are normalised at application time so they need not
/// sum to 1.0 in configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityVectorWeights {
    pub task_completion: f32,
    pub tool_effectiveness: f32,
    pub retrieval_utilization: f32,
    pub contradiction_safety: f32,
    pub user_friction: f32,
    pub explanation_quality: f32,
}

impl Default for QualityVectorWeights {
    fn default() -> Self {
        Self {
            task_completion: 0.25,
            tool_effectiveness: 0.20,
            retrieval_utilization: 0.15,
            contradiction_safety: 0.15,
            user_friction: 0.15,
            explanation_quality: 0.10,
        }
    }
}

impl QualityVectorWeights {
    #[must_use]
    pub(crate) fn normalised(&self) -> [f32; 6] {
        let raw = [
            self.task_completion,
            self.tool_effectiveness,
            self.retrieval_utilization,
            self.contradiction_safety,
            self.user_friction,
            self.explanation_quality,
        ];
        let sum: f32 = raw.iter().sum();
        if sum <= f32::EPSILON {
            return [1.0 / 6.0; 6];
        }
        let inv = 1.0 / sum;
        [
            raw[0] * inv,
            raw[1] * inv,
            raw[2] * inv,
            raw[3] * inv,
            raw[4] * inv,
            raw[5] * inv,
        ]
    }
}

/// Multi-dimensional quality assessment for a single turn.
///
/// Each component is a bounded score in `[0.0, 1.0]`.
/// The `composite_score` is the weighted combination used as the
/// primary reward signal when v2 is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub(crate) struct TurnQualityVector {
    /// How well the assistant addressed the user's request.
    pub task_completion_score: f32,
    /// Effectiveness of tool usage (success rate, relevance).
    pub tool_effectiveness_score: f32,
    /// How well retrieved memory items were utilised in the response.
    pub retrieval_utilization_score: f32,
    /// Absence of contradictions and safety violations.
    pub contradiction_safety_score: f32,
    /// Inverse of user friction (corrections, repeated requests).
    pub user_friction_score: f32,
    /// Structural quality of the explanation (coherence, length fit).
    pub explanation_quality_score: f32,
    /// Weighted composite of all dimensions.
    pub composite_score: f32,
}

impl Default for TurnQualityVector {
    fn default() -> Self {
        Self {
            task_completion_score: 0.5,
            tool_effectiveness_score: 0.5,
            retrieval_utilization_score: 0.5,
            contradiction_safety_score: 1.0,
            user_friction_score: 0.5,
            explanation_quality_score: 0.5,
            composite_score: 0.5,
        }
    }
}
