use std::collections::HashSet;

use anyhow::Result;

use crate::contracts::ids::SlotKey;
use crate::core::memory::{Memory, MemoryRecallEntry, MemorySource, RecallQuery};
use crate::security::policy::TenantPolicyContext;

pub(super) struct RecalledMemoryContext {
    pub(super) replayable_entries: Vec<MemoryRecallEntry>,
    pub(super) contradicted_slots: HashSet<SlotKey>,
}

pub(super) async fn recall_memory_context(
    mem: &dyn Memory,
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
) -> Result<RecalledMemoryContext> {
    let query = build_context_recall_query(entity_id, user_msg, policy_context.clone())?;
    let entries = mem.recall_scoped(query).await?;
    let replayable_entries = filter_replayable_entries(mem, entries).await;
    let contradicted_slots = load_contradicted_slots(mem, entity_id, policy_context).await;

    Ok(RecalledMemoryContext {
        replayable_entries,
        contradicted_slots,
    })
}

fn build_context_recall_query(
    entity_id: &str,
    user_msg: &str,
    policy_context: TenantPolicyContext,
) -> Result<RecallQuery> {
    let query = RecallQuery::new(entity_id, user_msg, 8).with_policy_context(policy_context);
    query.enforce_policy()?;
    Ok(query)
}

async fn filter_replayable_entries(
    mem: &dyn Memory,
    entries: Vec<MemoryRecallEntry>,
) -> Vec<MemoryRecallEntry> {
    let mut replayable_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        if allow_context_replay_item(mem, &entry).await {
            replayable_entries.push(entry);
        }
    }
    replayable_entries
}

async fn allow_context_replay_item(mem: &dyn Memory, entry: &MemoryRecallEntry) -> bool {
    let resolved = mem
        .resolve_slot(entry.entity_id.as_str(), entry.slot_key.as_str())
        .await;
    matches!(resolved, Ok(Some(slot)) if slot.value == entry.value)
}

async fn load_contradicted_slots(
    mem: &dyn Memory,
    entity_id: &str,
    policy_context: TenantPolicyContext,
) -> HashSet<SlotKey> {
    let mut contradicted_slots = HashSet::new();
    let contradiction_queries = [
        "contradiction_marked",
        "inference.post_turn.contradiction_event",
        "contradiction",
    ];

    for query_text in contradiction_queries {
        let query =
            RecallQuery::new(entity_id, query_text, 64).with_policy_context(policy_context.clone());
        if let Err(error) = query.enforce_policy() {
            tracing::warn!(%error, "contradiction recall query rejected by policy");
            continue;
        }

        match mem.recall_scoped(query).await {
            Ok(items) => {
                for item in items {
                    if item.source == MemorySource::System {
                        contradicted_slots.insert(item.slot_key);
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "contradiction recall lookup failed");
            }
        }
    }

    contradicted_slots
}
