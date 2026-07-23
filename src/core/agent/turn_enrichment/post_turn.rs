use std::sync::Arc;

use crate::contracts::ids::{EntityId, PersonId};
use crate::contracts::memory::MemoryLayer;
use crate::contracts::observability::{Observer, ObserverMetric};
use crate::contracts::scores::Confidence;
use crate::contracts::strings::data_model::{
    SLOT_CONVERSATION_ASSISTANT_RESP, SLOT_CONVERSATION_USER_MSG,
};
use crate::core::affect::{AffectLabel, AffectReading};
use crate::core::agent::memory_excerpt::safe_memory_excerpt;
use crate::core::agent::turn_contract::CompanionTurnContract;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};
use crate::core::persona::relationship::update_relationship_after_turn_for_entity;
use crate::core::persona::soul_core::{
    SelfAmendmentCandidate, SelfAmendmentCandidateInput, SelfAmendmentCandidateSink,
    SoulIdentityCues, SoulPressureInput, SoulRecallExposure, SoulSurfaceExposure,
    derive_soul_pressure, generate_self_amendment_candidates,
};
use crate::core::persona::user_model::infer_user_model;
use crate::security::writeback_guard::enforce_agent_autosave_write_policy;

/// Inputs consumed by post-turn hooks.
pub struct PostTurnInput {
    /// Memory backend for persisting post-turn events.
    pub mem: Arc<dyn Memory>,
    /// Whether conversation-derived summaries should be persisted.
    pub auto_save: bool,
    /// Person ID for relationship and working-memory updates.
    pub person_id: PersonId,
    /// Canonical person entity ID for persona-memory writes.
    pub person_entity_id: EntityId,
    /// User message text (truncated before storage).
    pub user_message: String,
    /// Final assistant response text (truncated before storage).
    pub response: String,
    /// Affect label detected for this turn (used in relationship update).
    pub affect_label: AffectLabel,
    /// Affect confidence score in [0, 1] (used as intensity signal).
    pub affect_intensity: f32,
    /// Whether the turn completed successfully (affects relationship scoring).
    pub is_success: bool,
    /// Tenant scope attached to this transport turn, if tenant mode is active.
    pub tenant_id: Option<String>,
    /// Surface/channel where this post-turn hook is running.
    pub surface: Option<String>,
    /// Whether dry-run self-amendment candidate generation is enabled.
    pub enable_self_amendment_candidates: bool,
    /// Optional ephemeral review sink for dry-run self-amendment candidates.
    pub(crate) self_amendment_candidate_sink: Option<Arc<dyn SelfAmendmentCandidateSink>>,
    /// Contract built during pre-turn enrichment.
    pub contract: CompanionTurnContract,
    /// Runtime observer for post-turn hook success/failure counters.
    pub observer: Arc<dyn Observer>,
}

/// Execute relationship updates and contract-gated turn-summary writebacks.
///
/// In order:
/// 1. Update the relationship model for the person (trust, rapport, affect).
/// 2. Persist a compact user-message summary when the contract allows it.
/// 3. Persist a compact assistant-response summary when the contract allows it.
///
/// Returns `false` when a required relationship or autosave write fails.
pub async fn run_post_turn_hooks(input: &PostTurnInput) -> bool {
    let relationship_ok = update_relationship_for_post_turn(input).await;
    let user_summary_ok = !input.auto_save || persist_user_summary_for_post_turn(input).await;
    let assistant_summary_ok =
        !input.auto_save || persist_assistant_summary_for_post_turn(input).await;
    record_self_amendment_candidates_for_post_turn(input);
    relationship_ok && user_summary_ok && assistant_summary_ok
}

async fn update_relationship_for_post_turn(input: &PostTurnInput) -> bool {
    let outcome_success = if input.is_success { 0.85 } else { 0.3 };
    if let Err(error) = update_relationship_after_turn_for_entity(
        input.mem.as_ref(),
        input.person_entity_id.as_str(),
        input.person_id.as_str(),
        input.affect_label,
        input.affect_intensity,
        outcome_success,
    )
    .await
    {
        tracing::debug!(
            person_id = %input.person_id,
            %error,
            "post-turn relationship update skipped"
        );
        record_post_turn_hook(input.observer.as_ref(), "relationship_update", "failure");
        false
    } else {
        record_post_turn_hook(input.observer.as_ref(), "relationship_update", "success");
        true
    }
}

async fn persist_user_summary_for_post_turn(input: &PostTurnInput) -> bool {
    let user_summary = safe_memory_excerpt(&input.user_message, 100);
    if writeback_slot_allowed(&input.contract, SLOT_CONVERSATION_USER_MSG) {
        let user_event = MemoryEventInput::new(
            input.person_entity_id.as_str(),
            SLOT_CONVERSATION_USER_MSG,
            MemoryEventType::FactAdded,
            user_summary,
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Working)
        .with_confidence(0.9)
        .with_importance(0.5)
        .with_source_kind(SourceKind::Conversation)
        .with_source_ref("post_turn.user_summary")
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::ExplicitUser,
            "post_turn.user_summary",
        ));
        if let Err(error) = enforce_agent_autosave_write_policy(&user_event) {
            tracing::warn!(%error, "post-turn user message save rejected by write policy");
            record_post_turn_hook(input.observer.as_ref(), "autosave_user_summary", "rejected");
            return false;
        } else if let Err(error) = input.mem.append_event(user_event).await {
            tracing::debug!(%error, "post-turn user message save failed");
            record_post_turn_hook(input.observer.as_ref(), "autosave_user_summary", "failure");
            return false;
        } else {
            record_post_turn_hook(input.observer.as_ref(), "autosave_user_summary", "success");
        }
    } else {
        tracing::warn!(
            slot_key = SLOT_CONVERSATION_USER_MSG,
            "post-turn write skipped: slot not declared in companion writeback contract"
        );
        record_post_turn_hook(input.observer.as_ref(), "autosave_user_summary", "skipped");
    }
    true
}

