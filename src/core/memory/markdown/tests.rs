//! Tests for the markdown memory backend.

use std::fs as sync_fs;
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::core::memory::MemoryEventType;
use crate::core::memory::{Memory, MemoryError};

fn temp_workspace() -> (TempDir, MarkdownMemory) {
    let tmp = TempDir::new().unwrap();
    let mem = MarkdownMemory::new(tmp.path());
    (tmp, mem)
}

#[tokio::test]
async fn markdown_name() {
    let (_tmp, mem) = temp_workspace();
    assert_eq!(mem.name(), "markdown");
}

#[tokio::test]
async fn markdown_health_check() {
    let (_tmp, mem) = temp_workspace();
    assert!(mem.health_check().await);
}

#[tokio::test]
async fn markdown_store_core() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "pref",
        "User likes Rust",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    let content = sync_fs::read_to_string(mem.core_path()).unwrap();
    assert!(content.contains("User likes Rust"));
}

#[tokio::test]
async fn markdown_store_daily() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "note",
        "Finished tests",
        MemoryCategory::Daily,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    let path = mem.daily_path();
    let content = sync_fs::read_to_string(path).unwrap();
    assert!(content.contains("Finished tests"));
}

#[tokio::test]
async fn markdown_recall_keyword() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "a",
        "Rust is fast",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    mem.upsert_projection(
        "b",
        "Python is slow",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    mem.upsert_projection(
        "c",
        "Rust and safety",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();

    let results = mem.search_projection("Rust", 10, None, None).await.unwrap();
    assert!(results.len() >= 2);
    assert!(
        results
            .iter()
            .all(|r| r.content.to_lowercase().contains("rust"))
    );
}

#[tokio::test]
async fn markdown_recall_no_match() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "a",
        "Rust is great",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    let results = mem
        .search_projection("javascript", 10, None, None)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn markdown_count() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "a",
        "first",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    mem.upsert_projection(
        "b",
        "second",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    let count = mem.count_projection_entries().await.unwrap();
    assert!(count >= 2);
}

#[tokio::test]
async fn markdown_list_by_category() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "a",
        "core fact",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    mem.upsert_projection(
        "b",
        "daily note",
        MemoryCategory::Daily,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();

    let core = mem
        .list_projection_entries(Some(&MemoryCategory::Core))
        .await
        .unwrap();
    assert!(core.iter().all(|e| e.category == MemoryCategory::Core));

    let daily = mem
        .list_projection_entries(Some(&MemoryCategory::Daily))
        .await
        .unwrap();
    assert!(daily.iter().all(|e| e.category == MemoryCategory::Daily));
}

#[tokio::test]
async fn markdown_forget_is_noop() {
    let (_tmp, mem) = temp_workspace();
    mem.upsert_projection(
        "a",
        "permanent",
        MemoryCategory::Core,
        MemoryLayer::Working,
        None,
    )
    .await
    .unwrap();
    let removed = mem.delete_projection_entry("a").await.unwrap();
    assert!(!removed, "Markdown memory is append-only");
}

#[tokio::test]
async fn markdown_empty_recall() {
    let (_tmp, mem) = temp_workspace();
    let results = mem
        .search_projection("anything", 10, None, None)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn markdown_empty_count() {
    let (_tmp, mem) = temp_workspace();
    assert_eq!(mem.count_projection_entries().await.unwrap(), 0);
}

#[tokio::test]
async fn markdown_lists_entities_for_admin_memory_review() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.name",
        MemoryEventType::FactAdded,
        "Alice",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();
    mem.append_event(MemoryEventInput::new(
        "room:writer-lounge",
        "topic.active",
        MemoryEventType::FactAdded,
        "Noir worldbuilding",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();

    let entities = mem.list_entities().await.unwrap();

    assert!(entities.contains(&"person:alice".to_string()));
    assert!(entities.contains(&"room:writer-lounge".to_string()));
}

#[tokio::test]
async fn markdown_concurrent_appends_preserve_all_core_entries() {
    let (_tmp, mem) = temp_workspace();
    let mem = Arc::new(mem);
    let mut handles = Vec::new();

    for index in 0..16 {
        let mem = Arc::clone(&mem);
        handles.push(tokio::spawn(async move {
            mem.append_event(MemoryEventInput::new(
                format!("person:concurrent-{index}"),
                "profile.note",
                MemoryEventType::FactAdded,
                format!("Concurrent note {index}"),
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            ))
            .await
        }));
    }

    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    let content = sync_fs::read_to_string(mem.core_path()).unwrap();
    for index in 0..16 {
        assert!(
            content.contains(&format!("person:concurrent-{index}:profile.note")),
            "missing concurrent append {index} in {content}"
        );
    }
}

#[tokio::test]
async fn markdown_cross_instance_appends_preserve_all_core_entries() {
    let tmp = TempDir::new().unwrap();
    let mut handles = Vec::new();

    for index in 0..16 {
        let workspace = tmp.path().to_path_buf();
        handles.push(tokio::spawn(async move {
            let mem = MarkdownMemory::new(&workspace);
            mem.append_event(MemoryEventInput::new(
                format!("person:cross-process-{index}"),
                "profile.note",
                MemoryEventType::FactAdded,
                format!("Cross instance note {index}"),
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            ))
            .await
        }));
    }

    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    let content = sync_fs::read_to_string(tmp.path().join("MEMORY.md")).unwrap();
    for index in 0..16 {
        assert!(
            content.contains(&format!("person:cross-process-{index}:profile.note")),
            "missing cross-instance append {index} in {content}"
        );
    }
}

#[tokio::test]
async fn markdown_lists_current_slots_for_entity_review() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.name",
        MemoryEventType::FactAdded,
        "Alice",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.preference",
        MemoryEventType::FactAdded,
        "Quiet replies",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();

    let slots = mem.list_slots("person:alice").await.unwrap();

    assert_eq!(slots.len(), 2);
    assert!(
        slots
            .iter()
            .any(|slot| slot.slot_key.as_str() == "profile.name")
    );
    assert!(
        slots
            .iter()
            .any(|slot| slot.slot_key.as_str() == "profile.preference")
    );
}

#[tokio::test]
async fn markdown_current_slot_review_uses_latest_same_day_entry() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.name",
        MemoryEventType::FactAdded,
        "Alice",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.name",
        MemoryEventType::FactUpdated,
        "Alice Liddell",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();

    let slots = mem.list_slots("person:alice").await.unwrap();

    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].value, "Alice Liddell");
}

