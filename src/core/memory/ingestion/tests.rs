//! Tests for the memory ingestion pipeline.

use tempfile::TempDir;

use super::*;
use crate::core::memory::MarkdownMemory;
use crate::runtime::observability::PrometheusObserver;

#[tokio::test]
async fn ingestion_pipeline_writes_valid_envelope() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(Arc::clone(&mem));

    let envelope = SignalEnvelope::new(
        SourceKind::Discord,
        "discord:12345",
        "hello from discord",
        "person:discord.12345",
    )
    .with_privacy_level(PrivacyLevel::Private)
    .with_signal_tier(SignalTier::Raw);

    let result = pipeline.ingest(envelope).await.unwrap();
    assert!(result.accepted);
    assert!(result.slot_key.as_str().starts_with("external.discord."));
}

#[tokio::test]
async fn ingestion_pipeline_rejects_empty_source_ref() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(mem);

    let envelope = SignalEnvelope::new(SourceKind::Api, "   ", "payload", "person:api.test");
    let err = pipeline.ingest(envelope).await.unwrap_err().to_string();
    assert!(err.contains("source_ref"));
}

#[tokio::test]
async fn ingestion_pipeline_rejects_source_ref_that_sanitizes_to_empty() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(mem);

    let envelope = SignalEnvelope::new(SourceKind::Api, "???", "payload", "person:api.test");
    let err = pipeline.ingest(envelope).await.unwrap_err().to_string();
    assert!(err.contains("signal_envelope.source_ref must not be empty"));
}

#[tokio::test]
async fn ingestion_pipeline_drops_exact_source_ref_duplicates() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(mem);

    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-1",
        "stable payload",
        "person:api.test",
    );

    let first = pipeline.ingest(envelope.clone()).await.unwrap();
    assert!(first.accepted);

    let second = pipeline.ingest(envelope).await.unwrap();
    assert!(!second.accepted);
    assert_eq!(second.reason.as_deref(), Some("dedup:source_ref_exact"));
}

#[tokio::test]
async fn ingestion_pipeline_drops_semantically_similar_duplicates() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(mem);

    let first = SignalEnvelope::new(
        SourceKind::News,
        "news:item-1",
        "Asterel release planning update",
        "person:news.test",
    );
    let second = SignalEnvelope::new(
        SourceKind::News,
        "news:item-2",
        "Asterel release planning update",
        "person:news.test",
    );

    let first_result = pipeline.ingest(first).await.unwrap();
    assert!(first_result.accepted);

    let second_result = pipeline.ingest(second).await.unwrap();
    assert!(!second_result.accepted);
    assert_eq!(
        second_result.reason.as_deref(),
        Some("dedup:semantic_similar")
    );
}

#[tokio::test]
async fn ingestion_pipeline_keeps_same_content_when_source_kind_differs() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(mem);

    let api = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-same-content",
        "Asterel release planning update",
        "person:dedup.partition",
    );
    let news = SignalEnvelope::new(
        SourceKind::News,
        "news:item-same-content",
        "Asterel release planning update",
        "person:dedup.partition",
    );

    let api_result = pipeline.ingest(api).await.unwrap();
    assert!(api_result.accepted);

    let news_result = pipeline.ingest(news).await.unwrap();
    assert!(news_result.accepted);
    assert!(news_result.reason.is_none());
}

#[tokio::test]
async fn ingestion_pipeline_keeps_same_content_when_entity_id_differs() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let pipeline = DefaultIngestPipeline::new(mem);

    let a = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-entity-a",
        "Asterel release planning update",
        "person:dedup.entity_a",
    );
    let b = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-entity-b",
        "Asterel release planning update",
        "person:dedup.entity_b",
    );

    let first = pipeline.ingest(a).await.unwrap();
    assert!(first.accepted);

    let second = pipeline.ingest(b).await.unwrap();
    assert!(second.accepted);
    assert!(second.reason.is_none());
}

