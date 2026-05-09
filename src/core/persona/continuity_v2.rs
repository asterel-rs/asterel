//! Extended continuity scoring (V2): augments the base drift score with
//! dialogue-act consistency and value consistency signals.
//!
//! Composite formula:
//! ```text
//! composite = base×0.5 + dialogue_act_consistency×0.25 + value_consistency×0.25
//! ```
//! - `base` is `1.0 - drift_score` from the base `DriftAssessment`.
//! - `dialogue_act_consistency` is the Jaccard similarity between the
//!   historical (first-half) and recent (second-half) windows of the
//!   `DialogueAct` sequence; returns `1.0` when fewer than two turns exist.
//! - `value_consistency` is the fraction of current principles that share
//!   a Jaccard word-token similarity ≥ 0.5 with at least one previous
//!   principle; returns `1.0` when both sets are empty, `0.0` if one is empty.
//!
//! The `DialogueAct` enum and `classify_dialogue_act` rule-based classifier
//! are also defined here and re-used by `behavior_selector` and `empathy_policy`.

#[cfg(test)]
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::drift_detector::DriftAssessment;
#[cfg(test)]
use crate::core::experience::distill_types::Principle;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(test)]
pub(crate) struct ContinuityScoreV2 {
    pub base_continuity: f64,
    pub dialogue_act_consistency: f64,
    pub value_consistency: f64,
    pub composite_score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DialogueAct {
    Inform,
    Question,
    Request,
    Confirm,
    Deny,
    Greet,
    Thank,
    Apologize,
    Clarify,
}

pub(crate) fn classify_dialogue_act(message: &str) -> DialogueAct {
    let lower = message.to_ascii_lowercase();
    let trimmed = lower.trim();

    if trimmed.ends_with('?') || trimmed.contains('?') {
        return DialogueAct::Question;
    }

    if contains_any(trimmed, &["thank", "thanks", "thx", "appreciate"]) {
        return DialogueAct::Thank;
    }

    if contains_any(trimmed, &["sorry", "apologize", "apologies"]) {
        return DialogueAct::Apologize;
    }

    if is_greeting(trimmed) {
        return DialogueAct::Greet;
    }

    if contains_any(trimmed, &["clarify", "what do you mean", "to be clear"]) {
        return DialogueAct::Clarify;
    }

    let first = first_token(trimmed);
    if is_imperative_start(first) {
        return DialogueAct::Request;
    }

    if is_confirm(trimmed) {
        return DialogueAct::Confirm;
    }

    if is_deny(trimmed) {
        return DialogueAct::Deny;
    }

    DialogueAct::Inform
}

#[cfg(test)]
pub(crate) fn compute_continuity_v2(
    base_drift: &DriftAssessment,
    dialogue_acts: &[DialogueAct],
    principles: &[Principle],
    previous_principles: &[Principle],
) -> ContinuityScoreV2 {
    let base_continuity = base_drift.continuity_score.clamp(0.0, 1.0);
    let dialogue_act_consistency = compute_dialogue_act_consistency(dialogue_acts);
    let value_consistency = compute_value_consistency(principles, previous_principles);
    let composite_score =
        (base_continuity * 0.5 + dialogue_act_consistency * 0.25 + value_consistency * 0.25)
            .clamp(0.0, 1.0);

    ContinuityScoreV2 {
        base_continuity,
        dialogue_act_consistency,
        value_consistency,
        composite_score,
    }
}

#[cfg(test)]
fn compute_dialogue_act_consistency(dialogue_acts: &[DialogueAct]) -> f64 {
    if dialogue_acts.len() < 2 {
        return 1.0;
    }

    let split_index = dialogue_acts.len() / 2;
    let historical = &dialogue_acts[..split_index.max(1)];
    let recent = &dialogue_acts[split_index.max(1)..];

    jaccard_dialogue_acts(historical, recent)
}

#[cfg(test)]
fn jaccard_dialogue_acts(lhs: &[DialogueAct], rhs: &[DialogueAct]) -> f64 {
    let left = lhs.iter().copied().collect::<BTreeSet<_>>();
    let right = rhs.iter().copied().collect::<BTreeSet<_>>();

    if left.is_empty() && right.is_empty() {
        return 1.0;
    }

    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count().max(1);
    let intersection_u32 = u32::try_from(intersection).unwrap_or(u32::MAX);
    let union_u32 = u32::try_from(union).unwrap_or(u32::MAX).max(1);
    f64::from(intersection_u32) / f64::from(union_u32)
}

#[cfg(test)]
fn compute_value_consistency(principles: &[Principle], previous_principles: &[Principle]) -> f64 {
    if principles.is_empty() && previous_principles.is_empty() {
        return 1.0;
    }
    if principles.is_empty() || previous_principles.is_empty() {
        return 0.0;
    }

    let mut overlap = 0_u32;
    for current in principles {
        let has_match = previous_principles
            .iter()
            .any(|previous| statement_similarity(&current.statement, &previous.statement) >= 0.5);
        if has_match {
            overlap = overlap.saturating_add(1);
        }
    }

    let denominator = u32::try_from(principles.len().max(previous_principles.len()).max(1))
        .unwrap_or(u32::MAX)
        .max(1);
    (f64::from(overlap) / f64::from(denominator)).clamp(0.0, 1.0)
}

#[cfg(test)]
fn statement_similarity(lhs: &str, rhs: &str) -> f64 {
    let left_tokens = tokenize_statement(lhs);
    let right_tokens = tokenize_statement(rhs);

    if left_tokens.is_empty() && right_tokens.is_empty() {
        return 1.0;
    }
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }

