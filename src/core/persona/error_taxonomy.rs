//! Error taxonomy for agentic metacognition.
//!
//! Structured classification of agent failure modes into 5 modules
//! and 13 categories, enabling the metacognitive calibration loop
//! to reason about *why* a turn failed, not just *that* it failed.
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};

use crate::contracts::scores::Confidence;

/// Top-level module of the error taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorModule {
    /// Failures in retrieving or utilizing stored knowledge.
    Memory,
    /// Failures in self-assessment, confidence estimation, or introspection.
    Reflection,
    /// Failures in decomposing tasks, sequencing steps, or adapting plans.
    Planning,
    /// Failures in executing actions (tool calls, output generation).
    Action,
    /// External failures beyond the agent's control.
    System,
}

/// Specific error category within a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorCategory {
    // ── Memory module ───────────────────────────────────
    /// Relevant knowledge existed but wasn't retrieved.
    RetrievalFailure,
    /// Retrieved knowledge was outdated or incorrect.
    StaleKnowledge,
    /// Important context was lost between turns.
    ContextLoss,

    // ── Reflection module ───────────────────────────────
    /// Agent was overconfident in an incorrect answer.
    Overconfidence,
    /// Agent was underconfident and hedged unnecessarily.
    Underconfidence,
    /// Agent failed to recognize the limits of its knowledge.
    BlindSpot,

    // ── Planning module ─────────────────────────────────
    /// Task was decomposed incorrectly or incompletely.
    DecompositionError,
    /// Steps were executed in wrong order or with missing dependencies.
    SequencingError,
    /// Plan wasn't adapted when initial approach failed.
    AdaptationFailure,

    // ── Action module ───────────────────────────────────
    /// Tool was called with incorrect arguments.
    ToolMisuse,
    /// Generated output contained factual or logical errors.
    OutputError,

    // ── System module ───────────────────────────────────
    /// External service failure (API timeout, network error).
    ExternalFailure,
    /// Task was inherently impossible given current capabilities.
    CapabilityGap,
}

impl ErrorCategory {
    /// Get the module this category belongs to.
    #[must_use]
    pub(crate) const fn module(self) -> ErrorModule {
        match self {
            Self::RetrievalFailure | Self::StaleKnowledge | Self::ContextLoss => {
                ErrorModule::Memory
            }
            Self::Overconfidence | Self::Underconfidence | Self::BlindSpot => {
                ErrorModule::Reflection
            }
            Self::DecompositionError | Self::SequencingError | Self::AdaptationFailure => {
                ErrorModule::Planning
            }
            Self::ToolMisuse | Self::OutputError => ErrorModule::Action,
            Self::ExternalFailure | Self::CapabilityGap => ErrorModule::System,
        }
    }

    /// Whether this error type is potentially improvable through learning.
    #[must_use]
    pub(crate) const fn is_learnable(self) -> bool {
        !matches!(self, Self::ExternalFailure | Self::CapabilityGap)
    }
}

/// A classified error with confidence and evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ClassifiedError {
    /// Primary error category.
    pub category: ErrorCategory,
    /// Confidence in this classification (0.0–1.0).
    pub confidence: Confidence,
    /// Brief explanation of why this classification was chosen.
    pub reasoning: String,
    /// Contributing factors identified in the turn.
    pub factors: Vec<String>,
}

/// Signals from a turn used for error classification.
pub(crate) struct TurnSignals<'a> {
    /// The user's original message.
    pub user_message: &'a str,
    /// The assistant's response.
    pub assistant_answer: &'a str,
    /// Whether tool calls were made and their success rate.
    pub tool_success_rate: Option<f64>,
    /// Whether the response was very short relative to question complexity.
    pub response_too_short: bool,
    /// The heuristic success score for the turn.
    pub success_score: f64,
    /// Whether the turn had tool call failures.
    pub had_tool_failures: bool,
}

