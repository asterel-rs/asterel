//! Context window assembly for a turn.
//!
//! Merges conversation state, fact ledger, and recalled memory into
//! a budget-capped context string, applying external-content
//! sanitization to all untrusted fragments.

mod budget;
mod recall;
mod render;
pub(crate) mod sanitize;

use anyhow::Result;

use self::budget::split_memory_fragment_budgets;
pub use self::budget::{ContextBudget, context_budget_for_model};
use self::recall::recall_memory_context;
pub use self::render::{
    ContextRuntimeMetadata, render_provider_history_block, seed_context_contract,
};
use self::render::{
    RenderedMemoryContext, push_fragment_with_budget, render_fact_ledger_block,
    render_memory_context_fragments, render_runtime_metadata_block, render_state_context_block,
};
#[cfg(test)]
use super::context_contract::ContextFragment;
use super::context_contract::{ContextFragmentKind, ContextFragmentTrust, TurnContextContract};
use super::conversation_state::{load_conversation_state, load_fact_ledger};
use crate::core::memory::Memory;
use crate::security::policy::TenantPolicyContext;

/// All inputs required to assemble a complete turn context contract.
struct ContextAssemblyRequest<'a> {
    /// Memory backend for recall and state/ledger loading.
    mem: &'a dyn Memory,
    /// Entity ID used for all scoped memory queries.
    entity_id: &'a str,
    /// User message text, used as the semantic recall query.
    user_msg: &'a str,
    /// Tenant policy controlling which recall entries may be replayed.
    policy_context: TenantPolicyContext,
    /// Character budgets for each fragment category.
    budget: ContextBudget,
    /// Optional runtime metadata block (model, tenant mode, workspace, etc.).
    runtime_metadata: Option<&'a ContextRuntimeMetadata>,
}

#[cfg(test)]
async fn build_context(mem: &dyn Memory, user_msg: &str) -> String {
    build_context_with_policy(
        mem,
        "default",
        user_msg,
        TenantPolicyContext::disabled(),
        ContextBudget::default(),
    )
    .await
    .unwrap_or_default()
}

/// Assemble a [`TurnContextContract`] from all available context sources.
///
/// Fragment insertion order:
/// 1. Conversation state (if available)
/// 2. Fact ledger (if available)
/// 3. Runtime metadata (if provided)
/// 4. Memory fragments (trusted + sanitised-untrusted, budget-split)
///
/// Each fragment is pushed with an individual character budget. Fragments
/// that exceed their budget are truncated by the contract.
async fn assemble_context_contract(
    request: ContextAssemblyRequest<'_>,
) -> Result<TurnContextContract> {
    let mut contract = TurnContextContract::new(request.budget.total_chars);

    if let Some(state) = load_conversation_state(request.mem, request.entity_id).await {
        let block = render_state_context_block(&state, &request.budget);
        push_fragment_with_budget(
            &mut contract,
            ContextFragmentKind::ConversationState,
            ContextFragmentTrust::Trusted,
            &block,
            request.budget.state_chars,
        );
    }

    if let Some(ledger) = load_fact_ledger(request.mem, request.entity_id).await {
        let block = render_fact_ledger_block(&ledger, request.user_msg, &request.budget);
        push_fragment_with_budget(
            &mut contract,
            ContextFragmentKind::FactLedger,
            ContextFragmentTrust::Trusted,
            &block,
            request.budget.ledger_chars,
        );
    }

    if let Some(runtime_metadata) = request.runtime_metadata {
        let block = render_runtime_metadata_block(runtime_metadata, &request.budget);
        push_fragment_with_budget(
            &mut contract,
            ContextFragmentKind::RuntimeMetadata,
            ContextFragmentTrust::Trusted,
            &block,
            request.budget.runtime_metadata_chars,
        );
    }

    let recalled_context = recall_memory_context(
        request.mem,
        request.entity_id,
        request.user_msg,
        request.policy_context,
    )
    .await?;
    let rendered_memory = render_memory_context_fragments(
        &recalled_context.replayable_entries,
        &recalled_context.contradicted_slots,
        &request.budget,
    );
    append_memory_fragments(&mut contract, &request.budget, &rendered_memory);

    Ok(contract)
}

