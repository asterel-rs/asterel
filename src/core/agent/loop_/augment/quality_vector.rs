//! Turn quality vector: multi-dimensional outcome scoring.
//!
//! Replaces the single-scalar success heuristic with a
//! six-dimensional quality vector. Each dimension captures a
//! distinct aspect of turn quality; the weighted composite drives
//! post-answer outcome analysis.

use num_traits::ToPrimitive;

use super::policy::OutcomeScore;
use crate::contracts::quality::QualityVectorWeights;
pub(crate) use crate::contracts::quality::TurnQualityVector;

// ── Configurable weights ────────────────────────────────────────

// ── Input signals ───────────────────────────────────────────────

/// Raw signals collected from a completed turn for quality vector computation.
pub(crate) struct QualityInputs<'a> {
    /// User message text.
    pub user_message: &'a str,
    /// Assistant response text.
    pub assistant_answer: &'a str,
    /// Whether tool calls were made, and their success count / total.
    pub tool_stats: Option<ToolStats>,
    /// Retrieval utilization ratio from `RetrievalQualitySignal`.
    /// `None` when retrieval utilization is unavailable.
    pub retrieval_utilization: Option<f64>,
}

/// Summarised tool call statistics.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolStats {
    pub total: usize,
    pub succeeded: usize,
}

// ── Computation ─────────────────────────────────────────────────

impl TurnQualityVector {
    /// Compute a quality vector from available turn signals.
    ///
    /// Each dimension is computed independently, then combined via
    /// weighted sum into `composite_score`.
    #[must_use]
    pub(crate) fn compute(inputs: &QualityInputs<'_>, weights: &QualityVectorWeights) -> Self {
        let task_completion = compute_task_completion(inputs);
        let tool_effectiveness = compute_tool_effectiveness(inputs);
        let retrieval_utilization = compute_retrieval_utilization(inputs);
        let contradiction_safety = compute_contradiction_safety(inputs);
        let user_friction = compute_user_friction(inputs);
        let explanation_quality = compute_explanation_quality(inputs);

        let scores = [
            task_completion,
            tool_effectiveness,
            retrieval_utilization,
            contradiction_safety,
            user_friction,
            explanation_quality,
        ];
        let w = weights.normalised();
        let composite: f32 = scores
            .iter()
            .zip(w.iter())
            .map(|(s, w)| s * w)
            .sum::<f32>()
            .clamp(0.0, 1.0);

        Self {
            task_completion_score: task_completion,
            tool_effectiveness_score: tool_effectiveness,
            retrieval_utilization_score: retrieval_utilization,
            contradiction_safety_score: contradiction_safety,
            user_friction_score: user_friction,
            explanation_quality_score: explanation_quality,
            composite_score: composite,
        }
    }

    /// Convert composite score to an `OutcomeScore` for backward-compatible
    /// downstream consumers.
    #[must_use]
    pub(crate) fn as_outcome_score(&self) -> OutcomeScore {
        OutcomeScore::new(self.composite_score)
    }

    /// Convert composite score to reward in `[-1.0, 1.0]`.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn to_reward(&self) -> f64 {
        f64::from(self.composite_score) * 2.0 - 1.0
    }
}

// ── Per-dimension scorers ───────────────────────────────────────

/// Task completion: response length relative to query complexity,
/// plus keyword coherence.
fn compute_task_completion(inputs: &QualityInputs<'_>) -> f32 {
    let response_len = inputs.assistant_answer.len();
    let query_len = inputs.user_message.len();

    // Length adequacy: penalise very short answers to substantive queries
    let length_score: f32 = if query_len > 30 && response_len < 20 {
        0.2
    } else if query_len > 100 && response_len < 50 {
        0.3
    } else if response_len > 10 {
        0.7
    } else {
        0.4
    };

    // Keyword coherence (Jaccard-like)
    let coherence = compute_keyword_coherence(inputs.user_message, inputs.assistant_answer);

    (length_score * 0.4 + coherence * 0.6).clamp(0.0, 1.0)
}

/// Tool effectiveness: fraction of tool calls that succeeded.
fn compute_tool_effectiveness(inputs: &QualityInputs<'_>) -> f32 {
    match inputs.tool_stats {
        Some(stats) if stats.total > 0 => {
            let rate = bounded_count_to_f32(stats.succeeded) / bounded_count_to_f32(stats.total);
            rate.clamp(0.0, 1.0)
        }
        Some(_) | None => 0.5, // no effective tool info — neutral
    }
}

