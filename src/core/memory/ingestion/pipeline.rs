//! Default ingestion pipeline implementation.
//!
//! The [`DefaultIngestPipeline`] processes [`SignalEnvelope`] payloads through
//! the following steps:
//!
//! 1. **Write-policy enforcement** — `enforce_ingestion_write_policy` rejects
//!    signals that violate autonomy or privacy constraints before any I/O.
//! 2. **Semantic deduplication** — a bounded in-memory SHA-256 content hash
//!    cache ([`SemanticDedupCache`]) suppresses duplicate signals within a
//!    session without a database round-trip.
//! 3. **Persistence** — accepted envelopes are written to the memory backend
//!    via `Memory::remember`.
//! 4. **Observability** — per-signal and per-batch metrics are emitted to the
//!    configured [`Observer`].

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::error::{IngestionError, IngestionPipelineResult};
use super::signal_envelope::SignalEnvelope;
use crate::contracts::ids::SlotKey;
use crate::contracts::observability::NoopObserver;
use crate::contracts::observability::{AutonomySignal, Observer, ObserverMetric};
use crate::core::memory::memory_types::{
    MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, RecallQuery, SignalTier,
    SourceKind,
};
use crate::core::memory::traits::{Memory, MemoryLayer};
use crate::security::writeback_guard::enforce_ingestion_write_policy;

/// Outcome of ingesting a single signal envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionResult {
    /// Whether the signal was accepted into memory.
    pub accepted: bool,
    /// Slot key the signal was stored under.
    pub slot_key: SlotKey,
    /// Signal tier of the ingested envelope.
    pub signal_tier: SignalTier,
    /// Reason for rejection, if not accepted.
    pub reason: Option<String>,
}

/// Trait for ingesting external signals into memory.
// async_fn_in_trait: this trait is crate-internal and always used via Arc<dyn IngestionPipeline>
// with Send + Sync bounds; explicit impl Future<Output = ...> + Send syntax adds noise for no gain.
#[allow(async_fn_in_trait)]
pub trait IngestionPipeline: Send + Sync {
    /// Ingest a single signal envelope.
    ///
    /// # Errors
    ///
    /// Returns an error if the envelope cannot be persisted.
    async fn ingest(&self, envelope: SignalEnvelope) -> IngestionPipelineResult<IngestionResult>;

    /// Ingest a batch of envelopes sequentially.
    ///
    /// # Errors
    ///
    /// Returns an error if any envelope in the batch fails.
    async fn ingest_batch(
        &self,
        envelopes: Vec<SignalEnvelope>,
    ) -> IngestionPipelineResult<Vec<IngestionResult>> {
        let mut results = Vec::with_capacity(envelopes.len());
        for envelope in envelopes {
            results.push(self.ingest(envelope).await?);
        }
        Ok(results)
    }
}

/// Default pipeline: validates, deduplicates, and persists signals.
#[derive(Clone)]
pub struct DefaultIngestPipeline {
    memory: Arc<dyn Memory>,
    semantic_dedup_cache: Arc<Mutex<SemanticDedupCache>>,
    observer: Arc<dyn Observer>,
}

const DEFAULT_SEMANTIC_DEDUP_CACHE_MAX: usize = 20_000;

/// Bounded in-process cache of SHA-256 content hashes for deduplication.
///
/// Maintains insertion order so the oldest entries are evicted first when
/// the cache exceeds `max_entries` (default: [`DEFAULT_SEMANTIC_DEDUP_CACHE_MAX`]).
/// Prevents duplicate signals from being written during a session without
/// requiring a database query.
#[derive(Debug)]
struct SemanticDedupCache {
    entries: HashSet<String>,
    insertion_order: VecDeque<String>,
    max_entries: usize,
}

impl SemanticDedupCache {
    fn new(max_entries: usize) -> Self {
        Self {
            entries: HashSet::new(),
            insertion_order: VecDeque::new(),
            max_entries: max_entries.max(1),
        }
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.contains(key)
    }

    fn insert(&mut self, key: String) {
        if self.entries.contains(&key) {
            return;
        }

        self.insertion_order.push_back(key.clone());
        self.entries.insert(key);

        while self.entries.len() > self.max_entries {
            if let Some(evicted) = self.insertion_order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }
    }
}