async fn persist_assistant_summary_for_post_turn(input: &PostTurnInput) -> bool {
    let response_summary = safe_memory_excerpt(&input.response, 100);
    if writeback_slot_allowed(&input.contract, SLOT_CONVERSATION_ASSISTANT_RESP) {
        let response_event = MemoryEventInput::new(
            input.person_entity_id.as_str(),
            SLOT_CONVERSATION_ASSISTANT_RESP,
            MemoryEventType::FactAdded,
            response_summary,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Working)
        .with_confidence(0.85)
        .with_importance(0.4)
        .with_source_kind(SourceKind::Conversation)
        .with_source_ref("post_turn.assistant_summary")
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            "post_turn.assistant_summary",
        ));
        if let Err(error) = enforce_agent_autosave_write_policy(&response_event) {
            tracing::warn!(%error, "post-turn assistant response save rejected by write policy");
            record_post_turn_hook(
                input.observer.as_ref(),
                "autosave_assistant_summary",
                "rejected",
            );
            return false;
        } else if let Err(error) = input.mem.append_event(response_event).await {
            tracing::debug!(%error, "post-turn assistant response save failed");
            record_post_turn_hook(
                input.observer.as_ref(),
                "autosave_assistant_summary",
                "failure",
            );
            return false;
        } else {
            record_post_turn_hook(
                input.observer.as_ref(),
                "autosave_assistant_summary",
                "success",
            );
        }
    } else {
        tracing::warn!(
            slot_key = SLOT_CONVERSATION_ASSISTANT_RESP,
            "post-turn write skipped: slot not declared in companion writeback contract"
        );
        record_post_turn_hook(
            input.observer.as_ref(),
            "autosave_assistant_summary",
            "skipped",
        );
    }
    true
}

fn record_self_amendment_candidates_for_post_turn(input: &PostTurnInput) {
    let candidates = build_self_amendment_candidates_for_post_turn(input);
    if let Some(sink) = &input.self_amendment_candidate_sink {
        sink.record_self_amendment_candidates(&candidates);
        record_post_turn_hook(
            input.observer.as_ref(),
            "self_amendment_candidates",
            "success",
        );
    } else {
        record_post_turn_hook(
            input.observer.as_ref(),
            "self_amendment_candidates",
            "skipped",
        );
    }
    for candidate in &candidates {
        tracing::debug!(
            candidate_id = %candidate.candidate_id,
            person_id = %candidate.person_id,
            tenant_id = ?candidate.tenant_id,
            surface = %candidate.surface,
            evidence_ids = ?candidate.evidence_ids,
            "self-amendment candidate generated in dry-run mode"
        );
    }
}

fn record_post_turn_hook(observer: &dyn Observer, hook: &str, status: &str) {
    observer.record_metric(&ObserverMetric::PostTurnHook {
        hook: hook.to_string(),
        status: status.to_string(),
    });
}

#[must_use]
pub(crate) fn build_self_amendment_candidates_for_post_turn(
    input: &PostTurnInput,
) -> Vec<SelfAmendmentCandidate> {
    if !input.enable_self_amendment_candidates {
        return Vec::new();
    }

    let affect = AffectReading {
        label: input.affect_label,
        valence: 0.0,
        arousal: f64::from(input.affect_intensity),
        dominance: 0.5,
        confidence: Confidence::new(f64::from(input.affect_intensity.clamp(0.0, 1.0))),
    };
    let dialogue_act =
        crate::core::persona::continuity_v2::classify_dialogue_act(&input.user_message);
    let user_model = infer_user_model(&input.user_message, &affect, &[]);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: &input.user_message,
        identity: SoulIdentityCues::default(),
        affect: &affect,
        dialogue_act,
        user_model: &user_model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::Unknown,
    });
    let evidence_ids = post_turn_self_amendment_evidence_ids(input);
    let evidence_refs = evidence_ids.iter().map(String::as_str).collect::<Vec<_>>();
    generate_self_amendment_candidates(SelfAmendmentCandidateInput {
        user_message: &input.user_message,
        assistant_response: &input.response,
        soul_pressure: &pressure,
        tenant_id: input.tenant_id.as_deref(),
        person_id: input.person_id.as_str(),
        surface: input.surface.as_deref(),
        evidence_ids: &evidence_refs,
    })
}

fn post_turn_self_amendment_evidence_ids(input: &PostTurnInput) -> Vec<String> {
    let mut ids = vec![
        "post_turn:user_message".to_string(),
        "post_turn:assistant_response".to_string(),
        "post_turn:soul_pressure".to_string(),
    ];
    ids.extend(
        input
            .contract
            .evidence
            .records
            .iter()
            .map(|record| format!("turn_contract:{}", record.reason_code)),
    );
    ids
}

fn writeback_slot_allowed(contract: &CompanionTurnContract, slot_key: &str) -> bool {
    if contract.writeback_plan.slots.is_empty() {
        return true;
    }
    contract.writeback_plan.slots.iter().any(|candidate| {
        if let Some(prefix) = candidate.slot.strip_suffix('*') {
            slot_key.starts_with(prefix)
        } else {
            slot_key == candidate.slot
        }
    })
}
