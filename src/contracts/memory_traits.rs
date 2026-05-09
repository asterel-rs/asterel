//! Core memory traits shared by all backends.

use std::future::Future;
use std::pin::Pin;

use crate::contracts::memory::MemoryEventInput;
use crate::contracts::memory::MemoryProvenance;
use crate::contracts::memory_domain::{
    BeliefSlot, MemoryEvent, MemoryInferenceEvent, MemoryIntegrityReport, MemoryRecallEntry,
    RecallQuery,
};
use crate::contracts::memory_error::{MemoryError, MemoryResult};
use crate::contracts::memory_forget::{ForgetMode, ForgetOutcome};

/// Write-path operations: appending events and inference payloads.
pub trait MemoryWriter: Send + Sync {
    /// Append a raw memory event to the backend.
    ///
    /// # Errors
    /// Returns an error if the event cannot be persisted.
    fn append_event(
        &self,
        input: MemoryEventInput,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryEvent>> + Send + '_>>;

    /// Append a single inference event (inferred claim or contradiction).
    ///
    /// # Errors
    /// Returns an error if the event cannot be persisted.
    fn append_inference_event(
        &self,
        event: MemoryInferenceEvent,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryEvent>> + Send + '_>> {
        Box::pin(async move { self.append_event(event.into_memory_event_input()).await })
    }

    /// Append multiple inference events in sequence.
    ///
    /// # Errors
    /// Returns an error if any event in the batch cannot be persisted.
    fn append_inference_events(
        &self,
        events: Vec<MemoryInferenceEvent>,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryEvent>>> + Send + '_>> {
        Box::pin(async move {
            let mut persisted = Vec::with_capacity(events.len());
            for event in events {
                persisted.push(self.append_inference_event(event).await?);
            }
            Ok(persisted)
        })
    }
}

/// Read-path operations: recall and slot resolution.
pub trait MemoryReader: Send + Sync {
    /// Recall memory items matching a scoped query.
    ///
    /// # Errors
    /// Returns an error if the backend query fails.
    fn recall_scoped(
        &self,
        query: RecallQuery,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryRecallEntry>>> + Send + '_>>;

    /// Recall memory items with phased retrieval (defaults to scoped).
    ///
    /// # Errors
    /// Returns an error if the backend query fails.
    fn recall_phased(
        &self,
        query: RecallQuery,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryRecallEntry>>> + Send + '_>> {
        Box::pin(async move { self.recall_scoped(query).await })
    }

    /// Resolve a single belief slot to its current value.
    ///
    /// # Errors
    /// Returns an error if the backend lookup fails.
    fn resolve_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Option<BeliefSlot>>> + Send + 'a>>;
}

/// Governance operations: deletion, counting, integrity, diagnostics.
pub trait MemoryGovernance: Send + Sync {
    /// Return the backend name (e.g. `"postgres"`, `"markdown"`).
    fn name(&self) -> &str;

    /// Check if the backend is healthy and reachable.
    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;

    /// Forget a slot using the specified deletion mode.
    ///
    /// # Errors
    /// Returns an error if the forget operation fails.
    fn forget_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
        mode: ForgetMode,
        reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<ForgetOutcome>> + Send + 'a>>;

    /// Count persisted events, optionally filtered by entity.
    ///
    /// # Errors
    /// Returns an error if the count query fails.
    fn count_events<'a>(
        &'a self,
        entity_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<usize>> + Send + 'a>>;

    /// List entity identifiers currently known to the backend.
    fn list_entities(
        &self,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<String>>> + Send + '_>> {
        Box::pin(async move {
            Err(MemoryError::unsupported(format!(
                "memory backend '{}' does not support entity listing",
                self.name()
            )))
        })
    }

    /// List current belief slots for a specific entity.
    fn list_slots<'a>(
        &'a self,
        entity_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<BeliefSlot>>> + Send + 'a>> {
        Box::pin(async move {
            Err(MemoryError::unsupported(format!(
                "memory backend '{}' does not support slot listing for entity '{}'",
                self.name(),
                entity_id
            )))
        })
    }

    /// Resolve provenance metadata for the current value of a slot when available.
    fn slot_provenance<'a>(
        &'a self,
        _entity_id: &'a str,
        _slot_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Option<MemoryProvenance>>> + Send + 'a>> {
        Box::pin(async move { Ok(None) })
    }

    /// Verify the integrity of hash chains in the event ledger.
    ///
    /// # Errors
    /// Returns an error if integrity verification is unsupported.
    fn verify_integrity(
        &self,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryIntegrityReport>> + Send + '_>> {
        Box::pin(async move {
            Err(MemoryError::unsupported(format!(
                "memory backend '{}' does not support integrity verification",
                self.name()
            )))
        })
    }

    /// Fraction of retrieval units with a non-zero contradiction penalty (0.0-1.0).
    ///
    /// Backends that do not track contradiction penalties return `Ok(0.0)`.
    fn contradiction_ratio(&self) -> Pin<Box<dyn Future<Output = MemoryResult<f64>> + Send + '_>> {
        Box::pin(async move { Ok(0.0) })
    }
}

/// Unified memory contract combining write, read, and governance operations.
pub trait Memory: MemoryWriter + MemoryReader + MemoryGovernance {}

impl<T: MemoryWriter + MemoryReader + MemoryGovernance> Memory for T {}