impl DefaultIngestPipeline {
    /// Create a pipeline with no-op observer.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self::new_with_observer(memory, Arc::new(NoopObserver))
    }

    /// Create a pipeline with a custom observer for metrics.
    pub fn new_with_observer(memory: Arc<dyn Memory>, observer: Arc<dyn Observer>) -> Self {
        Self {
            memory,
            semantic_dedup_cache: Arc::new(Mutex::new(SemanticDedupCache::new(
                DEFAULT_SEMANTIC_DEDUP_CACHE_MAX,
            ))),
            observer,
        }
    }

    async fn is_source_ref_exact_duplicate(
        &self,
        envelope: &SignalEnvelope,
        slot_key: &str,
    ) -> IngestionPipelineResult<bool> {
        let existing = self
            .memory
            .resolve_slot(envelope.entity_id.as_str(), slot_key)
            .await?;
        Ok(existing.is_some_and(|slot| slot.value == envelope.content))
    }

    async fn is_semantic_duplicate(
        &self,
        envelope: &SignalEnvelope,
        slot_key: &str,
    ) -> IngestionPipelineResult<bool> {
        let source_kind_prefix = format!("external.{}.", envelope.source_kind_str());
        let semantic_candidates = self
            .memory
            .recall_scoped(RecallQuery::new(
                envelope.entity_id.clone(),
                &envelope.content,
                5,
            ))
            .await?;

        Ok(semantic_candidates.iter().any(|item| {
            if item.slot_key.as_str() == slot_key
                || !item.slot_key.as_str().starts_with(&source_kind_prefix)
            {
                return false;
            }
            // Exact content match is an unambiguous duplicate.
            if item.value == envelope.content {
                return true;
            }
            // High recall-score matches (≥ 0.95) are treated as semantic
            // duplicates ONLY if the two values have near-identical length.
            // Without this length check, a keyword-overlap-based backend
            // (e.g. MarkdownMemory) flags any stored value whose keywords
            // are a subset of the new value — for example, "release pulse
            // signal" vs "release pulse signal on x" scores 1.0 because
            // all three stored keywords appear in the new content, even
            // though the new content carries additional information. The
            // length ratio rules out the "stored ⊆ new" case while still
            // catching true near-duplicates that differ only in
            // punctuation or whitespace.
            #[allow(clippy::cast_precision_loss)]
            let len_new = envelope.content.chars().count() as f64;
            #[allow(clippy::cast_precision_loss)]
            let len_stored = item.value.chars().count() as f64;
            let len_ratio = len_new.min(len_stored) / len_new.max(len_stored).max(1.0);
            item.score >= 0.95 && len_ratio >= 0.85
        }))
    }

    fn dedup_cache_contains(&self, semantic_key: &str) -> IngestionPipelineResult<bool> {
        let cache = self.semantic_dedup_cache.lock().map_err(|e| {
            IngestionError::state(format!("semantic dedup cache lock poisoned: {e}"))
        })?;
        Ok(cache.contains(semantic_key))
    }

    fn dedup_cache_insert(&self, semantic_key: String) -> IngestionPipelineResult<()> {
        let mut cache = self.semantic_dedup_cache.lock().map_err(|e| {
            IngestionError::state(format!("semantic dedup cache lock poisoned: {e}"))
        })?;
        cache.insert(semantic_key);
        Ok(())
    }
}

