use std::sync::Arc;

use asterel::core::memory::{
    DefaultIngestPipeline, IngestionPipeline, Memory, MemoryCategory, MemoryEventInput,
    MemoryEventType, MemorySource, PrivacyLevel, RecallQuery, SignalEnvelope, SourceKind,
};

use super::memory_harness;

async fn assert_recall_phased_fallback(memory: &dyn Memory, entity_id: &str, slot_key: &str) {
    memory_harness::append_test_event(
        memory,
        entity_id,
        slot_key,
        "compatibility fallback payload",
        MemoryCategory::Core,
    )
    .await;

    let scoped = memory
        .recall_scoped(RecallQuery::new(entity_id, "fallback payload", 10))
        .await
        .expect("recall_scoped should succeed");
    let phased = memory
        .recall_phased(RecallQuery::new(entity_id, "fallback payload", 10))
        .await
        .expect("recall_phased should succeed");

    assert_eq!(scoped.len(), phased.len());
    assert_eq!(
        scoped.first().map(|item| item.slot_key.as_str()),
        phased.first().map(|item| item.slot_key.as_str())
    );
}

async fn assert_ingestion_exact_dedup(
    memory: Arc<dyn Memory>,
    source_kind: SourceKind,
    source_ref: &str,
) {
    let pipeline = DefaultIngestPipeline::new(Arc::clone(&memory));
    let envelope = SignalEnvelope::new(
        source_kind,
        source_ref,
        "compat dedup payload",
        "compat:dedup.entity",
    );

    let first = pipeline
        .ingest(envelope.clone())
        .await
        .expect("first ingest should succeed");
    assert!(first.accepted);

    let second = pipeline
        .ingest(envelope)
        .await
        .expect("second ingest should return dedup decision");
    assert!(!second.accepted);
    assert_eq!(second.reason.as_deref(), Some("dedup:source_ref_exact"));
}

async fn assert_ingestion_source_ref_partitioned_by_source_kind(memory: Arc<dyn Memory>) {
    let pipeline = DefaultIngestPipeline::new(Arc::clone(&memory));
    let shared_source_ref = "compat:shared-source-ref";

    let api = SignalEnvelope::new(
        SourceKind::Api,
        shared_source_ref,
        "compat partition payload",
        "compat:dedup.partition.entity",
    );
    let news = SignalEnvelope::new(
        SourceKind::News,
        shared_source_ref,
        "compat partition payload",
        "compat:dedup.partition.entity",
    );

    let first = pipeline
        .ingest(api)
        .await
        .expect("api ingest should succeed");
    assert!(first.accepted);

    let second = pipeline
        .ingest(news)
        .await
        .expect("news ingest should succeed");
    assert!(
        second.accepted,
        "source_kind should partition source_ref exact dedup"
    );
    assert!(second.reason.is_none());
}

async fn assert_append_event_normalizes_identifiers(memory: &dyn Memory) {
    memory
        .append_event(MemoryEventInput::new(
            "  compat entity 01  ",
            " profile name / display ",
            MemoryEventType::FactAdded,
            "normalized ingress payload",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect("append_event should normalize identifiers");

    let normalized = memory
        .resolve_slot("compat_entity_01", "profile_name_/_display")
        .await
        .expect("resolve_slot should succeed")
        .expect("normalized slot should exist");
    assert!(normalized.value.contains("normalized ingress payload"));
}

async fn assert_append_event_rejects_empty_identifiers(memory: &dyn Memory) {
    let err = memory
        .append_event(MemoryEventInput::new(
            "   ",
            "profile-name",
            MemoryEventType::FactAdded,
            "invalid ingress payload",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect_err("append_event should reject empty normalized entity_id")
        .to_string();
    assert!(err.contains("memory_event_input.entity_id must not be empty"));

    let err = memory
        .append_event(MemoryEventInput::new(
            "compat_entity_01",
            "   ",
            MemoryEventType::FactAdded,
            "invalid ingress payload",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect_err("append_event should reject empty normalized slot_key")
        .to_string();
    assert!(err.contains("memory_event_input.slot_key must not be empty"));
}

async fn assert_append_event_rejects_invalid_slot_taxonomy_pattern(memory: &dyn Memory) {
    let err = memory
        .append_event(MemoryEventInput::new(
            "compat_entity_01",
            ".invalid-slot",
            MemoryEventType::FactAdded,
            "invalid taxonomy payload",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect_err("append_event should reject invalid slot taxonomy pattern")
        .to_string();
    assert!(err.contains("memory_event_input.slot_key must match taxonomy pattern"));
}

#[tokio::test]
async fn markdown_recall_phased_falls_back_to_scoped() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    assert_recall_phased_fallback(&markdown, "compat:markdown", "compat.markdown.fallback").await;
}

#[tokio::test]
async fn markdown_append_event_normalizes_identifiers() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    assert_append_event_normalizes_identifiers(&markdown).await;
}

#[tokio::test]
async fn markdown_append_event_rejects_empty_identifiers() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    assert_append_event_rejects_empty_identifiers(&markdown).await;
}

#[tokio::test]
async fn markdown_append_event_rejects_invalid_slot_taxonomy_pattern() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    assert_append_event_rejects_invalid_slot_taxonomy_pattern(&markdown).await;
}

#[tokio::test]
async fn ingestion_pipeline_accepts_markdown_backend() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    let memory: Arc<dyn Memory> = Arc::new(markdown);
    let pipeline = DefaultIngestPipeline::new(Arc::clone(&memory));

    let result = pipeline
        .ingest(SignalEnvelope::new(
            SourceKind::Api,
            "compat:md:1",
            "markdown ingestion compatibility payload",
            "compat:markdown.ingest",
        ))
        .await
        .expect("ingest should succeed for markdown backend");
    assert!(result.accepted);

    let slot = memory
        .resolve_slot("compat:markdown.ingest", result.slot_key.as_str())
        .await
        .expect("resolve_slot should succeed")
        .expect("ingestion slot should exist");
    assert!(
        slot.value
            .contains("markdown ingestion compatibility payload")
    );
}

#[tokio::test]
async fn ingestion_pipeline_dedup_works_with_markdown_backend() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    let memory: Arc<dyn Memory> = Arc::new(markdown);
    assert_ingestion_exact_dedup(memory, SourceKind::Api, "compat:md:dedup").await;
}

#[tokio::test]
async fn ingestion_pipeline_source_ref_partitioned_by_source_kind_with_markdown_backend() {
    let (_tmp, markdown) = memory_harness::markdown_fixture();
    let memory: Arc<dyn Memory> = Arc::new(markdown);
    assert_ingestion_source_ref_partitioned_by_source_kind(memory).await;
}