#[test]
fn signal_envelope_normalize_sanitizes_fields() {
    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        " api://user/123?x ",
        "  hello\n\tworld  ",
        " person:api user ",
    )
    .with_language(" JA_jp ");

    let normalized = envelope.normalize().unwrap();
    assert_eq!(normalized.source_ref, "api://user/123_x");
    assert_eq!(normalized.content, "hello world");
    assert_eq!(normalized.entity_id.as_str(), "person:api_user");
    assert_eq!(normalized.language.as_deref(), Some("jajp"));
}

#[test]
fn signal_envelope_normalize_rejects_empty_content() {
    let envelope = SignalEnvelope::new(SourceKind::Api, "api:1", "   \n\t", "person:api.1");
    let err = envelope.normalize().unwrap_err().to_string();
    assert!(err.contains("content"));
}

#[test]
fn signal_envelope_classifies_risk_topic_and_entity_metadata() {
    let envelope = SignalEnvelope::new(
        SourceKind::News,
        "feed:item-1",
        "Unverified rumor about security policy leak with token",
        "person:alice",
    );

    let normalized = envelope.normalize().unwrap();
    let risk = normalized
        .metadata
        .get("risk_flags")
        .cloned()
        .unwrap_or_default();
    assert!(risk.contains("rumor"));
    assert!(risk.contains("unverified"));
    assert!(risk.contains("sensitive"));
    assert!(risk.contains("policy_risky"));
    assert_eq!(
        normalized.metadata.get("topic").map(String::as_str),
        Some("security")
    );
    assert_eq!(
        normalized.metadata.get("entity_hint").map(String::as_str),
        Some("person:alice")
    );
}

#[test]
fn signal_envelope_classification_preserves_preseeded_topic_and_entity_hint() {
    let envelope = SignalEnvelope::new(
        SourceKind::News,
        "feed:item-preserve",
        "release policy note",
        "person:bob",
    )
    .with_metadata("topic", "custom-topic")
    .with_metadata("entity_hint", "custom-entity");

    let normalized = envelope.normalize().unwrap();
    assert_eq!(
        normalized.metadata.get("topic").map(String::as_str),
        Some("custom-topic")
    );
    assert_eq!(
        normalized.metadata.get("entity_hint").map(String::as_str),
        Some("custom-entity")
    );
}

#[test]
fn signal_envelope_classification_emits_sorted_deduplicated_risk_flags() {
    let envelope = SignalEnvelope::new(
        SourceKind::News,
        "feed:item-risk",
        "rumor unverified leak policy secret guidance",
        "person:carol",
    );

    let normalized = envelope.normalize().unwrap();
    assert_eq!(
        normalized.metadata.get("risk_flags").map(String::as_str),
        Some("policy_risky|rumor|sensitive|unverified")
    );
}

#[test]
fn signal_envelope_classification_uses_source_kind_fallback_topic() {
    let envelope = SignalEnvelope::new(
        SourceKind::Manual,
        "trend:item-topic",
        "neutral payload without keyword hints",
        "person:trend.topic",
    );

    let normalized = envelope.normalize().unwrap();
    assert_eq!(
        normalized.metadata.get("topic").map(String::as_str),
        Some("manual")
    );
}

#[test]
fn signal_envelope_classification_uses_community_fallback_for_discord() {
    let envelope = SignalEnvelope::new(
        SourceKind::Discord,
        "discord:item-topic",
        "neutral payload without keyword hints",
        "person:discord.topic",
    );

    let normalized = envelope.normalize().unwrap();
    assert_eq!(
        normalized.metadata.get("topic").map(String::as_str),
        Some("community")
    );
}

#[test]
fn signal_envelope_normalize_rewrites_invalid_ingested_at() {
    let mut envelope = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-timestamp",
        "timestamp payload",
        "person:timestamp.test",
    );
    envelope.ingested_at = "not-a-rfc3339-timestamp".to_string();

    let normalized = envelope.normalize().unwrap();
    assert_ne!(normalized.ingested_at, "not-a-rfc3339-timestamp");
    assert!(chrono::DateTime::parse_from_rfc3339(&normalized.ingested_at).is_ok());
}

