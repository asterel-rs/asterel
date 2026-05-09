//! Write-policy enforcement for memory events.
//!
//! Validates provenance metadata, source classification, and
//! privacy levels for each memory write path (agent autosave,
//! inference, ingestion, persona, tool, conversation state).

use anyhow::Context as _;

use crate::contracts::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemorySource, PrivacyLevel, SourceKind,
};
use crate::contracts::strings::data_model::{
    ENTITY_PREFIX_PERSON, ENTITY_PREFIX_USER, PREFIX_EXTERNAL, PREFIX_PERSONA_WRITEBACK,
    PREFIX_USER_SLOT, RESERVED_SLOT_PREFIXES, SLOT_CONVERSATION_ASSISTANT_RESP,
    SLOT_CONVERSATION_LEDGER_V1, SLOT_CONVERSATION_STATE_V1, SLOT_CONVERSATION_USER_MSG,
    SLOT_VERIFY_REPAIR_ESCALATION,
};
use crate::contracts::strings::verdicts::{
    AGENT_AUTOSAVE_REJECTED_SLOT_KEY, AGENT_AUTOSAVE_REJECTED_SOURCE,
    AGENT_AUTOSAVE_REQUIRES_EVENT_TYPE_FACT_ADDED, AGENT_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE,
    AGENT_AUTOSAVE_REQUIRES_SOURCE_KIND_CONVERSATION, CONVERSATION_STATE_REJECTED_SLOT_KEY,
    CONVERSATION_STATE_REQUIRES_EVENT_TYPE_FACT_UPDATED,
    CONVERSATION_STATE_REQUIRES_PRIVACY_PRIVATE,
    CONVERSATION_STATE_REQUIRES_SOURCE_KIND_CONVERSATION,
    CONVERSATION_STATE_REQUIRES_SOURCE_SYSTEM, EXTERNAL_AUTOSAVE_REJECTED_SOURCE_KIND,
    EXTERNAL_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE, EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_EXPLICIT_USER,
    EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_KIND, INFERENCE_WB_REJECTED_EVENT_TYPE,
    INFERENCE_WB_REJECTED_SOURCE, INFERENCE_WB_REQUIRES_PRIVACY_PRIVATE,
    INFERENCE_WB_REQUIRES_SOURCE_KIND_CONVERSATION, INGESTION_REJECTED_SOURCE,
    INGESTION_REQUIRES_EVENT_TYPE_FACT_ADDED, INGESTION_REQUIRES_EXTERNAL_SLOT_KEY_PREFIX,
    INGESTION_REQUIRES_SOURCE_KIND, PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED,
    PERSONA_WB_ENTITY_ID_MISMATCH, PERSONA_WB_INFERRED_REQUIRES_INFERRED_CLAIM,
    PERSONA_WB_REJECTED_PROTECTED_SELF_EDIT, PERSONA_WB_REJECTED_SLOT_KEY,
    PERSONA_WB_RELATIONSHIP_REQUIRES_FACT_UPDATED, PERSONA_WB_REQUIRES_PRIVACY_PRIVATE,
    PERSONA_WB_REQUIRES_PROVENANCE_SOURCE_SYSTEM, PERSONA_WB_REQUIRES_SOURCE_KIND_MANUAL,
    PERSONA_WB_REQUIRES_SOURCE_SYSTEM, PERSONA_WB_STYLE_PROFILE_REQUIRES_FACT_UPDATED,
    PERSONA_WB_WORLD_MODEL_REQUIRES_FACT_UPDATED, PERSONA_WB_WRITEBACK_REQUIRES_SUMMARY_COMPACTED,
    TOOL_WB_REJECTS_PRIVACY_SECRET, TOOL_WB_REQUIRES_SOURCE_KIND_MANUAL,
    USER_INFERENCE_ENTITY_ID_MISMATCH, USER_INFERENCE_REJECTED_RESERVED_SLOT_KEY,
    USER_INFERENCE_REJECTED_SLOT_KEY_FORMAT, USER_INFERENCE_REQUIRES_EVENT_TYPE_INFERRED_CLAIM,
    USER_INFERENCE_REQUIRES_PRIVACY_PRIVATE, USER_INFERENCE_REQUIRES_SOURCE_KIND_MANUAL,
    USER_INFERENCE_REQUIRES_SOURCE_SYSTEM, USER_INFERENCE_REQUIRES_USER_PREFIX,
    VERIFY_REPAIR_REJECTED_SLOT_KEY, VERIFY_REPAIR_REQUIRES_EVENT_TYPE_SUMMARY_COMPACTED,
    VERIFY_REPAIR_REQUIRES_PRIVACY_PRIVATE, VERIFY_REPAIR_REQUIRES_SOURCE_KIND_MANUAL,
    VERIFY_REPAIR_REQUIRES_SOURCE_SYSTEM, WRITE_POLICY_REQUIRES_PROVENANCE,
    WRITE_POLICY_REQUIRES_PROVENANCE_REFERENCE,
    WRITE_POLICY_REQUIRES_PROVENANCE_SOURCE_CLASS_MATCH_SOURCE, WRITE_POLICY_REQUIRES_SOURCE_REF,
    WRITE_POLICY_SOURCE_REF_MUST_NOT_BE_EMPTY,
};
use crate::security::policy::TenantPolicyContext;

