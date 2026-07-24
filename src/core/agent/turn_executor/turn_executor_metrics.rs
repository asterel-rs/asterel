use anyhow::{Result, bail};

use super::{
    AffectReading, Arc, CompanionTurnContract, EntityId, LoopStopReason, Observer, ObserverEvent,
    PersonId, PostTurnInput, PreTurnInput, SessionId, ToolLoopResult, TurnPostExecutionSeed,
    WorkingMemoryView, affect_intensity, flush_working_memory, run_post_turn_hooks,
};

pub(super) fn emit_turn_evidence_trace(
    pre_turn: &PreTurnInput<'_>,
    contract: &CompanionTurnContract,
    observer: &dyn Observer,
) {
    for rail in &contract.policy_rails.rails {
        tracing::info!(
            target: "core::agent::turn_trace",
            entity_id = %pre_turn.entity_id,
            person_id = %pre_turn.person_id,
            session_id = ?pre_turn.session_id,
            phase = %rail.phase.as_str(),
            enforcement = %rail.enforcement.as_str(),
            reason_code = %rail.reason_code,
            "companion.turn.policy_rail"
        );
        observer.record_event(&ObserverEvent::CompanionPolicyRail {
            entity_id: EntityId::new(pre_turn.entity_id),
            person_id: PersonId::new(pre_turn.person_id),
            session_id: pre_turn.session_id.map(SessionId::new),
            phase: rail.phase.as_str().to_string(),
            enforcement: rail.enforcement.as_str().to_string(),
            reason_code: rail.reason_code.to_string(),
        });
    }

    for record in &contract.evidence.records {
        tracing::info!(
            target: "core::agent::turn_trace",
            entity_id = %pre_turn.entity_id,
            person_id = %pre_turn.person_id,
            session_id = ?pre_turn.session_id,
            phase = %record.phase.as_str(),
            decision = %record.decision.as_str(),
            reason_code = %record.reason_code,
            provenance = %record.provenance.as_str(),
            summary = %record.summary,
            "companion.turn.evidence"
        );
        observer.record_event(&ObserverEvent::CompanionTurnEvidence {
            entity_id: EntityId::new(pre_turn.entity_id),
            person_id: PersonId::new(pre_turn.person_id),
            session_id: pre_turn.session_id.map(SessionId::new),
            phase: record.phase.as_str().to_string(),
            decision: record.decision.as_str().to_string(),
            reason_code: record.reason_code.to_string(),
            provenance: record.provenance.as_str().to_string(),
            summary: record.summary.clone(),
        });
    }
}

pub(super) fn emit_turn_output_trace(
    pre_turn: &PreTurnInput<'_>,
    result: &ToolLoopResult,
    observer: &dyn Observer,
) {
    let success = matches!(result.stop_reason, LoopStopReason::Completed)
        || (matches!(result.stop_reason, LoopStopReason::MaxIterations)
            && !result.final_text.trim().is_empty());
    let (decision, reason_code, summary) = if success {
        (
            "allow",
            "turn_output_available",
            "turn produced a user-facing response",
        )
    } else {
        (
            "deny",
            "turn_output_unavailable",
            "turn stopped without a user-facing response",
        )
    };

    tracing::info!(
        target: "core::agent::turn_trace",
        entity_id = %pre_turn.entity_id,
        person_id = %pre_turn.person_id,
        session_id = ?pre_turn.session_id,
        phase = "output",
        decision = %decision,
        reason_code = %reason_code,
        provenance = "turn_executor",
        summary = %summary,
        "companion.turn.evidence"
    );
    observer.record_event(&ObserverEvent::CompanionTurnEvidence {
        entity_id: EntityId::new(pre_turn.entity_id),
        person_id: PersonId::new(pre_turn.person_id),
        session_id: pre_turn.session_id.map(SessionId::new),
        phase: "output".to_string(),
        decision: decision.to_string(),
        reason_code: reason_code.to_string(),
        provenance: "turn_executor".to_string(),
        summary: summary.to_string(),
    });
}

/// Complete required post-turn persistence before delivery.
///
/// # Errors
/// Returns an error if relationship or memory persistence fails.
pub(super) async fn run_post_turn_processing(
    post_turn_seed: TurnPostExecutionSeed,
    affect: &AffectReading,
    result: &ToolLoopResult,
    working_memory: Option<WorkingMemoryView>,
) -> Result<()> {
    let flush_policy_context = post_turn_seed.tenant_id.as_deref().map_or_else(
        crate::security::policy::TenantPolicyContext::disabled,
        crate::security::policy::TenantPolicyContext::enabled,
    );
    let observer = Arc::clone(&post_turn_seed.observer);
    let post_turn_input = PostTurnInput {
        mem: Arc::clone(&post_turn_seed.mem),
        auto_save: post_turn_seed.auto_save,
        person_id: post_turn_seed.person_id,
        person_entity_id: post_turn_seed.person_entity_id,
        user_message: post_turn_seed.user_message,
        tenant_id: post_turn_seed.tenant_id,
        surface: post_turn_seed.surface,
        enable_self_amendment_candidates: post_turn_seed.enable_self_amendment_candidates,
        self_amendment_candidate_sink: post_turn_seed.self_amendment_candidate_sink,
        response: result.final_text.clone(),
        affect_label: affect.label,
        affect_intensity: affect_intensity(affect),
        is_success: matches!(result.stop_reason, LoopStopReason::Completed)
            || (matches!(result.stop_reason, LoopStopReason::MaxIterations)
                && !result.final_text.trim().is_empty()),
        contract: post_turn_seed.contract,
        observer: Arc::clone(&observer),
    };
    if !run_post_turn_hooks(&post_turn_input).await {
        bail!("post-turn relationship or autosave persistence failed");
    }
    if post_turn_seed.auto_save
        && let Some(mut working_memory) = working_memory
        && !flush_working_memory(
            post_turn_seed.mem.as_ref(),
            &mut working_memory,
            &flush_policy_context,
            Some(observer.as_ref()),
        )
        .await
    {
        bail!("post-turn working memory persistence failed");
    }
    Ok(())
}