#[test]
fn signal_envelope_normalize_rejects_invalid_language_token() {
    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-lang",
        "language payload",
        "person:language.test",
    )
    .with_language("###");

    let err = envelope.normalize().unwrap_err().to_string();
    assert!(err.contains("signal_envelope.language"));
}

#[test]
fn signal_envelope_normalize_rejects_entity_id_over_limit() {
    let long_entity = format!("person:{}", "a".repeat(140));
    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-long-entity",
        "entity length payload",
        long_entity,
    );

    let err = envelope.normalize().unwrap_err().to_string();
    assert!(err.contains("signal_envelope.entity_id must be <= 128 chars"));
}

#[test]
fn signal_envelope_normalize_rejects_source_ref_over_limit() {
    let long_ref = format!("api:{}", "x".repeat(300));
    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        long_ref,
        "source ref length payload",
        "person:source_ref.test",
    );

    let err = envelope.normalize().unwrap_err().to_string();
    assert!(err.contains("signal_envelope.source_ref must be <= 256 chars"));
}

#[tokio::test]
async fn ingestion_pipeline_records_ingested_and_deduplicated_signals() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let observer = Arc::new(PrometheusObserver::new());
    let pipeline = DefaultIngestPipeline::new_with_observer(mem, observer.clone());

    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-observe",
        "observe payload",
        "person:obs.test",
    );

    let first = pipeline.ingest(envelope.clone()).await.unwrap();
    assert!(first.accepted);
    let second = pipeline.ingest(envelope).await.unwrap();
    assert!(!second.accepted);

    let counts = observer.snapshot_autonomy_counts();
    assert!(counts.ingested >= 1);
    assert!(counts.deduplicated >= 1);

    let signal_counts = observer.snapshot_signal_counts();
    assert!(
        signal_counts
            .ingested_by_source
            .get("api")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        signal_counts
            .dedup_drop_by_source
            .get("api")
            .copied()
            .unwrap_or(0)
            >= 1
    );
}

#[tokio::test]
async fn ingestion_pipeline_records_source_metrics_per_kind() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let observer = Arc::new(PrometheusObserver::new());
    let pipeline = DefaultIngestPipeline::new_with_observer(mem, observer.clone());

    let api = SignalEnvelope::new(
        SourceKind::Api,
        "api:source-kind-1",
        "api payload",
        "person:kind.test",
    );
    let news = SignalEnvelope::new(
        SourceKind::News,
        "news:source-kind-1",
        "news payload",
        "person:kind.test",
    );

    let api_result = pipeline.ingest(api).await.unwrap();
    assert!(api_result.accepted);
    let news_result = pipeline.ingest(news).await.unwrap();
    assert!(news_result.accepted);

    let signal_counts = observer.snapshot_signal_counts();
    assert_eq!(signal_counts.ingested_by_source.get("api"), Some(&1));
    assert_eq!(signal_counts.ingested_by_source.get("news"), Some(&1));
}

#[tokio::test]
async fn ingestion_pipeline_records_source_metrics_for_manual_and_discord_kinds() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let observer = Arc::new(PrometheusObserver::new());
    let pipeline = DefaultIngestPipeline::new_with_observer(mem, observer.clone());

    let manual = SignalEnvelope::new(
        SourceKind::Manual,
        "manual:source-kind-1",
        "manual payload",
        "person:kind.manual.discord",
    );
    let discord = SignalEnvelope::new(
        SourceKind::Discord,
        "discord:source-kind-1",
        "discord payload",
        "person:kind.manual.discord",
    );

    let manual_result = pipeline.ingest(manual).await.unwrap();
    assert!(manual_result.accepted);
    let discord_result = pipeline.ingest(discord).await.unwrap();
    assert!(discord_result.accepted);

    let signal_counts = observer.snapshot_signal_counts();
    assert_eq!(signal_counts.ingested_by_source.get("manual"), Some(&1));
    assert_eq!(signal_counts.ingested_by_source.get("discord"), Some(&1));
}