#[tokio::test]
async fn markdown_fact_updated_keeps_existing_projection_category() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.timezone",
        MemoryEventType::FactAdded,
        "PST",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.timezone",
        MemoryEventType::FactUpdated,
        "PDT",
        MemorySource::System,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();

    let slot = mem
        .resolve_slot("person:alice", "profile.timezone")
        .await
        .unwrap()
        .expect("slot should exist");

    assert_eq!(slot.value, "PDT");

    let core_entries = mem
        .list_projection_entries(Some(&MemoryCategory::Core))
        .await
        .unwrap();
    assert!(core_entries.iter().any(|entry| entry.content == "PDT"));
    let daily_entries = mem
        .list_projection_entries(Some(&MemoryCategory::Daily))
        .await
        .unwrap();
    assert!(daily_entries.is_empty());
}

#[tokio::test]
async fn markdown_current_slot_prefers_dated_update_over_undated_core_entry() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.timezone",
        MemoryEventType::FactAdded,
        "PST",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.timezone",
        MemoryEventType::FactAdded,
        "PDT",
        MemorySource::System,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();

    let resolved = mem
        .resolve_slot("person:alice", "profile.timezone")
        .await
        .unwrap()
        .expect("slot should exist");
    assert_eq!(resolved.value, "PDT");

    let slots = mem.list_slots("person:alice").await.unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].value, "PDT");
}

#[tokio::test]
async fn markdown_slot_provenance_uses_latest_same_day_entry() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(
        MemoryEventInput::new(
            "person:alice",
            "profile.name",
            MemoryEventType::FactAdded,
            "Alice",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        )
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::ExplicitUser,
            "first-reference",
        )),
    )
    .await
    .unwrap();
    mem.append_event(
        MemoryEventInput::new(
            "person:alice",
            "profile.name",
            MemoryEventType::FactUpdated,
            "Alice Liddell",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        )
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::ExplicitUser,
            "second-reference",
        )),
    )
    .await
    .unwrap();

    let memory: Arc<dyn Memory> = Arc::new(mem);
    let provenance = memory
        .slot_provenance("person:alice", "profile.name")
        .await
        .unwrap()
        .expect("latest provenance should be returned");

    assert_eq!(provenance.reference, "second-reference");
}

