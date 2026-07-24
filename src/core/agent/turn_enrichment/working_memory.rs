use crate::contracts::memory::MemoryLayer;
use crate::contracts::observability::{Observer, ObserverMetric};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    RecallQuery, SourceKind, WorkingMemorySource, WorkingMemoryView,
};
use crate::security::policy::{TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR, TenantPolicyContext};
use crate::security::writeback_guard::enforce_working_memory_write_policy;

/// Build a [`WorkingMemoryView`] seeded from a scoped memory recall.
///
/// Recalls up to `min(capacity, 20)` items for `entity_id` using
/// `user_message` as the query, then injects the current turn's message
/// at a fixed importance of `0.7`. Recall failures are logged and treated
/// as an empty seed.
pub async fn materialize_working_memory(
    mem: &dyn Memory,
    session_id: &str,
    entity_id: &str,
    user_message: &str,
    capacity: usize,
    policy_context: &TenantPolicyContext,
) -> WorkingMemoryView {
    let scoped_entity_id = match scoped_working_memory_entity_id(entity_id, policy_context) {
        Ok(entity_id) => entity_id,
        Err(error) => {
            tracing::debug!(entity_id, %error, "working memory materialization rejected by tenant policy");
            entity_id.to_string()
        }
    };
    let recall_query = RecallQuery::new(&scoped_entity_id, user_message, capacity.min(20))
        .with_policy_context(policy_context.clone());
    let recalled = match mem.recall_scoped(recall_query).await {
        Ok(items) => items,
        Err(error) => {
            tracing::debug!(%error, "working memory materialization recall failed");
            Vec::new()
        }
    };

    let mut view = WorkingMemoryView::materialize_from_recall(
        session_id,
        scoped_entity_id.as_str(),
        recalled,
        capacity,
    );
    view.add_item(
        "conversation.current_turn",
        user_message,
        WorkingMemorySource::Conversation,
        0.7,
    );
    view
}

/// Persist accumulated (non-recalled) working memory items to the memory store.
///
/// Items with source [`WorkingMemorySource::Recalled`] are skipped — they were
/// loaded from storage and do not need to be re-written. Returns `false` when
/// tenant policy or persistence rejects any accumulated item.
pub async fn flush_working_memory(
    mem: &dyn Memory,
    view: &mut WorkingMemoryView,
    policy_context: &TenantPolicyContext,
    observer: Option<&dyn Observer>,
) -> bool {
    let mut complete = true;
    let items = view.drain_accumulated();
    let scoped_entity_id = match scoped_working_memory_entity_id(view.entity_id(), policy_context) {
        Ok(entity_id) => entity_id,
        Err(error) => {
            tracing::debug!(entity_id = %view.entity_id(), %error, "working memory flush rejected by tenant policy");
            record_working_memory_flush(observer, "rejected");
            return false;
        }
    };
    let source_ref = format!("working-memory:{}", view.session_id());
    for item in &items {
        if item.source == WorkingMemorySource::Recalled {
            continue;
        }
        if item.source == WorkingMemorySource::Conversation
            && item.key == "conversation.current_turn"
        {
            continue;
        }

        let event = MemoryEventInput::new(
            &scoped_entity_id,
            &item.key,
            MemoryEventType::FactAdded,
            &item.value,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Working)
        .with_importance(item.importance)
        .with_source_kind(SourceKind::Conversation)
        .with_source_ref(source_ref.clone())
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            source_ref.clone(),
        ));

        if let Err(error) = enforce_working_memory_write_policy(&event, policy_context) {
            tracing::debug!(key = %item.key, %error, "working memory write policy rejected flush item");
            record_working_memory_flush(observer, "rejected");
            complete = false;
            continue;
        }

        if let Err(error) = mem.append_event(event).await {
            tracing::debug!(key = %item.key, %error, "working memory flush failed");
            record_working_memory_flush(observer, "failure");
            complete = false;
        } else {
            record_working_memory_flush(observer, "success");
        }
    }
    complete
}