fn expected_person_entity(person_id: &str) -> String {
    format!("{ENTITY_PREFIX_PERSON}{person_id}")
}

fn require_common_write_metadata(event: &MemoryEventInput) -> anyhow::Result<()> {
    let Some(source_ref) = event.source_ref.as_deref() else {
        anyhow::bail!("{WRITE_POLICY_REQUIRES_SOURCE_REF}");
    };
    if source_ref.trim().is_empty() {
        anyhow::bail!("{WRITE_POLICY_SOURCE_REF_MUST_NOT_BE_EMPTY}");
    }

    let Some(provenance) = event.provenance.as_ref() else {
        anyhow::bail!("{WRITE_POLICY_REQUIRES_PROVENANCE}");
    };
    if provenance.source_class != event.source {
        anyhow::bail!("{WRITE_POLICY_REQUIRES_PROVENANCE_SOURCE_CLASS_MATCH_SOURCE}");
    }
    if provenance.reference.trim().is_empty() {
        anyhow::bail!("{WRITE_POLICY_REQUIRES_PROVENANCE_REFERENCE}");
    }

    Ok(())
}

/// # Errors
/// Returns an error when the event metadata violates persona writeback policy constraints.
pub fn enforce_persona_long_term_write_policy(
    event: &MemoryEventInput,
    person_id: &str,
) -> anyhow::Result<()> {
    if event.source != MemorySource::System {
        anyhow::bail!("{PERSONA_WB_REQUIRES_SOURCE_SYSTEM}");
    }

    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{PERSONA_WB_REQUIRES_PRIVACY_PRIVATE}");
    }

    if event.source_kind != Some(SourceKind::Manual) {
        anyhow::bail!("{PERSONA_WB_REQUIRES_SOURCE_KIND_MANUAL}");
    }

    require_common_write_metadata(event)?;
    let provenance = event
        .provenance
        .as_ref()
        .context("provenance missing after validation")?;
    if provenance.source_class != MemorySource::System {
        anyhow::bail!("{PERSONA_WB_REQUIRES_PROVENANCE_SOURCE_SYSTEM}");
    }

    if event.entity_id.as_str() != expected_person_entity(person_id) {
        anyhow::bail!("{PERSONA_WB_ENTITY_ID_MISMATCH}");
    }

    if is_protected_self_edit_slot(event.slot_key.as_str(), person_id) {
        anyhow::bail!("{PERSONA_WB_REJECTED_PROTECTED_SELF_EDIT}");
    }

    if event
        .slot_key
        .as_str()
        .starts_with(PREFIX_PERSONA_WRITEBACK)
    {
        if event.event_type != MemoryEventType::SummaryCompacted {
            anyhow::bail!("{PERSONA_WB_WRITEBACK_REQUIRES_SUMMARY_COMPACTED}");
        }
        return Ok(());
    }

    let canonical_prefix = format!("persona/{person_id}/state_header/");
    if event.slot_key.as_str().starts_with(&canonical_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    let style_profile_prefix = format!("persona/{person_id}/style_profile/");
    if event.slot_key.as_str().starts_with(&style_profile_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_STYLE_PROFILE_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    let big_five_prefix = format!("persona/{person_id}/big_five/");
    if event.slot_key.as_str().starts_with(&big_five_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    let relationship_prefix = format!("persona/{person_id}/relationship/");
    if event.slot_key.as_str().starts_with(&relationship_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_RELATIONSHIP_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    let world_model_prefix = format!("persona/{person_id}/world_model/");
    if event.slot_key.as_str().starts_with(&world_model_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_WORLD_MODEL_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    let user_knowledge_prefix = format!("persona/{person_id}/user_knowledge/");
    if event.slot_key.as_str().starts_with(&user_knowledge_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    let user_facts_prefix = format!("persona/{person_id}/user_facts/");
    if event.slot_key.as_str().starts_with(&user_facts_prefix) {
        if event.event_type != MemoryEventType::FactUpdated {
            anyhow::bail!("{PERSONA_WB_CANONICAL_REQUIRES_FACT_UPDATED}");
        }
        return Ok(());
    }

    if event.slot_key.as_str().starts_with("inferred.") {
        if event.event_type != MemoryEventType::InferredClaim {
            anyhow::bail!("{PERSONA_WB_INFERRED_REQUIRES_INFERRED_CLAIM}");
        }
        return Ok(());
    }

    anyhow::bail!("{PERSONA_WB_REJECTED_SLOT_KEY}");
}

fn is_protected_self_edit_slot(slot_key: &str, person_id: &str) -> bool {
    let protected_prefixes = [
        format!("persona/{person_id}/character/"),
        format!("persona/{person_id}/identity_contract/"),
        format!("persona/{person_id}/self_contract/"),
        format!("persona/{person_id}/motivational_core/"),
        format!("persona/{person_id}/negative_identity/"),
        format!("persona/{person_id}/affect_topology/"),
        format!("persona/{person_id}/latent_bias/"),
    ];

    protected_prefixes
        .iter()
        .any(|prefix| slot_key.starts_with(prefix))
}

/// # Errors
/// Returns an error when the event metadata violates tool memory write policy constraints.
pub fn enforce_tool_memory_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.privacy_level == PrivacyLevel::Secret {
        anyhow::bail!("{TOOL_WB_REJECTS_PRIVACY_SECRET}");
    }
    if event.source_kind != Some(SourceKind::Manual) {
        anyhow::bail!("{TOOL_WB_REQUIRES_SOURCE_KIND_MANUAL}");
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates external autosave write policy constraints.
pub fn enforce_external_autosave_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.source != MemorySource::ExplicitUser {
        anyhow::bail!("{EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_EXPLICIT_USER}");
    }
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{EXTERNAL_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE}");
    }
    let Some(source_kind) = event.source_kind else {
        anyhow::bail!("{EXTERNAL_AUTOSAVE_REQUIRES_SOURCE_KIND}");
    };
    match source_kind {
        SourceKind::Api
        | SourceKind::Conversation
        | SourceKind::Discord
        | SourceKind::Telegram
        | SourceKind::Slack => {}
        _ => anyhow::bail!("{EXTERNAL_AUTOSAVE_REJECTED_SOURCE_KIND}"),
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates agent autosave write policy constraints.
pub fn enforce_agent_autosave_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{AGENT_AUTOSAVE_REQUIRES_PRIVACY_PRIVATE}");
    }
    if event.source_kind != Some(SourceKind::Conversation) {
        anyhow::bail!("{AGENT_AUTOSAVE_REQUIRES_SOURCE_KIND_CONVERSATION}");
    }
    if event.event_type != MemoryEventType::FactAdded {
        anyhow::bail!("{AGENT_AUTOSAVE_REQUIRES_EVENT_TYPE_FACT_ADDED}");
    }
    if event.slot_key.as_str() != SLOT_CONVERSATION_USER_MSG
        && event.slot_key.as_str() != SLOT_CONVERSATION_ASSISTANT_RESP
    {
        anyhow::bail!("{AGENT_AUTOSAVE_REJECTED_SLOT_KEY}");
    }
    match event.source {
        MemorySource::ExplicitUser | MemorySource::System => {}
        _ => anyhow::bail!("{AGENT_AUTOSAVE_REJECTED_SOURCE}"),
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when a working-memory event violates tenant or provenance policy constraints.
pub fn enforce_working_memory_write_policy(
    event: &MemoryEventInput,
    policy_context: &TenantPolicyContext,
) -> anyhow::Result<()> {
    policy_context
        .enforce_recall_scope(event.entity_id.as_str())
        .map_err(anyhow::Error::msg)
        .context("working memory entity outside tenant scope")?;
    if event.layer != MemoryLayer::Working {
        anyhow::bail!("working memory writes require working layer");
    }
    if event.source != MemorySource::System {
        anyhow::bail!("working memory writes require system source");
    }
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("working memory writes require private privacy");
    }
    if event.source_kind != Some(SourceKind::Conversation) {
        anyhow::bail!("working memory writes require conversation source kind");
    }
    if event.event_type != MemoryEventType::FactAdded {
        anyhow::bail!("working memory writes require fact_added event type");
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates conversation state write policy constraints.
pub fn enforce_conversation_state_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.source != MemorySource::System {
        anyhow::bail!("{CONVERSATION_STATE_REQUIRES_SOURCE_SYSTEM}");
    }
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{CONVERSATION_STATE_REQUIRES_PRIVACY_PRIVATE}");
    }
    if event.source_kind != Some(SourceKind::Conversation) {
        anyhow::bail!("{CONVERSATION_STATE_REQUIRES_SOURCE_KIND_CONVERSATION}");
    }
    if event.event_type != MemoryEventType::FactUpdated {
        anyhow::bail!("{CONVERSATION_STATE_REQUIRES_EVENT_TYPE_FACT_UPDATED}");
    }
    if event.slot_key.as_str() != SLOT_CONVERSATION_STATE_V1
        && event.slot_key.as_str() != SLOT_CONVERSATION_LEDGER_V1
    {
        anyhow::bail!("{CONVERSATION_STATE_REJECTED_SLOT_KEY}");
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates inference write policy constraints.
pub fn enforce_inference_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{INFERENCE_WB_REQUIRES_PRIVACY_PRIVATE}");
    }
    if event.source_kind != Some(SourceKind::Conversation) {
        anyhow::bail!("{INFERENCE_WB_REQUIRES_SOURCE_KIND_CONVERSATION}");
    }
    match event.source {
        MemorySource::Inferred | MemorySource::System => {}
        _ => anyhow::bail!("{INFERENCE_WB_REJECTED_SOURCE}"),
    }
    match event.event_type {
        MemoryEventType::InferredClaim | MemoryEventType::ContradictionMarked => {}
        _ => anyhow::bail!("{INFERENCE_WB_REJECTED_EVENT_TYPE}"),
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates user-inference write policy constraints.
pub fn enforce_user_inference_write_policy(
    event: &MemoryEventInput,
    person_id: &str,
) -> anyhow::Result<()> {
    if event.source != MemorySource::System {
        anyhow::bail!("{USER_INFERENCE_REQUIRES_SOURCE_SYSTEM}");
    }
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{USER_INFERENCE_REQUIRES_PRIVACY_PRIVATE}");
    }
    if event.source_kind != Some(SourceKind::Manual) {
        anyhow::bail!("{USER_INFERENCE_REQUIRES_SOURCE_KIND_MANUAL}");
    }
    if event.event_type != MemoryEventType::InferredClaim {
        anyhow::bail!("{USER_INFERENCE_REQUIRES_EVENT_TYPE_INFERRED_CLAIM}");
    }
    let expected_entity = format!("{ENTITY_PREFIX_USER}{person_id}");
    if event.entity_id.as_str() != expected_entity {
        anyhow::bail!("{USER_INFERENCE_ENTITY_ID_MISMATCH}");
    }
    if !event.slot_key.as_str().starts_with(PREFIX_USER_SLOT) {
        anyhow::bail!("{USER_INFERENCE_REQUIRES_USER_PREFIX}");
    }
    if event.slot_key.as_str().is_empty()
        || event.slot_key.as_str().len() > 128
        || event.slot_key.as_str().contains("..")
        || !event
            .slot_key
            .as_str()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        anyhow::bail!("{USER_INFERENCE_REJECTED_SLOT_KEY_FORMAT}");
    }
    if RESERVED_SLOT_PREFIXES
        .iter()
        .any(|prefix| event.slot_key.as_str().starts_with(prefix))
    {
        anyhow::bail!("{USER_INFERENCE_REJECTED_RESERVED_SLOT_KEY}");
    }

    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates verify-repair write policy constraints.
pub fn enforce_verify_repair_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.source != MemorySource::System {
        anyhow::bail!("{VERIFY_REPAIR_REQUIRES_SOURCE_SYSTEM}");
    }
    if event.privacy_level != PrivacyLevel::Private {
        anyhow::bail!("{VERIFY_REPAIR_REQUIRES_PRIVACY_PRIVATE}");
    }
    if event.source_kind != Some(SourceKind::Manual) {
        anyhow::bail!("{VERIFY_REPAIR_REQUIRES_SOURCE_KIND_MANUAL}");
    }
    if event.slot_key.as_str() != SLOT_VERIFY_REPAIR_ESCALATION {
        anyhow::bail!("{VERIFY_REPAIR_REJECTED_SLOT_KEY}");
    }
    if event.event_type != MemoryEventType::SummaryCompacted {
        anyhow::bail!("{VERIFY_REPAIR_REQUIRES_EVENT_TYPE_SUMMARY_COMPACTED}");
    }
    require_common_write_metadata(event)
}

/// # Errors
/// Returns an error when the event metadata violates ingestion write policy constraints.
pub fn enforce_ingestion_write_policy(event: &MemoryEventInput) -> anyhow::Result<()> {
    if event.event_type != MemoryEventType::FactAdded {
        anyhow::bail!("{INGESTION_REQUIRES_EVENT_TYPE_FACT_ADDED}");
    }
    match event.source {
        MemorySource::ExplicitUser
        | MemorySource::ExternalPrimary
        | MemorySource::ExternalSecondary => {}
        _ => anyhow::bail!("{INGESTION_REJECTED_SOURCE}"),
    }
    let Some(source_kind) = event.source_kind else {
        anyhow::bail!("{INGESTION_REQUIRES_SOURCE_KIND}");
    };
    match source_kind {
        SourceKind::Conversation
        | SourceKind::Manual
        | SourceKind::Discord
        | SourceKind::Telegram
        | SourceKind::Slack
        | SourceKind::Api
        | SourceKind::News
        | SourceKind::Document => {}
    }
    if !event.slot_key.as_str().starts_with(PREFIX_EXTERNAL) {
        anyhow::bail!("{INGESTION_REQUIRES_EXTERNAL_SLOT_KEY_PREFIX}");
    }
    require_common_write_metadata(event)
}