/// Classify the error type for a failed or partially-failed turn.
///
/// Returns `None` if the turn was successful (`success_score` >= 0.7).
/// Otherwise returns the most likely error classification.
pub(crate) fn classify_turn_error(signals: &TurnSignals<'_>) -> Option<ClassifiedError> {
    if signals.success_score >= 0.7 {
        return None;
    }

    let answer_lower = signals.assistant_answer.to_lowercase();
    let msg_lower = signals.user_message.to_lowercase();

    let keyword_coverage = compute_keyword_coverage(&msg_lower, &answer_lower);
    let mut candidates =
        collect_error_candidates(signals, &answer_lower, &msg_lower, keyword_coverage);

    // Select the highest-confidence classification.
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));

    candidates
        .into_iter()
        .next()
        .map(|(category, confidence, reasoning)| {
            let factors = collect_contributing_factors(signals, keyword_coverage);
            ClassifiedError {
                category,
                confidence: Confidence::new(confidence.clamp(0.1, 0.95)),
                reasoning,
                factors,
            }
        })
}

/// Compute how much of the user's question keywords appear in the answer.
fn compute_keyword_coverage(msg_lower: &str, answer_lower: &str) -> f64 {
    let question_keywords: Vec<&str> = msg_lower
        .split_whitespace()
        .filter(|w| w.len() > 4)
        .collect();
    if question_keywords.is_empty() {
        return 1.0;
    }
    let keyword_hits = question_keywords
        .iter()
        .filter(|w| answer_lower.contains(*w))
        .count();
    keyword_hits as f64 / question_keywords.len() as f64
}

/// Collect candidate error classifications from all modules.
fn collect_error_candidates(
    signals: &TurnSignals<'_>,
    answer_lower: &str,
    msg_lower: &str,
    keyword_coverage: f64,
) -> Vec<(ErrorCategory, f64, String)> {
    let mut candidates: Vec<(ErrorCategory, f64, String)> = Vec::new();

    // ── Action module checks ────────────────────────────────────
    if signals.had_tool_failures {
        let conf = if let Some(rate) = signals.tool_success_rate {
            (1.0 - rate).clamp(0.3, 0.9)
        } else {
            0.6
        };
        candidates.push((
            ErrorCategory::ToolMisuse,
            conf,
            "Tool calls failed during this turn".to_string(),
        ));
    }

    // ── Reflection module checks ────────────────────────────────
    let definitive_markers = [
        "definitely",
        "certainly",
        "absolutely",
        "clearly",
        "obviously",
    ];
    let definitive_count = definitive_markers
        .iter()
        .filter(|m| answer_lower.contains(*m))
        .count();
    if definitive_count >= 1 && signals.success_score < 0.4 {
        candidates.push((
            ErrorCategory::Overconfidence,
            0.3 + definitive_count as f64 * 0.15,
            format!("Used {definitive_count} definitive markers in failed turn"),
        ));
    }

    let hedge_markers = [
        "i'm not sure",
        "i think",
        "maybe",
        "perhaps",
        "might be",
        "could be",
    ];
    let hedge_count = hedge_markers
        .iter()
        .filter(|m| answer_lower.contains(*m))
        .count();
    if hedge_count >= 2 {
        candidates.push((
            ErrorCategory::Underconfidence,
            0.3 + hedge_count as f64 * 0.1,
            format!("Used {hedge_count} hedging markers"),
        ));
    }

    // ── Memory module checks ────────────────────────────────────
    if keyword_coverage < 0.2 && signals.success_score < 0.5 {
        candidates.push((
            ErrorCategory::ContextLoss,
            0.5,
            "Response doesn't address key topics from the question".to_string(),
        ));
    }

    // ── Planning module checks ──────────────────────────────────
    if signals.assistant_answer.len() > 500 && signals.success_score < 0.4 {
        candidates.push((
            ErrorCategory::AdaptationFailure,
            0.4,
            "Long response with low success suggests failed approach not adapted".to_string(),
        ));
    }
    if signals.response_too_short && msg_lower.len() > 100 {
        candidates.push((
            ErrorCategory::DecompositionError,
            0.45,
            "Very short response to complex question suggests incomplete task decomposition"
                .to_string(),
        ));
    }

    // ── System module checks ────────────────────────────────────
    let system_error_markers = ["timeout", "connection refused", "rate limit", "503", "502"];
    if system_error_markers
        .iter()
        .any(|m| answer_lower.contains(m))
    {
        candidates.push((
            ErrorCategory::ExternalFailure,
            0.7,
            "Response mentions system/infrastructure errors".to_string(),
        ));
    }

    candidates
}