/// Retrieval utilization: how many recalled items were actually used.
fn compute_retrieval_utilization(inputs: &QualityInputs<'_>) -> f32 {
    match inputs.retrieval_utilization {
        Some(ratio) => bounded_ratio_to_f32(ratio),
        None => 0.5, // no retrieval data — neutral
    }
}

/// Contradiction / safety: detect hedging, self-contradiction, or
/// safety-flagged content in the response.
fn compute_contradiction_safety(inputs: &QualityInputs<'_>) -> f32 {
    let lower = inputs.assistant_answer.to_lowercase();

    // Contradiction indicators (response contradicts itself)
    let contradiction_phrases = [
        "but actually",
        "i was wrong",
        "correction:",
        "let me correct",
        "that contradicts",
        "on the other hand, no",
    ];
    let has_contradiction = contradiction_phrases
        .iter()
        .any(|phrase| lower.contains(phrase));

    // Safety-risk phrases (response contains potentially unsafe content)
    let safety_phrases = [
        "i cannot verify",
        "this may be harmful",
        "proceed with caution",
        "this is not medical advice",
        "this is not legal advice",
    ];
    let has_safety_flag = safety_phrases.iter().any(|phrase| lower.contains(phrase));

    match (has_contradiction, has_safety_flag) {
        (true, true) => 0.2,
        (true, false) => 0.4,
        (false, true) => 0.6, // safety caveats are actually a positive signal
        (false, false) => 1.0,
    }
}

/// User friction: inverse of negative-feedback signals in the user message.
fn compute_user_friction(inputs: &QualityInputs<'_>) -> f32 {
    let lower = inputs.user_message.to_lowercase();

    let negative = [
        "no",
        "wrong",
        "try again",
        "incorrect",
        "not right",
        "redo",
        "that's not what",
        "i said",
        "already told you",
        "not helpful",
    ];
    let positive = [
        "thanks", "perfect", "great", "good", "awesome", "correct", "exactly", "nice",
    ];

    let neg_count = negative.iter().filter(|kw| lower.contains(**kw)).count();
    let pos_count = positive.iter().filter(|kw| lower.contains(**kw)).count();

    if neg_count > 0 && pos_count == 0 {
        let penalty = (bounded_count_to_f32(neg_count) * 0.2).min(0.6);
        (0.8 - penalty).clamp(0.0, 1.0)
    } else if pos_count > 0 && neg_count == 0 {
        0.9_f32.min(0.7 + bounded_count_to_f32(pos_count) * 0.1)
    } else {
        0.5 // neutral or mixed
    }
}

/// Explanation quality: structural coherence of the response.
fn compute_explanation_quality(inputs: &QualityInputs<'_>) -> f32 {
    let answer = inputs.assistant_answer;
    let len = answer.len();

    // Very short answers are low quality unless query is simple
    if len < 10 {
        return 0.2;
    }

    let has_structure = answer.contains('\n') || answer.contains("- ") || answer.contains("1.");
    let has_question = inputs.user_message.contains('?');
    let sentence_count = answer.matches(['.', '!', '?']).count();

    let base: f32 = if has_question && sentence_count >= 2 {
        0.7
    } else if sentence_count >= 1 {
        0.6
    } else {
        0.4
    };

    let structure_bonus: f32 = if has_structure { 0.15 } else { 0.0 };
    // Penalise excessively long responses slightly
    let length_penalty: f32 = if len > 5000 { 0.1 } else { 0.0 };

    (base + structure_bonus - length_penalty).clamp(0.0, 1.0)
}

/// Compute keyword coherence (Jaccard overlap) between user message
/// and assistant answer.
fn compute_keyword_coherence(user_message: &str, assistant_answer: &str) -> f32 {
    let user_lower = user_message.to_lowercase();
    let user_words: std::collections::HashSet<&str> = user_lower
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .collect();
    let assistant_lower = assistant_answer.to_lowercase();
    let assistant_words: std::collections::HashSet<&str> = assistant_lower
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .collect();

    if user_words.is_empty() || assistant_words.is_empty() {
        return 0.5;
    }

    let intersection = user_words.intersection(&assistant_words).count();
    let union = user_words.union(&assistant_words).count();
    if union == 0 {
        return 0.5;
    }

    let jaccard = bounded_count_to_f32(intersection) / bounded_count_to_f32(union);
    // Scale: 0 overlap → 0.3, full overlap → 1.0
    (0.3 + jaccard * 0.7).clamp(0.0, 1.0)
}

