//! Policy selector: chooses reasoning strategy and memory policy
//! from past outcome patterns and distilled principles.

#![allow(clippy::cast_precision_loss)]

use super::outcome_record::TurnOutcomeRecord;
use super::policy::{MemoryPolicy, PolicyDecision, ReasoningStrategy, SituationFeatures};
use crate::core::experience::distill_types::Principle;

/// Minimum number of outcome records needed before learning overrides defaults.
const MIN_DATA_POINTS: usize = 5;

/// Select a policy based on past outcome patterns and distilled principles.
///
/// Falls back to `PolicyDecision::default()` when insufficient data (<5 records).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn select_policy(
    situation: &SituationFeatures,
    outcomes: &[TurnOutcomeRecord],
    principles: &[Principle],
) -> PolicyDecision {
    if outcomes.len() < MIN_DATA_POINTS {
        return PolicyDecision::default();
    }

    let reasoning = select_reasoning_strategy(situation, outcomes, principles);
    let memory = if outcomes.len() < MIN_DATA_POINTS {
        MemoryPolicy::default()
    } else {
        select_memory_policy(situation, outcomes)
    };

    PolicyDecision { reasoning, memory }
}

/// Select the reasoning strategy for the current turn via a two-phase cascade.
///
/// **Phase 1 — Principle override**:
///   Scan `Strategy`-category principles with confidence > 0.7.  Keyword match
///   on "stepwise"/"step-by-step" → `Stepwise`; "verify"/"check first" → `VerifyFirst`.
///
/// **Phase 2 — Outcome heuristics**:
///   - Domain avg success < 0.4 → `VerifyFirst` (be cautious).
///   - Complexity > 0.6 AND avg success < 0.7 → `Stepwise` (scaffold the user).
///
/// Falls back to `Standard` when no phase produces a strong signal.
fn select_reasoning_strategy(
    situation: &SituationFeatures,
    outcomes: &[TurnOutcomeRecord],
    principles: &[Principle],
) -> ReasoningStrategy {
    // Phase 1: High-confidence principle override (keyword heuristic).
    for principle in principles {
        if principle.category == crate::core::experience::distill_types::PrincipleCategory::Strategy
            && principle.confidence > crate::contracts::scores::Confidence::new(0.7)
        {
            let lower = principle.statement.to_lowercase();
            if lower.contains("stepwise") || lower.contains("step-by-step") {
                return ReasoningStrategy::Stepwise;
            }
            if lower.contains("verify") || lower.contains("check first") {
                return ReasoningStrategy::VerifyFirst;
            }
        }
    }

    // Phase 2: Domain-filtered outcome analysis — single-pass fold to avoid
    // allocating an intermediate `Vec<&TurnOutcomeRecord>`.
    let mut sum_success: f32 = 0.0;
    let mut domain_count: usize = 0;
    for o in outcomes {
        if o.situation.domain == situation.domain {
            sum_success += o.outcome.success.value();
            domain_count += 1;
        }
    }

    if domain_count >= MIN_DATA_POINTS {
        let avg_success = sum_success / domain_count as f32;

        // Low success in this domain → be more careful.
        if avg_success < 0.4 {
            return ReasoningStrategy::VerifyFirst;
        }

        // High complexity + moderate success → stepwise.
        if situation.complexity > 0.6 && avg_success < 0.7 {
            return ReasoningStrategy::Stepwise;
        }
    }

    // Fallback to standard.
    ReasoningStrategy::Standard
}

/// Select memory retrieval parameters from outcome history and situation features.
///
/// Heuristics (applied additively):
/// - Domain-filtered tool-heavy outcomes with avg success < 0.5 → `retrieve_top_k = 15`.
/// - Complexity > 0.7 → `retrieve_top_k ≥ 12`.
fn select_memory_policy(
    situation: &SituationFeatures,
    outcomes: &[TurnOutcomeRecord],
) -> MemoryPolicy {
    let mut base = MemoryPolicy::default();

    // If past outcomes show tool calls correlate with needing more context,
    // increase top_k. Single-pass fold (no intermediate Vec<&T>).
    let mut tool_sum: f32 = 0.0;
    let mut tool_count: usize = 0;
    for o in outcomes {
        if o.outcome.had_tool_calls && o.situation.domain == situation.domain {
            tool_sum += o.outcome.success.value();
            tool_count += 1;
        }
    }

    if tool_count >= 3 {
        let avg_success = tool_sum / tool_count as f32;

        if avg_success < 0.5 {
            // Increase retrieval for tool-heavy, low-success domains.
            base.retrieve_top_k = 15;
        }
    }

    // High complexity → more retrieval.
    if situation.complexity > 0.7 {
        base.retrieve_top_k = base.retrieve_top_k.max(12);
    }

    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::affect::AffectLabel;
    use crate::core::agent::loop_::augment::policy::{
        DomainTag, OutcomeScore, SituationFeatures, TurnOutcome,
    };

    fn make_outcomes(domain: DomainTag, success: f32, count: usize) -> Vec<TurnOutcomeRecord> {
        (0..count)
            .map(|_| TurnOutcomeRecord {
                id: uuid::Uuid::new_v4().to_string(),
                situation: SituationFeatures {
                    domain,
                    complexity: 0.5,
                    affect_label: AffectLabel::Neutral,
                    affect_intensity: 0.3,
                },
                policy: PolicyDecision::default(),
                outcome: TurnOutcome {
                    success: OutcomeScore::new(success),
                    user_effort: OutcomeScore::new(0.3),
                    response_length: 100,
                    had_tool_calls: false,
                },
                occurred_at: String::new(),
                quality_vector: None,
                reason_trace: None,
            })
            .collect()
    }

    #[test]
    fn falls_back_to_default_on_insufficient_data() {
        let outcomes = make_outcomes(DomainTag::General, 0.8, 3);
        let policy = select_policy(&SituationFeatures::default(), &outcomes, &[]);
        assert_eq!(policy.reasoning, ReasoningStrategy::Standard);
    }

    #[test]
    fn selects_verify_first_on_low_success() {
        let outcomes = make_outcomes(DomainTag::Technical, 0.3, 6);
        let situation = SituationFeatures {
            domain: DomainTag::Technical,
            ..SituationFeatures::default()
        };
        let policy = select_policy(&situation, &outcomes, &[]);
        assert_eq!(policy.reasoning, ReasoningStrategy::VerifyFirst);
    }

    #[test]
    fn selects_stepwise_on_high_complexity_moderate_success() {
        let outcomes = make_outcomes(DomainTag::Technical, 0.55, 6);
        let situation = SituationFeatures {
            domain: DomainTag::Technical,
            complexity: 0.8,
            ..SituationFeatures::default()
        };
        let policy = select_policy(&situation, &outcomes, &[]);
        assert_eq!(policy.reasoning, ReasoningStrategy::Stepwise);
    }

    #[test]
    fn principle_can_override_strategy() {
        let outcomes = make_outcomes(DomainTag::General, 0.8, 6);
        let principles = vec![Principle {
            id: "p1".into(),
            category: crate::core::experience::distill_types::PrincipleCategory::Strategy,
            statement: "Always verify reasoning before responding".into(),
            confidence: crate::contracts::scores::Confidence::new(0.9),
            source_experience_ids: vec![],
            validation_count: 5,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }];
        let policy = select_policy(&SituationFeatures::default(), &outcomes, &principles);
        assert_eq!(policy.reasoning, ReasoningStrategy::VerifyFirst);
    }
}