/// Collect contributing factors for the final classification.
fn collect_contributing_factors(signals: &TurnSignals<'_>, keyword_coverage: f64) -> Vec<String> {
    let mut factors = Vec::new();
    if signals.had_tool_failures {
        factors.push("tool_failures".to_string());
    }
    if signals.response_too_short {
        factors.push("response_too_short".to_string());
    }
    if keyword_coverage < 0.3 {
        factors.push("low_topic_coverage".to_string());
    }
    factors
}

/// Aggregate error patterns across recent turns to identify systemic issues.
pub(crate) fn identify_error_patterns(errors: &[ClassifiedError]) -> Vec<ErrorPattern> {
    use std::collections::HashMap;

    if errors.len() < 3 {
        return Vec::new();
    }

    // Count by module and category.
    let mut module_counts: HashMap<ErrorModule, usize> = HashMap::new();
    let mut category_counts: HashMap<ErrorCategory, usize> = HashMap::new();

    for error in errors {
        *module_counts.entry(error.category.module()).or_insert(0) += 1;
        *category_counts.entry(error.category).or_insert(0) += 1;
    }

    let total = errors.len() as f64;
    let mut patterns = Vec::new();

    // Dominant module pattern (>40% of errors in one module).
    for (module, count) in &module_counts {
        let ratio = *count as f64 / total;
        if ratio >= 0.4 {
            patterns.push(ErrorPattern {
                pattern_type: PatternType::DominantModule(*module),
                frequency: ratio,
                recommendation: module_recommendation(*module),
            });
        }
    }

    // Recurring category (>30% in one category).
    for (category, count) in &category_counts {
        let ratio = *count as f64 / total;
        if ratio >= 0.3 {
            patterns.push(ErrorPattern {
                pattern_type: PatternType::RecurringCategory(*category),
                frequency: ratio,
                recommendation: category_recommendation(*category),
            });
        }
    }

    patterns
}

/// An identified pattern in error occurrences.
#[derive(Debug, Clone)]
pub(crate) struct ErrorPattern {
    pub pattern_type: PatternType,
    pub frequency: f64,
    pub recommendation: String,
}

/// Type of error pattern detected.
#[derive(Debug, Clone)]
pub(crate) enum PatternType {
    /// Most errors cluster in one module.
    DominantModule(ErrorModule),
    /// A specific category recurs frequently.
    RecurringCategory(ErrorCategory),
}

fn module_recommendation(module: ErrorModule) -> String {
    match module {
        ErrorModule::Memory => {
            "Memory-related errors dominate. Consider increasing retrieval depth \
             or improving context retention between turns."
                .to_string()
        }
        ErrorModule::Reflection => {
            "Self-assessment errors dominate. Calibrate confidence estimates \
             and use verify-first reasoning for uncertain domains."
                .to_string()
        }
        ErrorModule::Planning => "Planning errors dominate. Adopt stepwise decomposition and \
             verify intermediate results before proceeding."
            .to_string(),
        ErrorModule::Action => "Action execution errors dominate. Validate tool arguments and \
             check outputs before using them in subsequent steps."
            .to_string(),
        ErrorModule::System => "System errors dominate. These are external factors; consider \
             retry strategies and graceful degradation."
            .to_string(),
    }
}

