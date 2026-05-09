//! Foundation types for the governed policy selection system
//! (ADR-0010). Phase 1 provides static defaults; later phases add
//! learned selection.

pub(crate) use crate::contracts::policy::{
    DomainTag, MemoryPolicy, OutcomeScore, PolicyDecision, ReasoningStrategy, SituationFeatures,
    TurnOutcome,
};

/// Signals collected from a completed turn for multi-signal outcome scoring.
pub(crate) struct TurnSignals<'a> {
    pub user_message: &'a str,
    pub assistant_answer: &'a str,
    pub tool_calls: &'a [crate::core::agent::tool_types::ToolCallRecord],
}

impl TurnOutcome {
    /// Build a richer outcome from multiple turn signals.
    ///
    /// Four weighted signals:
    /// - `length_signal` (0.20): response length heuristic
    /// - `tool_success_rate` (0.35): fraction of tool calls with success=true
    /// - `coherence` (0.25): keyword overlap between user/assistant (Jaccard)
    /// - `user_effort` (0.20): negative/positive feedback keyword detection
    #[must_use]
    pub(crate) fn from_turn_signals(signals: &TurnSignals<'_>) -> Self {
        let response_length = signals.assistant_answer.len();

        // 1. Length signal
        let length_signal: f32 = if response_length < 20 && signals.user_message.len() > 30 {
            0.3
        } else {
            0.7
        };

        // 2. Tool success rate
        let tool_success_rate: f32 = if signals.tool_calls.is_empty() {
            0.7 // neutral when no tool calls
        } else {
            let successes = signals
                .tool_calls
                .iter()
                .filter(|tc| tc.result.success)
                .count();
            // Cast safety: tool call counts are bounded by per-turn tool executions.
            #[allow(clippy::cast_precision_loss)]
            {
                successes as f32 / signals.tool_calls.len() as f32
            }
        };

        // 3. Coherence (keyword Jaccard overlap)
        let coherence: f32 =
            compute_keyword_coherence(signals.user_message, signals.assistant_answer);

        // 4. User effort detection
        let user_effort_score: f32 = compute_user_effort_signal(signals.user_message);

        let composite = length_signal * 0.20
            + tool_success_rate * 0.35
            + coherence * 0.25
            + (1.0 - user_effort_score) * 0.20;

        Self {
            success: OutcomeScore::new(composite),
            user_effort: OutcomeScore::new(user_effort_score),
            response_length,
            had_tool_calls: !signals.tool_calls.is_empty(),
        }
    }
}

/// Compute word-level Jaccard coherence between user message and assistant answer.
///
/// Both texts are lowercased and split on whitespace; words shorter than 3
/// characters are excluded as stop-word noise.  The raw Jaccard value is
/// rescaled to `[0.3, 1.0]` so that even zero-overlap turns are not penalised
/// to the full minimum: `scaled = 0.3 + jaccard × 0.7`.
fn compute_keyword_coherence(user_message: &str, assistant_answer: &str) -> f32 {
    // Pre-size the hash sets using an upper bound on word count to avoid the
    // zero-capacity grow cascade that was previously allocating through
    // 0 → 16 → 32 → 64 slots per call on a hot post-answer path.
    let user_word_upper = user_message.len() / 4;
    let assistant_word_upper = assistant_answer.len() / 4;
    let user_lower = user_message.to_lowercase();
    let mut user_words: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(user_word_upper);
    user_words.extend(user_lower.split_whitespace().filter(|w| w.len() > 2));
    let assistant_lower = assistant_answer.to_lowercase();
    let mut assistant_words: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(assistant_word_upper);
    assistant_words.extend(assistant_lower.split_whitespace().filter(|w| w.len() > 2));

    if user_words.is_empty() || assistant_words.is_empty() {
        return 0.5;
    }

    let intersection = user_words.intersection(&assistant_words).count();
    let union = user_words.union(&assistant_words).count();
    if union == 0 {
        return 0.5;
    }

    // Cast safety: set intersection/union counts are bounded by tokenized message lengths.
    #[allow(clippy::cast_precision_loss)]
    let jaccard = intersection as f32 / union as f32;
    // Scale: 0 overlap → 0.3, full overlap → 1.0
    (0.3 + jaccard * 0.7).clamp(0.0, 1.0)
}