/// Push trusted memory and sanitised-untrusted memory fragments into the
/// context contract, splitting the remaining memory budget between them.
fn append_memory_fragments(
    contract: &mut TurnContextContract,
    budget: &ContextBudget,
    rendered_memory: &RenderedMemoryContext,
) {
    let (memory_budget_chars, untrusted_budget_chars) = split_memory_fragment_budgets(
        budget,
        !rendered_memory.memory_block.is_empty(),
        !rendered_memory.untrusted_block.is_empty(),
    );

    if !rendered_memory.memory_block.is_empty() {
        push_fragment_with_budget(
            contract,
            ContextFragmentKind::Memory,
            ContextFragmentTrust::Trusted,
            &rendered_memory.memory_block,
            memory_budget_chars,
        );
    }

    if !rendered_memory.untrusted_block.is_empty() {
        push_fragment_with_budget(
            contract,
            ContextFragmentKind::UntrustedContent,
            ContextFragmentTrust::SanitizedUntrusted,
            &rendered_memory.untrusted_block,
            untrusted_budget_chars,
        );
    }
}

/// Build a [`TurnContextContract`] without runtime metadata.
///
/// Convenience wrapper over [`build_context_contract_with_runtime_metadata`].
pub(super) async fn build_context_contract_with_policy(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
    budget: ContextBudget,
) -> Result<TurnContextContract> {
    build_context_contract_with_runtime_metadata(
        mem,
        entity_id,
        user_msg,
        policy_context,
        budget,
        None,
    )
    .await
}

pub(super) async fn build_context_contract_with_runtime_metadata(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
    budget: ContextBudget,
    runtime_metadata: Option<&ContextRuntimeMetadata>,
) -> Result<TurnContextContract> {
    assemble_context_contract(ContextAssemblyRequest {
        mem,
        entity_id,
        user_msg,
        policy_context,
        budget,
        runtime_metadata,
    })
    .await
}

/// Assemble the turn context string from conversation state, fact
/// ledger, and recalled memory, enforcing tenant policy on recall.
///
/// # Errors
///
/// Returns an error if policy enforcement or memory recall fails.
pub(super) async fn build_context_with_policy(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
    budget: ContextBudget,
) -> Result<String> {
    let contract =
        build_context_contract_with_policy(mem, entity_id, user_msg, policy_context, budget)
            .await?;
    Ok(contract.render())
}

/// # Errors
///
/// Returns an error when scoped memory recall fails.
pub async fn build_context_contract_for_integration(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
    budget: ContextBudget,
) -> Result<TurnContextContract> {
    build_context_contract_with_policy(mem, entity_id, user_msg, policy_context, budget).await
}

/// # Errors
///
/// Returns an error when scoped memory recall fails.
pub async fn build_context_contract_with_runtime_metadata_for_integration(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
    budget: ContextBudget,
    runtime_metadata: Option<&ContextRuntimeMetadata>,
) -> Result<TurnContextContract> {
    build_context_contract_with_runtime_metadata(
        mem,
        entity_id,
        user_msg,
        policy_context,
        budget,
        runtime_metadata,
    )
    .await
}

