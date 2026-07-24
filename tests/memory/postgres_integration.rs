use std::sync::Arc;

use asterel::core::memory::embeddings::NoopEmbedding;
use asterel::core::memory::{
    ForgetMode, MemoryEventInput, MemoryEventType, MemoryGovernance, MemoryLayer, MemoryReader,
    MemorySource, MemoryWriter, PostgresMemory, PrivacyLevel, RecallQuery,
};

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn postgres_append_resolve_recall_and_forget_happy_path() {
    init_tracing();

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
    assert_eq!(
        memory
            .count_events(Some(&entity_id))
            .await
            .expect("hard forget event count should run"),
        0,
        "hard forget must physically remove the slot event log"
    );
    assert!(
        memory
            .resolve_slot(&entity_id, slot_key)
            .await
            .expect("hard-forgotten slot lookup should run")
            .is_none(),
        "hard forget must remove all resolvable projections"
    );
    assert!(
        memory
            .recall_scoped(RecallQuery::new(&entity_id, "hydrangeas", 5))
            .await
            .expect("hard-forgotten slot recall should run")
            .iter()
            .all(|entry| entry.slot_key.as_str() != slot_key),
        "hard forget must remove searchable projections"
    );
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn postgres_recall_reinforces_multiple_hits() {
    init_tracing();

    let database_url = crate::test_env::postgres_url()
        .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
    let memory = PostgresMemory::connect(&database_url, Arc::new(NoopEmbedding), 0, false, 0.0)
        .await
        .expect("postgres memory should connect and migrate");

    let entity_id = format!("integration:postgres:batch:{}", uuid::Uuid::new_v4());
    let slots = [
        (
            "preferences.favorite_flower",
            "The user prefers blue hydrangeas for release-day bouquets.",
        ),
        (
            "preferences.garden_theme",
            "The user wants hydrangeas in the garden planning notes.",
        ),
        (
            "preferences.event_style",
            "The user associates hydrangeas with calm launch rituals.",
        ),
    ];

    for (slot_key, value) in slots {
        memory
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
            .expect("append_event should persist each batch recall fixture");
    }

    let recalled = memory
        .recall_scoped(RecallQuery::new(&entity_id, "hydrangeas", 5))
        .await
        .expect("recall_scoped should surface multiple retrieval units");
    assert!(
        recalled.len() >= 3,
        "expected multiple recall hits for batched reinforcement, got {recalled:?}"
    );

    for (slot_key, _) in slots {
        let forget = memory
            .forget_slot(
                &entity_id,
                slot_key,
                ForgetMode::Hard,
                "integration test cleanup",
            )
            .await
            .expect("hard forget should clean up each test slot");
        assert!(forget.was_applied, "hard forget should apply cleanup");
        assert!(
            forget.is_complete,
            "hard forget should verify cleanup artifacts"
        );
    }
}
