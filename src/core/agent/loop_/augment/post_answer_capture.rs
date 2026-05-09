//! Post-answer context assembly and update fan-out.
//!
//! This module is the entry point for all post-answer learning and memory
//! updates.  It owns two responsibilities:
//!
//! 1. **Context assembly** — `build_context` collects every signal needed by
//!    the four update pipelines into a single borrowing [`PostAnswerContext`].
//!    Assembly is parallelism-friendly: the three sub-tasks (`compute_policy_setup`,
//!    `assess_turn`, `collect_turn_diagnostics`) are independent and can be
//!    awaited sequentially without contention.
//!
//! 2. **Update fan-out** — `run_post_answer` calls the four update pipelines in
//!    dependency order:
//!
//!    | Step | Module | What it does |
//!    |------|--------|-------------|
//!    | 1 | [`super::memory_updates`] | Experience atoms, outcome records, error taxonomy |
//!    | 2 | [`super::distillation_updates`] | Experience-to-principle trigger |
//!    | 3 | [`super::persona_updates`] | Affect arc, Big Five, world model |
//!
//! ## Key types
//!
//! - [`PostAnswerContext`] — the fat borrowing context struct threaded through
//!   all update modules.
//! - [`PolicySetup`] — intermediate result of policy selection; kept private.
//! - [`TurnAssessment`] — quality vector and retrieval signal; kept private.
//! - [`TurnDiagnostics`] — variant provenance and external fitness signals; kept private.

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::policy::{TurnOutcome, TurnSignals, extract_situation, modulate_policy_for_affect};
use crate::config::PersonaConfig;
use crate::core::providers::response::TokenLogprob;

/// Borrowing context struct threaded through all four post-answer update pipelines.
///
/// Assembled once per turn by [`build_context`] and then passed by shared
/// reference to `run_memory_updates`, `run_distillation_updates`, and
/// `run_persona_updates`.
pub(super) struct PostAnswerContext<'a> {
    /// Memory backend for all reads and writes this turn.
    pub mem: &'a dyn crate::core::memory::Memory,
    /// Entity-scoped identity (e.g. `"person:<id>"`).
    pub entity_id: &'a str,
    /// Person-scoped identity used for user-facing memory (e.g. user knowledge).
    pub person_id: &'a str,
    /// Raw user message text for this turn.
    pub user_message: &'a str,
    /// Raw assistant response text for this turn.
    pub assistant_answer: &'a str,
    /// Persona feature flags (Big Five, affect decay, counterfactual, etc.).
    pub persona_config: &'a PersonaConfig,
    /// Structured outcome metrics computed from the turn signals.
    pub outcome: TurnOutcome,
    /// Situation features extracted from the user message and affect reading.
    pub situation: super::policy::SituationFeatures,
    /// Affect-modulated policy for this turn.
    pub policy: super::policy::PolicyDecision,
    /// Scalar success score in `[0, 1]` derived from quality vector or outcome.
    pub success_score: f32,
    /// Optional six-dimensional quality vector.
    pub quality_vector: Option<super::quality_vector::TurnQualityVector>,
    /// Classified error for the turn, if one was detected.
    pub classified_error: Option<crate::core::persona::error_taxonomy::ClassifiedError>,
    /// Single UTC clock reading used by post-turn updates.
    pub turn_started_at_utc: DateTime<Utc>,
}

/// Intermediate policy-selection result returned by `compute_policy_setup`.
struct PolicySetup {
    /// Affect-modulated policy that was actually applied this turn.
    policy: super::policy::PolicyDecision,
}

/// Intermediate quality assessment result returned by `assess_turn`.
struct TurnAssessment {
    /// Scalar success score in `[0, 1]`.
    success_score: f32,
    /// Optional six-dimensional quality vector.
    quality_vector: Option<super::quality_vector::TurnQualityVector>,
}

/// Intermediate diagnostics result returned by `collect_turn_diagnostics`.
struct TurnDiagnostics {
    /// Error taxonomy classification for this turn, if any.
    classified_error: Option<crate::core::persona::error_taxonomy::ClassifiedError>,
}

/// Entry point for all post-answer processing.
///
/// 1. Detects the affect reading.
/// 2. Assembles the full [`PostAnswerContext`] via `build_context`.
/// 3. Fans out to memory, distillation, and persona update pipelines.
///
/// Returns `None`; post-answer explainability is no longer surfaced.
pub(super) async fn run_post_answer(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    person_id: &str,
    user_message: &str,
    assistant_answer: &str,
    persona_config: &PersonaConfig,
    logprobs: Option<&[TokenLogprob]>,
) -> Result<Option<crate::contracts::policy::ReasonTrace>> {
    let reading = crate::core::affect::hybrid_detect(user_message, None)
        .await
        .final_reading;
    let ctx = build_context(
        mem,
        entity_id,
        person_id,
        user_message,
        assistant_answer,
        persona_config,
        logprobs,
        &reading,
    )
    .await;

    super::memory_updates::run_memory_updates(&ctx).await;
    super::distillation_updates::run_distillation_updates(&ctx).await;
    super::persona_updates::run_persona_updates(&ctx, &reading).await;

    Ok(None)
}