/// Score user effort from negative/positive feedback keywords.
///
/// - 0.8 — correction/retry keywords found (high effort, user is unsatisfied).
/// - 0.1 — appreciation keywords found (low effort, user is satisfied).
/// - 0.5 — neutral (no signal detected).
///
/// The score is complemented before weighting: `(1 − user_effort) × 0.20`
/// so that high effort *lowers* the composite success score.
fn compute_user_effort_signal(user_message: &str) -> f32 {
    let lower = user_message.to_lowercase();
    let negative = ["no", "wrong", "try again", "incorrect", "not right", "redo"]
        .iter()
        .any(|kw| lower.contains(kw));
    let positive = ["thanks", "perfect", "great", "good", "awesome", "correct"]
        .iter()
        .any(|kw| lower.contains(kw));

    if negative {
        0.8 // high effort = user correcting
    } else if positive {
        0.1 // low effort = user satisfied
    } else {
        0.5 // neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_f32_eq(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn outcome_score_clamps_to_bounds() {
        assert_f32_eq(OutcomeScore::new(1.5).value(), 1.0);
        assert_f32_eq(OutcomeScore::new(-0.5).value(), 0.0);
        assert!((OutcomeScore::new(0.7).value() - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn default_policy_decision_has_standard_reasoning() {
        let policy = PolicyDecision::default();
        assert_eq!(policy.reasoning, ReasoningStrategy::Standard);
        assert_eq!(policy.memory.retrieve_top_k, 10);
    }

    #[test]
    fn default_situation_features_are_neutral() {
        let features = SituationFeatures::default();
        assert_eq!(features.domain, DomainTag::General);
        assert_eq!(
            features.affect_label,
            crate::core::affect::AffectLabel::Neutral
        );
        assert_f32_eq(features.complexity, 0.0);
    }

    #[test]
    fn turn_outcome_short_answer_low_success() {
        let signals = TurnSignals {
            user_message: "Explain the theory of relativity in detail",
            assistant_answer: "I can't.",
            tool_calls: &[],
        };
        let outcome = TurnOutcome::from_turn_signals(&signals);
        assert!(outcome.success.value() < 0.5);
    }

    #[test]
    fn turn_outcome_normal_answer() {
        let signals = TurnSignals {
            user_message: "hello",
            assistant_answer: "Hello! How can I help you today?",
            tool_calls: &[],
        };
        let outcome = TurnOutcome::from_turn_signals(&signals);
        assert!(outcome.success.value() >= 0.5);
    }

    #[test]
    fn from_turn_signals_all_tools_succeed_high_score() {
        use crate::core::agent::tool_types::ToolCallRecord;
        use crate::core::tools::traits::ToolResult;
        let tool_calls = vec![
            ToolCallRecord {
                tool_name: "shell".to_string(),
                args: serde_json::json!({}),
                result: ToolResult {
                    success: true,
                    output: "ok".to_string(),
                    error: None,
                    attachments: vec![],
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                },
                iteration: 1,
            },
            ToolCallRecord {
                tool_name: "file_read".to_string(),
                args: serde_json::json!({}),
                result: ToolResult {
                    success: true,
                    output: "done".to_string(),
                    error: None,
                    attachments: vec![],
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                },
                iteration: 2,
            },
        ];
        let signals = TurnSignals {
            user_message: "Please fix the build error in the code",
            assistant_answer: "I fixed the build error in the code by updating the module imports.",
            tool_calls: &tool_calls,
        };
        let outcome = TurnOutcome::from_turn_signals(&signals);
        assert!(
            outcome.success.value() > 0.6,
            "all-success tools should produce high score, got {}",
            outcome.success.value()
        );
        assert!(outcome.had_tool_calls);
    }

    #[test]
    fn from_turn_signals_tools_fail_lower_score() {
        use crate::core::agent::tool_types::ToolCallRecord;
        use crate::core::tools::traits::ToolResult;
        let tool_calls = vec![ToolCallRecord {
            tool_name: "shell".to_string(),
            args: serde_json::json!({}),
            result: ToolResult {
                success: false,
                output: String::new(),
                error: Some("err".to_string()),
                attachments: vec![],
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            },
            iteration: 1,
        }];
        let signals = TurnSignals {
            user_message: "Run the tests please",
            assistant_answer: "The tests failed with an error.",
            tool_calls: &tool_calls,
        };
        let outcome = TurnOutcome::from_turn_signals(&signals);
        assert!(
            outcome.success.value() < 0.6,
            "failed tools should lower score, got {}",
            outcome.success.value()
        );
    }

    #[test]
    fn from_turn_signals_user_negative_feedback_lowers_score() {
        let signals = TurnSignals {
            user_message: "No, that's wrong. Try again.",
            assistant_answer: "Let me correct my approach.",
            tool_calls: &[],
        };
        let outcome = TurnOutcome::from_turn_signals(&signals);
        assert!(
            outcome.user_effort.value() > 0.5,
            "negative feedback should signal high user effort"
        );
    }

    #[test]
    fn from_turn_signals_user_positive_feedback_high_score() {
        let signals = TurnSignals {
            user_message: "Thanks, that's perfect!",
            assistant_answer: "Glad I could help!",
            tool_calls: &[],
        };
        let outcome = TurnOutcome::from_turn_signals(&signals);
        assert!(
            outcome.user_effort.value() < 0.3,
            "positive feedback should signal low user effort"
        );
        assert!(
            outcome.success.value() > 0.5,
            "positive feedback should boost success"
        );
    }
}