fn record_working_memory_flush(observer: Option<&dyn Observer>, status: &str) {
    if let Some(observer) = observer {
        observer.record_metric(&ObserverMetric::PostTurnHook {
            hook: "working_memory_flush".to_string(),
            status: status.to_string(),
        });
    }
}

fn scoped_working_memory_entity_id(
    entity_id: &str,
    policy_context: &TenantPolicyContext,
) -> Result<String, &'static str> {
    let requested = entity_id.trim();
    if policy_context.tenant_mode_enabled && (requested.is_empty() || requested == "default") {
        return Err(TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR);
    }
    let scoped = policy_context.scope_entity_id(requested);
    policy_context.enforce_recall_scope(&scoped)?;
    Ok(scoped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::memory_traits::{MemoryGovernance, MemoryReader, MemoryWriter};
    use crate::core::memory::MarkdownMemory;
    use crate::security::policy::TenantPolicyContext;

    #[tokio::test]
    async fn flush_skips_current_turn_conversation_item() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        let mut view = materialize_working_memory(
            &mem,
            "session-1",
            "person:gateway.user-1",
            "raw user turn with instruction-like text",
            10,
            &TenantPolicyContext::disabled(),
        )
        .await;

        flush_working_memory(&mem, &mut view, &TenantPolicyContext::disabled(), None).await;

        let slot = mem
            .resolve_slot("person:gateway.user-1", "conversation.current_turn")
            .await
            .unwrap();
        assert!(slot.is_none());
    }

    #[tokio::test]
    async fn flush_persists_non_current_turn_working_items() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        let mut view = WorkingMemoryView::new("session-1", "person:gateway.user-1", 10);
        view.add_item(
            "working.focus",
            "remember the active task",
            WorkingMemorySource::System,
            0.6,
        );

        flush_working_memory(&mem, &mut view, &TenantPolicyContext::disabled(), None).await;

        let slot = mem
            .resolve_slot("person:gateway.user-1", "working.focus")
            .await
            .unwrap()
            .expect("non-current-turn working item should persist");
        assert_eq!(slot.value, "remember the active task");
    }

    #[tokio::test]
    async fn materialize_working_memory_applies_tenant_policy_context() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        mem.append_event(MemoryEventInput::new(
            "tenant-alpha:person:gateway.user-1",
            "working.focus",
            MemoryEventType::FactAdded,
            "tenant scoped focus item",
            MemorySource::System,
            PrivacyLevel::Private,
        ))
        .await
        .unwrap();

        let view = materialize_working_memory(
            &mem,
            "session-1",
            "person:gateway.user-1",
            "tenant scoped focus",
            10,
            &TenantPolicyContext::enabled("tenant-alpha"),
        )
        .await;

        assert_eq!(view.entity_id(), "tenant-alpha:person:gateway.user-1");
        assert!(view.find_by_key("working.focus").is_some());
    }

    #[tokio::test]
    async fn flush_working_memory_writes_to_tenant_scoped_entity_with_provenance() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        let mut view = WorkingMemoryView::new("session-1", "person:gateway.user-1", 10);
        view.add_item(
            "working.focus",
            "remember the tenant scoped task",
            WorkingMemorySource::System,
            0.6,
        );

        flush_working_memory(
            &mem,
            &mut view,
            &TenantPolicyContext::enabled("tenant-alpha"),
            None,
        )
        .await;

        assert!(
            mem.resolve_slot("person:gateway.user-1", "working.focus")
                .await
                .unwrap()
                .is_none(),
            "tenant-mode flush must not write the unscoped entity"
        );
        let slot = mem
            .resolve_slot("tenant-alpha:person:gateway.user-1", "working.focus")
            .await
            .unwrap()
            .expect("tenant-scoped working item should persist");
        assert_eq!(slot.value, "remember the tenant scoped task");
        let provenance = mem
            .slot_provenance("tenant-alpha:person:gateway.user-1", "working.focus")
            .await
            .unwrap()
            .expect("working-memory flush should preserve provenance");
        assert_eq!(provenance.source_class, MemorySource::System);
        assert_eq!(provenance.reference, "working-memory:session-1");
    }
}