/// Compute a SHA-256 deduplication key from entity, source kind,
/// and content.
pub(super) fn semantic_dedup_key(
    entity_id: &str,
    source_kind: SourceKind,
    content: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(entity_id.as_bytes());
    hasher.update(b"::");
    hasher.update(source_kind.to_string().as_bytes());
    hasher.update(b"::");
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

impl DefaultIngestPipeline {
    fn source_class_for(kind: SourceKind) -> MemorySource {
        match kind {
            SourceKind::Conversation | SourceKind::Manual => MemorySource::ExplicitUser,
            SourceKind::Discord | SourceKind::Telegram | SourceKind::Slack => {
                MemorySource::ExternalPrimary
            }
            SourceKind::Api | SourceKind::News | SourceKind::Document => {
                MemorySource::ExternalSecondary
            }
        }
    }

    async fn check_dedup(
        &self,
        envelope: &SignalEnvelope,
        slot_key: &str,
        semantic_key: &str,
        source_kind_label: &str,
    ) -> IngestionPipelineResult<Option<String>> {
        if envelope.signal_tier != SignalTier::Raw {
            return Ok(None);
        }

        if self
            .is_source_ref_exact_duplicate(envelope, slot_key)
            .await?
        {
            self.record_dedup(source_kind_label);
            return Ok(Some("dedup:source_ref_exact".to_string()));
        }

        if self.is_semantic_duplicate(envelope, slot_key).await? {
            self.dedup_cache_insert(semantic_key.to_string())?;
            self.record_dedup(source_kind_label);
            return Ok(Some("dedup:semantic_similar".to_string()));
        }

        if self.dedup_cache_contains(semantic_key)? {
            self.record_dedup(source_kind_label);
            return Ok(Some("dedup:semantic_similar".to_string()));
        }

        Ok(None)
    }

    fn record_dedup(&self, source_kind_label: &str) {
        self.observer
            .emit_autonomy_signal(AutonomySignal::Deduplicated);
        self.observer
            .record_metric(&ObserverMetric::SignalDedupDropTotal {
                source_kind: source_kind_label.to_string(),
            });
    }
}

impl IngestionPipeline for DefaultIngestPipeline {
    async fn ingest(&self, envelope: SignalEnvelope) -> IngestionPipelineResult<IngestionResult> {
        let envelope = envelope.normalize()?;
        let source_class = Self::source_class_for(envelope.source_kind);

        let slot_key = format!(
            "external.{}.{}",
            envelope.source_kind_str(),
            envelope.source_ref
        );
        let source_kind_label = envelope.source_kind_str();
        let semantic_key = semantic_dedup_key(
            envelope.entity_id.as_str(),
            envelope.source_kind,
            &envelope.content,
        );

        if let Some(reason) = self
            .check_dedup(&envelope, &slot_key, &semantic_key, &source_kind_label)
            .await?
        {
            return Ok(IngestionResult {
                accepted: false,
                slot_key: SlotKey::new(slot_key),
                signal_tier: envelope.signal_tier,
                reason: Some(reason),
            });
        }

        let source_ref = envelope.source_ref;

        let input = MemoryEventInput::new(
            envelope.entity_id,
            &slot_key,
            MemoryEventType::FactAdded,
            envelope.content,
            source_class,
            envelope.privacy_level,
        )
        .with_signal_tier(envelope.signal_tier)
        .with_source_kind(envelope.source_kind)
        .with_source_ref(&source_ref)
        .with_layer(MemoryLayer::Working)
        .with_importance(0.4)
        .with_provenance(MemoryProvenance::source_reference(
            source_class,
            format!("ingestion:{source_ref}"),
        ));

        enforce_ingestion_write_policy(&input)
            .map_err(|error| IngestionError::policy(error.to_string()))?;

        self.memory.append_event(input).await?;
        self.dedup_cache_insert(semantic_key)?;
        self.observer.emit_autonomy_signal(AutonomySignal::Ingested);
        self.observer
            .record_metric(&ObserverMetric::SignalIngestTotal {
                source_kind: source_kind_label,
            });

        Ok(IngestionResult {
            accepted: true,
            slot_key: SlotKey::new(slot_key),
            signal_tier: envelope.signal_tier,
            reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SemanticDedupCache;

    #[test]
    fn semantic_dedup_cache_evicts_oldest_entry_at_capacity() {
        let mut cache = SemanticDedupCache::new(2);
        cache.insert("first".to_string());
        cache.insert("second".to_string());
        cache.insert("third".to_string());

        assert!(!cache.contains("first"));
        assert!(cache.contains("second"));
        assert!(cache.contains("third"));
    }

    #[test]
    fn semantic_dedup_cache_ignores_duplicate_insertions() {
        let mut cache = SemanticDedupCache::new(2);
        cache.insert("same".to_string());
        cache.insert("same".to_string());
        cache.insert("new".to_string());

        assert!(cache.contains("same"));
        assert!(cache.contains("new"));
    }
}
