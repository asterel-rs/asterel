//! Session persistence: auto-saves user/assistant messages,
//! conversation state, fact ledger, inference pass results, and
//! augmentation post-answer capture to memory.

use std::sync::Arc;

use super::augment::{self, TurnAugmentor as _};
use super::conversation_state::update_conversation_state_and_ledger;
use super::inference::run_post_turn_inference_pass;
use super::types::{RuntimeMemoryWriteContext, TurnPipelineContext};
use crate::config::Config;
use crate::contracts::memory::MemoryLayer;
use crate::contracts::observability::Observer;
use crate::contracts::strings::data_model::{
    ENTITY_PREFIX_PERSON, SLOT_CONVERSATION_ASSISTANT_RESP, SLOT_CONVERSATION_USER_MSG,
    SOURCE_REF_AGENT_AUTOSAVE_ASSISTANT_RESP, SOURCE_REF_AGENT_AUTOSAVE_USER_MSG,
};
use crate::core::agent::memory_excerpt::safe_memory_excerpt;
use crate::core::experience::{
    ExperienceAtom, ExperienceKind, ExperienceOutcome, persist_experience_atom,
};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind, schedule_durable_memory_consolidation,
};
use crate::security::writeback_guard::enforce_agent_autosave_write_policy;
use crate::utils::text::truncate_ellipsis;

type ReasonTrace = crate::contracts::policy::ReasonTrace;

/// Persist the user's message to memory when auto-save is enabled.
pub(super) async fn save_user_message_if_enabled(
    config: &Config,
    mem: &dyn Memory,
    write_context: &RuntimeMemoryWriteContext,
    user_message: &str,
) {
    if config.memory.auto_save {
        let user_summary = safe_memory_excerpt(user_message, 200);
        let input = MemoryEventInput::new(
            write_context.entity_id.as_str(),
            SLOT_CONVERSATION_USER_MSG,
            MemoryEventType::FactAdded,
            user_summary,
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Working)
        .with_confidence(0.95)
        .with_importance(0.6)
        .with_source_kind(SourceKind::Conversation)
        .with_source_ref(SOURCE_REF_AGENT_AUTOSAVE_USER_MSG)
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::ExplicitUser,
            SOURCE_REF_AGENT_AUTOSAVE_USER_MSG,
        ));
        if let Err(error) = enforce_agent_autosave_write_policy(&input) {
            tracing::warn!(%error, "agent autosave user message rejected by write policy");
        } else if let Err(error) = mem.append_event(input).await {
            tracing::warn!(
                %error,
                "agent autosave: failed to persist user message"
            );
        }
    }
}

async fn save_assistant_response_summary(
    mem: &dyn Memory,
    write_context: &RuntimeMemoryWriteContext,
    response: &str,
) {
    let summary = safe_memory_excerpt(response, 100);
    let input = MemoryEventInput::new(
        write_context.entity_id.as_str(),
        SLOT_CONVERSATION_ASSISTANT_RESP,
        MemoryEventType::FactAdded,
        summary,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Working)
    .with_confidence(0.9)
    .with_importance(0.4)
    .with_source_kind(SourceKind::Conversation)
    .with_source_ref(SOURCE_REF_AGENT_AUTOSAVE_ASSISTANT_RESP)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        SOURCE_REF_AGENT_AUTOSAVE_ASSISTANT_RESP,
    ));
    if let Err(error) = enforce_agent_autosave_write_policy(&input) {
        tracing::warn!(%error, "agent autosave assistant response rejected by write policy");
    } else if let Err(error) = mem.append_event(input).await {
        tracing::warn!(
            %error,
            "agent autosave: failed to persist assistant response"
        );
    }
}

async fn persist_turn_experience(
    mem: &dyn Memory,
    entity_id: &str,
    user_message: &str,
    response: &str,
) {
    let turn_atom = ExperienceAtom::new(
        ExperienceKind::TurnInteraction,
        format!("Companion turn: {}", truncate_ellipsis(user_message, 120)),
        ExperienceOutcome::Success,
    )
    .with_lesson(truncate_ellipsis(response, 180))
    .with_confidence(0.75);
    if let Err(error) = persist_experience_atom(mem, entity_id, &turn_atom).await {
        tracing::warn!(error = %error, "post-turn experience persistence failed");
    }
}