fn bounded_count_to_f32(value: usize) -> f32 {
    match u16::try_from(value) {
        Ok(value) => f32::from(value),
        Err(_) => f32::from(u16::MAX),
    }
}

fn bounded_ratio_to_f32(value: f64) -> f32 {
    let bounded = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    };
    bounded.to_f32().unwrap_or(0.5)
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_inputs() -> QualityInputs<'static> {
        QualityInputs {
            user_message: "Can you explain how memory retrieval works in detail?",
            assistant_answer: "Memory retrieval works by querying the vector store \
                with the user's message, ranking results by relevance, \
                and injecting the top-k items into the prompt context.",
            tool_stats: None,
            retrieval_utilization: None,
        }
    }

    #[test]
    fn default_weights_sum_to_one() {
        let weights = QualityVectorWeights::default();
        let sum = weights.task_completion
            + weights.tool_effectiveness
            + weights.retrieval_utilization
            + weights.contradiction_safety
            + weights.user_friction
            + weights.explanation_quality;
        assert!(
            (sum - 1.0).abs() < 1e-5,
            "default weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn normalised_weights_sum_to_one() {
        let weights = QualityVectorWeights {
            task_completion: 5.0,
            tool_effectiveness: 3.0,
            retrieval_utilization: 2.0,
            contradiction_safety: 1.0,
            user_friction: 1.0,
            explanation_quality: 1.0,
        };
        let norm = weights.normalised();
        let sum: f32 = norm.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-5,
            "normalised weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn zero_weights_normalise_to_uniform() {
        let weights = QualityVectorWeights {
            task_completion: 0.0,
            tool_effectiveness: 0.0,
            retrieval_utilization: 0.0,
            contradiction_safety: 0.0,
            user_friction: 0.0,
            explanation_quality: 0.0,
        };
        let norm = weights.normalised();
        for w in &norm {
            assert!(
                (w - 1.0 / 6.0).abs() < 1e-5,
                "zero weights should normalise to uniform"
            );
        }
    }

    #[test]
    fn composite_score_clamped_to_unit_range() {
        let inputs = default_inputs();
        let weights = QualityVectorWeights::default();
        let vector = TurnQualityVector::compute(&inputs, &weights);

        assert!(
            (0.0..=1.0).contains(&vector.composite_score),
            "composite should be in [0,1], got {}",
            vector.composite_score
        );
        for score in [
            vector.task_completion_score,
            vector.tool_effectiveness_score,
            vector.retrieval_utilization_score,
            vector.contradiction_safety_score,
            vector.user_friction_score,
            vector.explanation_quality_score,
        ] {
            assert!(
                (0.0..=1.0).contains(&score),
                "all dimensions should be in [0,1], got {score}"
            );
        }
    }

    #[test]
    fn tool_success_all_pass_high_score() {
        let inputs = QualityInputs {
            tool_stats: Some(ToolStats {
                total: 5,
                succeeded: 5,
            }),
            ..default_inputs()
        };
        let score = compute_tool_effectiveness(&inputs);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "all tools succeed → 1.0, got {score}"
        );
    }

    #[test]
    fn tool_success_all_fail_low_score() {
        let inputs = QualityInputs {
            tool_stats: Some(ToolStats {
                total: 3,
                succeeded: 0,
            }),
            ..default_inputs()
        };
        let score = compute_tool_effectiveness(&inputs);
        assert!(
            score.abs() < f32::EPSILON,
            "all tools fail → 0.0, got {score}"
        );
    }

    #[test]
    fn no_tools_neutral_score() {
        let inputs = default_inputs();
        let score = compute_tool_effectiveness(&inputs);
        assert!(
            (score - 0.5).abs() < f32::EPSILON,
            "no tools → 0.5, got {score}"
        );
    }

    #[test]
    fn negative_feedback_lowers_friction_score() {
        let inputs = QualityInputs {
            user_message: "No, that's wrong. Try again please.",
            ..default_inputs()
        };
        let score = compute_user_friction(&inputs);
        assert!(score < 0.5, "negative feedback → low score, got {score}");
    }

    #[test]
    fn positive_feedback_raises_friction_score() {
        let inputs = QualityInputs {
            user_message: "Thanks, that's perfect!",
            ..default_inputs()
        };
        let score = compute_user_friction(&inputs);
        assert!(score > 0.7, "positive feedback → high score, got {score}");
    }

    #[test]
    fn short_answer_long_query_low_completion() {
        let inputs = QualityInputs {
            user_message: "Explain the theory of relativity in detail, including \
                special and general relativity, with examples.",
            assistant_answer: "It's complex.",
            ..default_inputs()
        };
        let score = compute_task_completion(&inputs);
        assert!(
            score < 0.5,
            "short answer to long query → low score, got {score}"
        );
    }

    #[test]
    fn contradiction_lowers_safety_score() {
        let inputs = QualityInputs {
            assistant_answer: "The answer is 42. But actually, let me correct myself, it's 43.",
            ..default_inputs()
        };
        let score = compute_contradiction_safety(&inputs);
        assert!(score < 0.8, "contradiction → lowered score, got {score}");
    }

    #[test]
    fn clean_answer_full_safety_score() {
        let inputs = default_inputs();
        let score = compute_contradiction_safety(&inputs);
        assert!(
            (score - 1.0).abs() < f32::EPSILON,
            "clean answer → 1.0, got {score}"
        );
    }

    #[test]
    fn retrieval_full_utilization_high_score() {
        let inputs = QualityInputs {
            retrieval_utilization: Some(0.95),
            ..default_inputs()
        };
        let score = compute_retrieval_utilization(&inputs);
        assert!(score > 0.9, "high utilization → high score, got {score}");
    }

    #[test]
    fn retrieval_none_neutral() {
        let inputs = default_inputs();
        let score = compute_retrieval_utilization(&inputs);
        assert!(
            (score - 0.5).abs() < f32::EPSILON,
            "no retrieval data → 0.5, got {score}"
        );
    }

    #[test]
    fn to_reward_maps_correctly() {
        let vector = TurnQualityVector {
            composite_score: 1.0,
            ..Default::default()
        };
        assert!((vector.to_reward() - 1.0).abs() < f64::EPSILON);

        let vector = TurnQualityVector {
            composite_score: 0.0,
            ..Default::default()
        };
        assert!((vector.to_reward() - (-1.0)).abs() < f64::EPSILON);

        let vector = TurnQualityVector {
            composite_score: 0.5,
            ..Default::default()
        };
        assert!(vector.to_reward().abs() < f64::EPSILON);
    }

    #[test]
    fn as_outcome_score_preserves_composite() {
        let vector = TurnQualityVector {
            composite_score: 0.73,
            ..Default::default()
        };
        let score = vector.as_outcome_score();
        assert!(
            (score.value() - 0.73).abs() < 1e-5,
            "outcome score should match composite"
        );
    }

    #[test]
    fn serde_round_trip() {
        let inputs = QualityInputs {
            user_message: "hello",
            assistant_answer: "Hello! How can I help you today?",
            tool_stats: None,
            retrieval_utilization: Some(0.8),
        };
        let vector = TurnQualityVector::compute(&inputs, &QualityVectorWeights::default());
        let json = serde_json::to_string(&vector).unwrap();
        let back: TurnQualityVector = serde_json::from_str(&json).unwrap();
        assert!(
            (back.composite_score - vector.composite_score).abs() < 1e-5,
            "serde round-trip should preserve scores"
        );
    }

    #[test]
    fn structured_answer_gets_quality_bonus() {
        let plain = QualityInputs {
            assistant_answer: "The answer is that memory retrieval queries the store.",
            ..default_inputs()
        };
        let structured = QualityInputs {
            assistant_answer: "Memory retrieval works in three steps:\n\
                1. Query vectorisation\n\
                2. Similarity search\n\
                3. Top-k injection into context.",
            ..default_inputs()
        };
        let plain_score = compute_explanation_quality(&plain);
        let structured_score = compute_explanation_quality(&structured);
        assert!(
            structured_score > plain_score,
            "structured answer should score higher: {structured_score} > {plain_score}"
        );
    }
}
