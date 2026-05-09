//! Memory review read models for operator inspection and correction.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, EventId, SlotKey};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntitySummaryReadModel {
    pub entity_id: EntityId,
    pub slot_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntityListReadModel {
    pub items: Vec<MemoryEntitySummaryReadModel>,
    pub count: usize,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySlotProvenanceReadModel {
    pub source_class: String,
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySlotSummaryReadModel {
    pub slot_key: SlotKey,
    pub value: String,
    pub source: String,
    pub confidence: f64,
    pub importance: f64,
    pub privacy_level: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<MemorySlotProvenanceReadModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySlotListReadModel {
    pub entity_id: EntityId,
    pub items: Vec<MemorySlotSummaryReadModel>,
    pub count: usize,
    pub event_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCorrectionReadModel {
    pub status: String,
    pub entity_id: EntityId,
    pub slot_key: SlotKey,
    pub event_id: EventId,
}

#[must_use]
pub fn build_memory_entity_list_read_model(
    backend: String,
    items: Vec<MemoryEntitySummaryReadModel>,
) -> MemoryEntityListReadModel {
    MemoryEntityListReadModel {
        count: items.len(),
        items,
        backend,
    }
}

#[must_use]
pub fn build_memory_slot_list_read_model(
    entity_id: EntityId,
    event_count: usize,
    items: Vec<MemorySlotSummaryReadModel>,
) -> MemorySlotListReadModel {
    MemorySlotListReadModel {
        entity_id,
        count: items.len(),
        event_count,
        items,
    }
}

#[must_use]
pub fn build_memory_correction_read_model(
    entity_id: EntityId,
    slot_key: SlotKey,
    event_id: EventId,
) -> MemoryCorrectionReadModel {
    MemoryCorrectionReadModel {
        status: "corrected".to_string(),
        entity_id,
        slot_key,
        event_id,
    }
}
