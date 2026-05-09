//! Contradiction detection trait and slot-value implementation.
//!
//! Compares inferred claims against existing belief slots to find
//! conflicting facts stored in memory.

use std::future::Future;
use std::pin::Pin;

use super::{Claim, ContradictionFinding};
use crate::core::memory::{Memory, MemoryInferenceEvent};

/// Trait for detecting contradictions between inferred events and
/// existing memory.
pub(crate) trait ContradictionDetector: Send + Sync {
    /// Compare inferred events against stored slots and return any
    /// contradictions found.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying memory query fails.
    fn detect_contradictions<'a>(
        &'a self,
        mem: &'a dyn Memory,
        entity_id: &'a str,
        inferred_events: &'a [MemoryInferenceEvent],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ContradictionFinding>>> + Send + 'a>>;
}

/// Slot-value based contradiction detector.
/// Compares inferred claims against existing belief slots.
pub(crate) struct SlotValueDetector {
    /// Minimum confidence of existing slot to trigger contradiction check.
    min_existing_confidence: f64,
}

impl SlotValueDetector {
    /// Create a detector with the default minimum confidence threshold.
    pub(crate) fn new() -> Self {
        Self {
            min_existing_confidence: 0.5,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_min_existing_confidence(min_existing_confidence: f64) -> Self {
        Self {
            min_existing_confidence: min_existing_confidence.clamp(0.0, 1.0),
        }
    }
}

impl Default for SlotValueDetector {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_value(value: &str) -> String {
    value.trim().to_lowercase()
}

fn finding_from_existing(existing_value: &str, claim: Claim) -> Option<ContradictionFinding> {
    if normalize_value(existing_value) == normalize_value(&claim.new_value) {
        return None;
    }

    Some(ContradictionFinding {
        slot_key: claim.slot_key,
        existing_value: existing_value.to_string(),
        new_value: claim.new_value,
        contradiction_confidence: claim.extraction_confidence,
    })
}

impl ContradictionDetector for SlotValueDetector {
    fn detect_contradictions<'a>(
        &'a self,
        mem: &'a dyn Memory,
        entity_id: &'a str,
        inferred_events: &'a [MemoryInferenceEvent],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ContradictionFinding>>> + Send + 'a>> {
        Box::pin(async move {
            // Cap the number of events processed to avoid unbounded sequential
            // resolve_slot calls on very large event sets.
            const MAX_EVENTS: usize = 100;
            let mut findings = Vec::new();

            for event in inferred_events.iter().take(MAX_EVENTS) {
                let MemoryInferenceEvent::InferredClaim {
                    slot_key,
                    value,
                    confidence,
                    ..
                } = event
                else {
                    continue;
                };

                let claim = Claim {
                    slot_key: slot_key.clone(),
                    new_value: value.clone(),
                    extraction_confidence: *confidence,
                };

                let Some(existing) = mem.resolve_slot(entity_id, claim.slot_key.as_str()).await?
                else {
                    continue;
                };

                if existing.confidence.get() < self.min_existing_confidence {
                    continue;
                }

                if let Some(finding) = finding_from_existing(&existing.value, claim) {
                    findings.push(finding);
                }
            }

            Ok(findings)
        })
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use tempfile::TempDir;

    use super::{ContradictionDetector, SlotValueDetector};
    use crate::core::memory::traits::MemoryWriter;
    use crate::core::memory::{
        MarkdownMemory, MemoryEventInput, MemoryEventType, MemoryInferenceEvent, MemorySource,
        PrivacyLevel,
    };

    #[tokio::test]
    async fn detects_contradiction_when_high_confidence_existing_value_differs() {
        let tmp = TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());

        mem.append_event(MemoryEventInput::new(
            "person:test",
            "profile.language",
            MemoryEventType::FactAdded,
            "Python",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .unwrap();

        let events = vec![
            MemoryInferenceEvent::inferred_claim("person:test", "profile.language", "Rust")
                .with_confidence(0.8),
        ];

        let detector = SlotValueDetector::new();
        let findings = detector
            .detect_contradictions(&mem, "person:test", &events)
            .await
            .unwrap();

        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.slot_key.as_str(), "profile.language");
        assert_eq!(finding.existing_value, "Python");
        assert_eq!(finding.new_value, "Rust");
        assert_eq!(
            finding.contradiction_confidence,
            crate::contracts::scores::Confidence::new(0.8)
        );
    }

    #[tokio::test]
    async fn ignores_equal_value_after_normalization() {
        let tmp = TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());

        mem.append_event(MemoryEventInput::new(
            "person:test",
            "profile.language",
            MemoryEventType::FactAdded,
            "Rust",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .unwrap();

        let events = vec![
            MemoryInferenceEvent::inferred_claim("person:test", "profile.language", "  rUsT  ")
                .with_confidence(0.9),
        ];

        let detector = SlotValueDetector::new();
        let findings = detector
            .detect_contradictions(&mem, "person:test", &events)
            .await
            .unwrap();

        assert!(findings.is_empty());
    }

    #[cfg(feature = "postgres")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn ignores_existing_slot_below_confidence_threshold() {
        use std::sync::Arc;
        use std::time::Duration;

        use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
        use crate::core::memory::postgres::{PostgresConnectOptions, PostgresMemory};

        struct StubEmbedding;

        impl EmbeddingProvider for StubEmbedding {
            fn name(&self) -> &'static str {
                "stub_consistency_test"
            }
            fn dimensions(&self) -> usize {
                3
            }
            fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
                Box::pin(async move { Ok(texts.iter().map(|_| vec![0.0, 0.0, 0.0]).collect()) })
            }
        }

        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let database_url = std::env::var("ASTEREL_POSTGRES_URL").expect("postgres url must be set");
        let mem = PostgresMemory::connect_with_options(
            &database_url,
            Arc::new(StubEmbedding),
            PostgresConnectOptions {
                cache_max: 16,
                graph_retrieval_fusion_enabled: false,
                graph_retrieval_weight: 0.0,
                max_connections: 4,
                min_connections: 1,
                connect_timeout: Duration::from_secs(5),
                idle_timeout: Duration::from_secs(30),
                vector_weight: 0.7,
                keyword_weight: 0.3,
                max_lifetime: Duration::from_secs(60),
                hnsw_ef_search: 0,
            },
        )
        .await
        .expect("connect postgres memory");

        let entity_id = format!("person:consistency-{}", uuid::Uuid::new_v4().simple());

        mem.append_event(
            MemoryEventInput::new(
                &entity_id,
                "profile.language",
                MemoryEventType::FactAdded,
                "Python",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.2),
        )
        .await
        .unwrap();

        let events = vec![
            MemoryInferenceEvent::inferred_claim(entity_id.as_str(), "profile.language", "Rust")
                .with_confidence(0.8),
        ];

        let detector = SlotValueDetector::with_min_existing_confidence(0.5);
        let findings = detector
            .detect_contradictions(&mem, &entity_id, &events)
            .await
            .unwrap();

        assert!(
            findings.is_empty(),
            "existing slot confidence 0.2 is below threshold 0.5; should not produce findings, got: {findings:?}"
        );
    }
}