#[tokio::test]
async fn ingestion_pipeline_records_dedup_drop_metrics_per_kind() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let observer = Arc::new(PrometheusObserver::new());
    let pipeline = DefaultIngestPipeline::new_with_observer(mem, observer.clone());

    let api = SignalEnvelope::new(
        SourceKind::Api,
        "api:source-kind-dedup-1",
        "api dedup payload",
        "person:kind.test",
    );
    let news = SignalEnvelope::new(
        SourceKind::News,
        "news:source-kind-dedup-1",
        "news dedup payload",
        "person:kind.test",
    );

    assert!(pipeline.ingest(api.clone()).await.unwrap().accepted);
    assert!(pipeline.ingest(news.clone()).await.unwrap().accepted);
    assert!(!pipeline.ingest(api).await.unwrap().accepted);
    assert!(!pipeline.ingest(news).await.unwrap().accepted);

    let signal_counts = observer.snapshot_signal_counts();
    assert_eq!(signal_counts.dedup_drop_by_source.get("api"), Some(&1));
    assert_eq!(signal_counts.dedup_drop_by_source.get("news"), Some(&1));
}

#[tokio::test]
async fn ingestion_pipeline_records_dedup_drop_metrics_for_manual_kind() {
    let tmp = TempDir::new().unwrap();
    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
    let observer = Arc::new(PrometheusObserver::new());
    let pipeline = DefaultIngestPipeline::new_with_observer(mem, observer.clone());

    let manual = SignalEnvelope::new(
        SourceKind::Manual,
        "manual:source-kind-dedup-1",
        "manual dedup payload",
        "person:kind.manual.test",
    );

    assert!(pipeline.ingest(manual.clone()).await.unwrap().accepted);
    assert!(!pipeline.ingest(manual).await.unwrap().accepted);

    let signal_counts = observer.snapshot_signal_counts();
    assert_eq!(signal_counts.dedup_drop_by_source.get("manual"), Some(&1));
}

#[test]
fn signal_envelope_json_roundtrip_preserves_required_fields() {
    let envelope = SignalEnvelope::new(
        SourceKind::Api,
        "api:item-json",
        "json payload",
        "person:json.test",
    )
    .with_signal_tier(SignalTier::Raw)
    .with_privacy_level(PrivacyLevel::Private)
    .with_language("en")
    .with_metadata("risk_flags", "sensitive");

    let json = serde_json::to_string(&envelope).expect("serialize envelope");
    let decoded: SignalEnvelope = serde_json::from_str(&json).expect("deserialize envelope");

    assert_eq!(decoded.source_kind, SourceKind::Api);
    assert_eq!(decoded.source_ref, "api:item-json");
    assert_eq!(decoded.entity_id.as_str(), "person:json.test");
    assert_eq!(decoded.content, "json payload");
    assert_eq!(decoded.signal_tier, SignalTier::Raw);
    assert_eq!(decoded.privacy_level, PrivacyLevel::Private);
    assert_eq!(decoded.language.as_deref(), Some("en"));
    assert_eq!(
        decoded.metadata.get("risk_flags").map(String::as_str),
        Some("sensitive")
    );
}

#[test]
fn signal_envelope_json_rejects_invalid_source_kind() {
    let invalid = serde_json::json!({
        "source_kind": "unknown_kind",
        "source_ref": "api:item-json-invalid",
        "content": "json payload",
        "entity_id": "person:json.invalid",
        "signal_tier": "raw",
        "privacy_level": "private",
        "language": "en",
        "metadata": {},
        "ingested_at": chrono::Utc::now().to_rfc3339()
    });

    let err = serde_json::from_value::<SignalEnvelope>(invalid)
        .expect_err("invalid source_kind must fail");
    assert!(err.to_string().contains("unknown variant"));
}