    let intersection = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.union(&right_tokens).count().max(1);
    let intersection_u32 = u32::try_from(intersection).unwrap_or(u32::MAX);
    let union_u32 = u32::try_from(union).unwrap_or(u32::MAX).max(1);
    f64::from(intersection_u32) / f64::from(union_u32)
}

#[cfg(test)]
fn tokenize_statement(statement: &str) -> BTreeSet<String> {
    statement
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn first_token(message: &str) -> &str {
    message.split_whitespace().next().map_or("", |token| {
        token.trim_matches(|ch: char| !ch.is_ascii_alphabetic())
    })
}

fn is_imperative_start(first_word: &str) -> bool {
    const IMPERATIVES: &[&str] = &[
        "do",
        "make",
        "fix",
        "run",
        "add",
        "remove",
        "delete",
        "update",
        "change",
        "set",
        "create",
        "build",
        "stop",
        "start",
        "show",
        "tell",
        "give",
        "help",
        "explain",
        "implement",
        "deploy",
        "install",
        "configure",
        "enable",
        "disable",
    ];
    IMPERATIVES.contains(&first_word)
}

fn is_confirm(message: &str) -> bool {
    const CONFIRM_PREFIXES: &[&str] = &["yes", "yep", "correct", "right", "exactly", "affirmative"];
    CONFIRM_PREFIXES
        .iter()
        .any(|prefix| message == *prefix || message.starts_with(&format!("{prefix} ")))
}

fn is_deny(message: &str) -> bool {
    const DENY_PREFIXES: &[&str] = &["no", "nope", "nah", "incorrect", "wrong"];
    DENY_PREFIXES
        .iter()
        .any(|prefix| message == *prefix || message.starts_with(&format!("{prefix} ")))
}

fn is_greeting(message: &str) -> bool {
    const GREETINGS: &[&str] = &["hi", "hello", "hey", "good morning", "good afternoon"];
    GREETINGS
        .iter()
        .any(|prefix| message == *prefix || message.starts_with(&format!("{prefix} ")))
}

#[cfg(test)]
mod tests {
    use super::{
        ContinuityScoreV2, DialogueAct, classify_dialogue_act, compute_continuity_v2,
        compute_value_consistency,
    };
    use crate::core::experience::distill_types::{Principle, PrincipleCategory};
    use crate::core::persona::drift_detector::DriftAssessment;

    fn make_principle(id: &str, statement: &str) -> Principle {
        Principle {
            id: id.to_string(),
            category: PrincipleCategory::Strategy,
            statement: statement.to_string(),
            confidence: 0.9.into(),
            source_experience_ids: vec![],
            validation_count: 1,
            created_at: "2026-03-02T00:00:00Z".to_string(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }
    }

    fn stable_drift() -> DriftAssessment {
        DriftAssessment {
            continuity_score: 1.0,
            drift_score: 0.0,
            stable_layer_changed: false,
            timestamp_regressed: false,
        }
    }

    #[test]
    fn classify_dialogue_act_question() {
        assert_eq!(classify_dialogue_act("What is X?"), DialogueAct::Question);
    }

    #[test]
    fn classify_dialogue_act_request() {
        assert_eq!(classify_dialogue_act("Fix this bug"), DialogueAct::Request);
    }

    #[test]
    fn classify_dialogue_act_thank() {
        assert_eq!(
            classify_dialogue_act("Thanks for helping"),
            DialogueAct::Thank
        );
    }

    #[test]
    fn continuity_score_perfect_consistency() {
        let principles = vec![make_principle("p1", "verify before apply")];
        let score: ContinuityScoreV2 = compute_continuity_v2(
            &stable_drift(),
            &[DialogueAct::Inform, DialogueAct::Inform],
            &principles,
            &principles,
        );

        assert!((score.composite_score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn continuity_score_degraded_on_act_change() {
        let principles = vec![make_principle("p1", "verify before apply")];
        let stable = compute_continuity_v2(
            &stable_drift(),
            &[DialogueAct::Inform, DialogueAct::Inform],
            &principles,
            &principles,
        );
        let changed = compute_continuity_v2(
            &stable_drift(),
            &[DialogueAct::Inform, DialogueAct::Request],
            &principles,
            &principles,
        );

        assert!(changed.composite_score < stable.composite_score);
    }

    #[test]
    fn value_consistency_with_shared_principles() {
        let current = vec![
            make_principle("p1", "verify before apply changes"),
            make_principle("p2", "ask clarifying question first"),
        ];
        let previous_shared = vec![
            make_principle("p3", "verify before applying changes"),
            make_principle("p4", "keep explanations concise"),
        ];
        let previous_none = vec![make_principle("p5", "avoid shell commands entirely")];

        let shared = compute_value_consistency(&current, &previous_shared);
        let none = compute_value_consistency(&current, &previous_none);
        assert!(shared > none);
    }

    #[test]
    fn dialogue_act_serde_round_trip() {
        let act = DialogueAct::Clarify;
        let serialized = serde_json::to_string(&act).expect("serialize dialogue act");
        let parsed: DialogueAct =
            serde_json::from_str(&serialized).expect("deserialize dialogue act");
        assert_eq!(parsed, act);
    }
}