#[tokio::test]
async fn markdown_trait_object_preserves_typed_validation_and_policy_errors() {
    let (_tmp, mem) = temp_workspace();
    let memory: Arc<dyn Memory> = Arc::new(mem);

    let validation_error = memory
        .append_event(MemoryEventInput::new(
            "   ",
            "profile.name",
            MemoryEventType::FactAdded,
            "Alice",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect_err("blank entity id should be rejected");
    assert!(matches!(validation_error, MemoryError::Validation(_)));

    let policy_error = memory
        .recall_scoped(
            RecallQuery::new("default", "anything", 5).with_policy_context(
                crate::security::policy::TenantPolicyContext::enabled("tenant-alpha"),
            ),
        )
        .await
        .expect_err("tenant policy should reject default scope");
    assert!(matches!(policy_error, MemoryError::Policy(_)));
}

#[tokio::test]
async fn markdown_resolve_slot_requires_exact_projection_key() {
    let (_tmp, mem) = temp_workspace();
    mem.append_event(MemoryEventInput::new(
        "person:alice",
        "profile.note",
        MemoryEventType::FactAdded,
        "This note mentions person:alice:profile.name but is not that slot",
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    ))
    .await
    .unwrap();

    let slot = mem
        .resolve_slot("person:alice", "profile.name")
        .await
        .unwrap();

    assert!(slot.is_none());
}

#[tokio::test]
async fn markdown_resolve_slot_uses_latest_dated_projection_entry() {
    let (_tmp, mem) = temp_workspace();
    sync_fs::create_dir_all(mem.memory_dir()).unwrap();
    sync_fs::write(
        mem.memory_dir().join("2026-04-29.md"),
        "- **person:alice:profile.name** [md:layer=semantic]: Alice\n",
    )
    .unwrap();
    sync_fs::write(
        mem.memory_dir().join("2026-04-30.md"),
        "- **person:alice:profile.name** [md:layer=semantic]: Alice Liddell\n",
    )
    .unwrap();

    let slot = mem
        .resolve_slot("person:alice", "profile.name")
        .await
        .unwrap()
        .expect("latest slot should be returned");

    assert_eq!(slot.value, "Alice Liddell");
    assert_eq!(slot.updated_at, "2026-04-30");
}

#[tokio::test]
async fn markdown_recall_scoped_filters_identity_layer() {
    let (_tmp, mem) = temp_workspace();
    for index in 0..3 {
        mem.append_event(
            MemoryEventInput::new(
                "person:alice",
                format!("profile.note.{index}"),
                MemoryEventType::FactAdded,
                "shared continuity note",
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Semantic),
        )
        .await
        .unwrap();
    }
    mem.append_event(
        MemoryEventInput::new(
            "person:alice",
            "identity.objective",
            MemoryEventType::IdentityObjectiveChanged,
            "shared continuity objective",
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Identity),
    )
    .await
    .unwrap();

    let results = mem
        .recall_scoped(
            RecallQuery::new("person:alice", "shared continuity", 1)
                .with_layer_filter(MemoryLayer::Identity),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].slot_key.as_str(), "identity.objective");
}

#[tokio::test]
async fn markdown_recall_scoped_filters_entity_before_limit() {
    let (_tmp, mem) = temp_workspace();
    for index in 0..3 {
        mem.append_event(
            MemoryEventInput::new(
                "person:bob",
                format!("identity.objective.{index}"),
                MemoryEventType::IdentityObjectiveChanged,
                "shared continuity objective",
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Identity),
        )
        .await
        .unwrap();
    }
    mem.append_event(
        MemoryEventInput::new(
            "person:alice",
            "identity.objective",
            MemoryEventType::IdentityObjectiveChanged,
            "shared continuity objective",
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Identity),
    )
    .await
    .unwrap();

    let results = mem
        .recall_scoped(
            RecallQuery::new("person:alice", "shared continuity", 1)
                .with_layer_filter(MemoryLayer::Identity),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id.as_str(), "person:alice");
    assert_eq!(results[0].slot_key.as_str(), "identity.objective");
}