/// # Errors
///
/// Returns an error when scoped memory recall fails.
pub async fn build_context_for_integration(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
    budget: ContextBudget,
) -> Result<String> {
    build_context_with_policy(mem, entity_id, user_msg, policy_context, budget).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{
        MarkdownMemory, MemoryEventInput, MemoryEventType, MemorySource, PrivacyLevel,
    };
    use crate::core::providers::response::ProviderMessage;

    #[tokio::test]
    async fn build_context_replay_ban_hides_raw_external_payload() {
        let temp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        mem.append_event(
            MemoryEventInput::new(
                "default",
                "external.gateway.webhook",
                MemoryEventType::FactAdded,
                "ATTACK_PAYLOAD_ALPHA",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.7),
        )
        .await
        .unwrap();

        let context = build_context(mem.as_ref(), "ATTACK_PAYLOAD_ALPHA").await;
        assert!(context.contains("external.gateway.webhook"));
        assert!(!context.contains("ATTACK_PAYLOAD_ALPHA"));
    }

    #[tokio::test]
    async fn build_context_contract_marks_external_memory_as_sanitized_untrusted() {
        let temp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        mem.append_event(
            MemoryEventInput::new(
                "default",
                "external.gateway.webhook",
                MemoryEventType::FactAdded,
                "digest_sha256=abc123 ATTACK_PAYLOAD_ALPHA",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.7),
        )
        .await
        .unwrap();

        let contract = build_context_contract_for_integration(
            mem.as_ref(),
            "default",
            "ATTACK_PAYLOAD_ALPHA",
            TenantPolicyContext::disabled(),
            ContextBudget::default(),
        )
        .await
        .unwrap();

        assert!(contract.has_sanitized_untrusted_content());
        assert!(contract.fragments.iter().any(|fragment| {
            fragment.kind == ContextFragmentKind::UntrustedContent
                && fragment.trust == ContextFragmentTrust::SanitizedUntrusted
        }));
    }

    #[tokio::test]
    async fn build_context_contract_includes_runtime_metadata_fragment() {
        let temp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let runtime_metadata = ContextRuntimeMetadata::from_entity_scope(
            "tenant-alpha:person:test",
            &TenantPolicyContext::enabled("tenant-alpha"),
        )
        .with_model_name("gpt-4o-mini")
        .with_workspace_dir(temp.path())
        .with_ephemeral(false);

        let contract = build_context_contract_with_runtime_metadata_for_integration(
            mem.as_ref(),
            "tenant-alpha:person:test",
            "hello",
            TenantPolicyContext::enabled("tenant-alpha"),
            ContextBudget::default(),
            Some(&runtime_metadata),
        )
        .await
        .unwrap();

        let rendered = contract.render();
        assert!(rendered.contains("[Runtime metadata]"));
        assert!(rendered.contains("tenant_mode: enabled"));
        assert!(rendered.contains("tenant_id: <tenant-scoped>"));
        assert!(rendered.contains("entity_id: tenant-alpha:<redacted>"));
        assert!(rendered.contains("workspace: <workspace:"));
        assert!(rendered.contains("model: gpt-4o-mini"));
        assert!(!rendered.contains(temp.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn context_budget_for_model_large() {
        let budget = context_budget_for_model("claude-4-sonnet");
        assert_eq!(budget.total_chars, 24_000);
        assert_eq!(budget.ledger_max_items, 16);
        assert_eq!(budget.entry_value_max_chars, 400);
    }

    #[test]
    fn context_budget_for_model_medium() {
        let budget = context_budget_for_model("claude-3-opus");
        assert_eq!(budget.total_chars, 12_000);
        assert_eq!(budget.ledger_max_items, 12);
        assert_eq!(budget.entry_value_max_chars, 300);
    }

    #[test]
    fn context_budget_for_model_default() {
        let budget = context_budget_for_model("unknown-model");
        assert_eq!(budget.total_chars, 6_000);
        assert_eq!(budget.ledger_max_items, 8);
        assert_eq!(budget.entry_value_max_chars, 220);
    }

    #[test]
    fn budget_distribution_ratios() {
        for model in ["claude-4", "claude-3", "unknown"] {
            let budget = context_budget_for_model(model);
            assert!(
                budget.state_chars
                    + budget.ledger_chars
                    + budget.runtime_metadata_chars
                    + budget.memory_chars
                    <= budget.total_chars
            );
        }
    }

    #[test]
    fn render_provider_history_block_formats_turns() {
        let rendered = render_provider_history_block(
            &[
                ProviderMessage::user("hello"),
                ProviderMessage::assistant("world"),
            ],
            400,
        );
        assert!(rendered.contains("[History]"));
        assert!(rendered.contains("user: hello"));
        assert!(rendered.contains("assistant: world"));
    }

    #[test]
    fn seed_context_contract_includes_base_instructions_and_history() {
        let mut dynamic = TurnContextContract::new(500);
        dynamic.push(
            ContextFragment::new(
                ContextFragmentKind::ConversationState,
                ContextFragmentTrust::Trusted,
                200,
                "[Conversation state]\n- focus: test",
            )
            .unwrap(),
        );
        let seeded = seed_context_contract(
            dynamic,
            Some("system prompt"),
            &[ProviderMessage::user("hello")],
            120,
            120,
        );
        assert_eq!(
            seeded.fragments[0].kind,
            ContextFragmentKind::BaseInstructions
        );
        assert_eq!(seeded.fragments[1].kind, ContextFragmentKind::History);
        assert_eq!(
            seeded.fragments[2].kind,
            ContextFragmentKind::ConversationState
        );
    }
}