fn category_recommendation(category: ErrorCategory) -> String {
    match category {
        ErrorCategory::Overconfidence => {
            "Pattern: overconfidence. Add uncertainty qualifiers when confidence is below 70%."
                .to_string()
        }
        ErrorCategory::ToolMisuse => {
            "Pattern: tool misuse. Verify tool arguments against documentation before calling."
                .to_string()
        }
        ErrorCategory::ContextLoss => {
            "Pattern: context loss. Explicitly reference earlier context when building responses."
                .to_string()
        }
        ErrorCategory::AdaptationFailure => {
            "Pattern: adaptation failure. When an approach isn't working, stop and try an \
             alternative rather than continuing the same strategy."
                .to_string()
        }
        _ => format!(
            "Recurring {category:?} errors. Review recent failures for corrective patterns."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failing_signals() -> TurnSignals<'static> {
        TurnSignals {
            user_message: "Explain the complex algorithm for graph traversal in detail",
            assistant_answer: "I'm not sure, maybe you could try something",
            tool_success_rate: None,
            response_too_short: true,
            success_score: 0.3,
            had_tool_failures: false,
        }
    }

    #[test]
    fn successful_turn_returns_none() {
        let signals = TurnSignals {
            success_score: 0.8,
            ..failing_signals()
        };
        assert!(classify_turn_error(&signals).is_none());
    }

    #[test]
    fn tool_failure_classified_as_tool_misuse() {
        let signals = TurnSignals {
            had_tool_failures: true,
            tool_success_rate: Some(0.2),
            success_score: 0.3,
            ..failing_signals()
        };
        let error = classify_turn_error(&signals).unwrap();
        assert_eq!(error.category, ErrorCategory::ToolMisuse);
        assert!(error.confidence > Confidence::new(0.5));
    }

    #[test]
    fn overconfident_language_detected() {
        let signals = TurnSignals {
            assistant_answer: "This is definitely and certainly the correct answer, obviously.",
            success_score: 0.2,
            response_too_short: false,
            ..failing_signals()
        };
        let error = classify_turn_error(&signals).unwrap();
        assert_eq!(error.category, ErrorCategory::Overconfidence);
    }

    #[test]
    fn hedging_language_detected() {
        let signals = TurnSignals {
            user_message: "What is 2+2?",
            assistant_answer: "I think maybe it could be 4, perhaps, I'm not sure though",
            success_score: 0.5,
            response_too_short: false,
            had_tool_failures: false,
            tool_success_rate: None,
        };
        let error = classify_turn_error(&signals).unwrap();
        assert_eq!(error.category, ErrorCategory::Underconfidence);
    }

    #[test]
    fn system_error_detected() {
        let signals = TurnSignals {
            assistant_answer: "I encountered a connection refused error when trying to fetch data",
            success_score: 0.2,
            response_too_short: false,
            ..failing_signals()
        };
        let error = classify_turn_error(&signals).unwrap();
        assert_eq!(error.category, ErrorCategory::ExternalFailure);
    }

    #[test]
    fn error_category_module_mapping() {
        assert_eq!(
            ErrorCategory::RetrievalFailure.module(),
            ErrorModule::Memory
        );
        assert_eq!(
            ErrorCategory::Overconfidence.module(),
            ErrorModule::Reflection
        );
        assert_eq!(
            ErrorCategory::DecompositionError.module(),
            ErrorModule::Planning
        );
        assert_eq!(ErrorCategory::ToolMisuse.module(), ErrorModule::Action);
        assert_eq!(ErrorCategory::ExternalFailure.module(), ErrorModule::System);
    }

    #[test]
    fn learnable_classification() {
        assert!(ErrorCategory::ToolMisuse.is_learnable());
        assert!(ErrorCategory::Overconfidence.is_learnable());
        assert!(!ErrorCategory::ExternalFailure.is_learnable());
        assert!(!ErrorCategory::CapabilityGap.is_learnable());
    }

    #[test]
    fn pattern_detection_requires_minimum_errors() {
        let errors = vec![ClassifiedError {
            category: ErrorCategory::ToolMisuse,
            confidence: Confidence::new(0.8),
            reasoning: "test".to_string(),
            factors: vec![],
        }];
        assert!(identify_error_patterns(&errors).is_empty());
    }

    #[test]
    fn dominant_module_pattern_detected() {
        let errors: Vec<ClassifiedError> = (0..5)
            .map(|_| ClassifiedError {
                category: ErrorCategory::Overconfidence,
                confidence: Confidence::new(0.7),
                reasoning: "test".to_string(),
                factors: vec![],
            })
            .collect();
        let patterns = identify_error_patterns(&errors);
        assert!(!patterns.is_empty());
        assert!(patterns.iter().any(|p| matches!(
            p.pattern_type,
            PatternType::DominantModule(ErrorModule::Reflection)
        )));
    }

    #[test]
    fn serde_round_trip() {
        let error = ClassifiedError {
            category: ErrorCategory::ToolMisuse,
            confidence: Confidence::new(0.75),
            reasoning: "test".to_string(),
            factors: vec!["tool_failures".to_string()],
        };
        let json = serde_json::to_string(&error).unwrap();
        let restored: ClassifiedError = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.category, ErrorCategory::ToolMisuse);
    }
}