async fn run_post_turn_memory_inference(
    mem: &dyn Memory,
    write_context: &RuntimeMemoryWriteContext,
    response: &str,
    observer: &Arc<dyn Observer>,
) {
    if let Err(error) = run_post_turn_inference_pass(mem, write_context, response, observer).await {
        tracing::warn!(error = %error, "post-turn memory inference pass failed");
    }
}

async fn update_turn_conversation_state(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: Option<&str>,
    user_message: &str,
    response: &str,
) {
    if let Err(error) =
        update_conversation_state_and_ledger(mem, entity_id, person_id, user_message, response)
            .await
    {
        tracing::warn!(error = %error, "conversation state/ledger update failed");
    }
}

async fn schedule_durable_memory_checkpoint(
    config: &Config,
    mem: &Arc<dyn Memory>,
    write_context: &RuntimeMemoryWriteContext,
    user_message: &str,
    response: &str,
    observer: &Arc<dyn Observer>,
) {
    if let Err(error) = schedule_durable_memory_consolidation(
        Arc::clone(mem),
        config.workspace_dir.clone(),
        write_context.entity_id.as_str(),
        user_message,
        response,
        Arc::clone(observer),
    )
    .await
    {
        tracing::warn!(error = %error, "post-turn durable memory scheduling skipped");
    }
}

/// Save the assistant response, run post-turn inference and
/// augmentation capture, update conversation state/ledger, and
/// enqueue memory consolidation.
pub(super) async fn save_response_and_consolidate(
    ctx: &TurnPipelineContext<'_>,
    write_context: &RuntimeMemoryWriteContext,
    person_id: Option<&str>,
    user_message: &str,
    response: &str,
    logprobs: Option<&[crate::core::providers::response::TokenLogprob]>,
) -> Option<ReasonTrace> {
    if !ctx.config.memory.auto_save {
        return None;
    }

    let safe_user_message = safe_memory_excerpt(user_message, 240);
    let safe_response = safe_memory_excerpt(response, 360);

    save_assistant_response_summary(ctx.mem.as_ref(), write_context, &safe_response).await;
    persist_turn_experience(
        ctx.mem.as_ref(),
        write_context.entity_id.as_str(),
        &safe_user_message,
        &safe_response,
    )
    .await;
    run_post_turn_memory_inference(ctx.mem.as_ref(), write_context, response, ctx.observer).await;

    // ── Turn augmentation post-answer ─────────────────────────────────
    let augmentor = augment::DefaultAugmentor::new(
        ctx.config.workspace_dir.clone(),
        ctx.config.persona.clone(),
        ctx.params.augmentor_provider.clone(),
        ctx.params.model_name.to_string(),
        None, // post-answer path; bridge.evaluate() runs only during pre-answer
        Some(Arc::clone(ctx.observer)),
    );
    let effective_person_id = person_id.unwrap_or_else(|| {
        write_context
            .entity_id
            .as_str()
            .strip_prefix(ENTITY_PREFIX_PERSON)
            .unwrap_or("local-default")
    });
    let reason_trace = run_augmentor_post_answer(
        &augmentor,
        ctx.mem.as_ref(),
        write_context.entity_id.as_str(),
        effective_person_id,
        &safe_user_message,
        &safe_response,
        logprobs,
    )
    .await;
    update_turn_conversation_state(
        ctx.mem.as_ref(),
        write_context.entity_id.as_str(),
        person_id,
        &safe_user_message,
        &safe_response,
    )
    .await;
    schedule_durable_memory_checkpoint(
        ctx.config,
        &ctx.mem,
        write_context,
        &safe_user_message,
        &safe_response,
        ctx.observer,
    )
    .await;

    reason_trace
}

async fn run_augmentor_post_answer(
    augmentor: &augment::DefaultAugmentor,
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    user_message: &str,
    response: &str,
    logprobs: Option<&[crate::core::providers::response::TokenLogprob]>,
) -> Option<ReasonTrace> {
    match augmentor
        .post_answer(
            mem,
            entity_id,
            person_id,
            user_message,
            response,
            logprobs.map(Vec::from),
        )
        .await
    {
        Ok(trace) => trace,
        Err(error) => {
            tracing::warn!(%error, "turn augmentation post-answer failed");
            None
        }
    }
}