#[allow(clippy::too_many_arguments)] // Context assembly requires all turn-level params
async fn build_context<'a>(
    mem: &'a dyn crate::core::memory::Memory,
    entity_id: &'a str,
    person_id: &'a str,
    user_message: &'a str,
    assistant_answer: &'a str,
    persona_config: &'a PersonaConfig,
    _logprobs: Option<&'a [TokenLogprob]>,
    reading: &crate::core::affect::AffectReading,
) -> PostAnswerContext<'a> {
    let outcome = TurnOutcome::from_turn_signals(&TurnSignals {
        user_message,
        assistant_answer,
        tool_calls: &[],
    });
    let situation = extract_situation(user_message, reading.label, reading.confidence.get());
    let policy_setup =
        compute_policy_setup(mem, entity_id, user_message, reading, &situation).await;
    let assessment = assess_turn(mem, entity_id, user_message, assistant_answer, &outcome).await;
    let diagnostics = collect_turn_diagnostics(
        user_message,
        assistant_answer,
        &outcome,
        assessment.success_score,
    );

    PostAnswerContext {
        mem,
        entity_id,
        person_id,
        user_message,
        assistant_answer,
        persona_config,
        outcome,
        situation,
        policy: policy_setup.policy,
        success_score: assessment.success_score,
        quality_vector: assessment.quality_vector,
        classified_error: diagnostics.classified_error,
        turn_started_at_utc: Utc::now(),
    }
}

async fn compute_policy_setup(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    user_message: &str,
    reading: &crate::core::affect::AffectReading,
    situation: &super::policy::SituationFeatures,
) -> PolicySetup {
    let outcomes = super::outcome_record::retrieve_recent_outcomes(mem, entity_id, 50)
        .await
        .unwrap_or_default();
    let principles_for_policy =
        crate::core::experience::principle_retrieve::retrieve_relevant_principles(
            mem,
            entity_id,
            user_message,
        )
        .await
        .unwrap_or_default();
    let base_policy =
        super::policy_selector::select_policy(situation, &outcomes, &principles_for_policy);
    // Cast safety: detector confidence is normalized to [0.0, 1.0] before f32 conversion.
    #[allow(clippy::cast_possible_truncation)]
    let policy =
        modulate_policy_for_affect(&base_policy, reading.label, reading.confidence.get() as f32);
    PolicySetup { policy }
}

async fn assess_turn(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    user_message: &str,
    assistant_answer: &str,
    outcome: &TurnOutcome,
) -> TurnAssessment {
    let retrieval_signal =
        maybe_assess_retrieval_quality(mem, entity_id, user_message, assistant_answer).await;

    // Fold citation verification into the retrieval signal.
    let retrieval_signal = retrieval_signal.map(|mut signal| {
        let citation = super::retrieval_quality::verify_citations(
            assistant_answer,
            signal.items_retrieved_count,
        );
        tracing::debug!(
            citations_found = citation.citations_found,
            items_available = citation.items_available,
            any_used = citation.any_citation_used,
            reward_modifier = citation.reward_modifier,
            "post-answer citation verification"
        );
        signal.quality_score = (signal.quality_score + citation.reward_modifier).clamp(0.0, 1.0);
        signal
    });

    let (success_score, quality_vector) = compute_quality_vector(
        user_message,
        assistant_answer,
        outcome,
        retrieval_signal.as_ref(),
    );

    TurnAssessment {
        success_score,
        quality_vector,
    }
}

async fn maybe_assess_retrieval_quality(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    user_message: &str,
    assistant_answer: &str,
) -> Option<super::retrieval_quality::RetrievalQualitySignal> {
    let query = crate::core::memory::RecallQuery::new(entity_id, user_message, 10);
    let recall_items = mem.recall_scoped(query).await.unwrap_or_default();
    Some(super::retrieval_quality::assess_retrieval_quality(
        &recall_items,
        assistant_answer,
        user_message,
    ))
}

fn compute_quality_vector(
    user_message: &str,
    assistant_answer: &str,
    outcome: &TurnOutcome,
    retrieval_signal: Option<&super::retrieval_quality::RetrievalQualitySignal>,
) -> (f32, Option<super::quality_vector::TurnQualityVector>) {
    let inputs = super::quality_vector::QualityInputs {
        user_message,
        assistant_answer,
        tool_stats: None,
        retrieval_utilization: retrieval_signal.map(|signal| signal.utilization_ratio),
    };
    let qv = super::quality_vector::TurnQualityVector::compute(
        &inputs,
        &crate::contracts::quality::QualityVectorWeights::default(),
    );
    let success_score = qv.as_outcome_score().value();
    if success_score.is_finite() {
        (success_score, Some(qv))
    } else {
        (outcome.success.value(), None)
    }
}

fn collect_turn_diagnostics(
    user_message: &str,
    assistant_answer: &str,
    outcome: &TurnOutcome,
    success_score: f32,
) -> TurnDiagnostics {
    let classified_error = crate::core::persona::error_taxonomy::classify_turn_error(
        &turn_error_signals(user_message, assistant_answer, outcome, success_score),
    );

    TurnDiagnostics { classified_error }
}

fn turn_error_signals<'a>(
    user_message: &'a str,
    assistant_answer: &'a str,
    outcome: &'a TurnOutcome,
    success_score: f32,
) -> crate::core::persona::error_taxonomy::TurnSignals<'a> {
    let success_score_f64 = f64::from(success_score);

    crate::core::persona::error_taxonomy::TurnSignals {
        user_message,
        assistant_answer,
        tool_success_rate: None,
        response_too_short: assistant_answer.len() < 20 && user_message.len() > 30,
        success_score: success_score_f64,
        had_tool_failures: outcome.had_tool_calls && success_score_f64 < 0.5,
    }
}
