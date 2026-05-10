use std::sync::Arc;

use asterel::core::memory::embeddings::NoopEmbedding;
use asterel::core::memory::{
    ForgetMode, MemoryEventInput, MemoryEventType, MemoryGovernance, MemoryLayer, MemoryReader,
    MemorySource, MemoryWriter, PostgresMemory, PrivacyLevel, RecallQuery,
};

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn postgres_append_resolve_recall_and_forget_happy_path() {
    let database_url = crate::test_env::postgres_url()
        .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
    let memory = PostgresMemory::connect(&database_url, Arc::new(NoopEmbedding), 0, false, 0.0)
        .await
        .expect("postgres memory should connect and migrate");

    let entity_id = format!("integration:postgres:{}", uuid::Uuid::new_v4());
    let slot_key = "preferences.favorite_flower";
    let value = "The user prefers blue hydrangeas for release-day bouquets.";

    let before_count = memory
        .count_events(Some(&entity_id))
        .await
        .expect("entity-scoped event count should run");
    assert_eq!(before_count, 0, "unique test entity should start empty");

    let appended = memory
        .append_event(
            MemoryEventInput::new(
                &entity_id,
                slot_key,
                MemoryEventType::PreferenceSet,
                value,
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Semantic)
            .with_confidence(0.97)
            .with_importance(0.8),
        )
        .await
        .expect(
            "append_event should persist the event, slot, retrieval unit, and graph projection",
        );

    assert_eq!(appended.entity_id.as_str(), entity_id);
    assert_eq!(appended.slot_key.as_str(), slot_key);

    let resolved = memory
        .resolve_slot(&entity_id, slot_key)
        .await
        .expect("resolve_slot should query belief_slots")
        .expect("appended slot should resolve");
    assert_eq!(resolved.value, value);

    let recalled = memory
        .recall_scoped(RecallQuery::new(&entity_id, "hydrangeas", 5))
        .await
        .expect("recall_scoped should query retrieval_units and graph metadata");
    assert!(
        recalled
            .iter()
            .any(|entry| entry.slot_key.as_str() == slot_key && entry.value == value),
        "expected FTS recall to surface the persisted retrieval unit, got {recalled:?}"
    );

    let after_count = memory
        .count_events(Some(&entity_id))
        .await
        .expect("entity-scoped event count should run after write");
    assert_eq!(after_count, 1);

    let forget = memory
        .forget_slot(
            &entity_id,
            slot_key,
            ForgetMode::Hard,
            "integration test cleanup",
        )
        .await
        .expect("hard forget should clean up the test slot");
    assert!(
        forget.was_applied,
        "hard forget should report applied cleanup"
    );
    assert!(
        forget.is_complete,
        "hard forget should verify cleanup artifacts"
    );
}
