//! Canonical memory domain types shared across backends.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, EventId, SessionId, SlotKey};
use crate::contracts::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource, PrivacyLevel,
};
use crate::contracts::memory_error::{MemoryError, MemoryResult};
use crate::contracts::memory_forget::ForgetMode;
use crate::contracts::scores::{Confidence, Importance};
use crate::contracts::tenant::TenantPolicyContext;

/// A single memory entry stored in a backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique identifier for this entry.
    pub id: String,
    /// Lookup key (e.g. entity or slot key).
    pub key: String,
    /// Textual content of the memory entry.
    pub content: String,
    /// Organizational category (core, daily, conversation, custom).
    pub category: MemoryCategory,
    /// ISO-8601 timestamp of when this entry was created.
    pub timestamp: String,
    /// Session that produced this entry, if any.
    pub session_id: Option<SessionId>,
    /// Relevance score assigned during recall, if any.
    pub score: Option<f64>,
    /// Source class extracted from provenance metadata (if available).
    #[serde(default)]
    pub source: Option<MemorySource>,
    /// Storage layer extracted from backend metadata, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<MemoryLayer>,
}

const fn default_inferred_claim_layer() -> MemoryLayer {
    MemoryLayer::Semantic
}

const fn default_contradiction_layer() -> MemoryLayer {
    MemoryLayer::Episodic
}

/// Persisted memory event in the append-only ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEvent {
    /// Unique event identifier (UUID).
    pub event_id: EventId,
    /// Entity this event belongs to (e.g. `person:alice`).
    pub entity_id: EntityId,
    /// Taxonomy-scoped slot key (e.g. `preference.food`).
    pub slot_key: SlotKey,
    /// Classification of the event (fact added, retracted, etc.).
    pub event_type: MemoryEventType,
    /// Textual payload of the event.
    pub value: String,
    /// How this event was sourced (explicit, inferred, system).
    pub source: MemorySource,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: Importance,
    /// Provenance metadata, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<MemoryProvenance>,
    /// Privacy classification for access control.
    pub privacy_level: PrivacyLevel,
    /// When the event originally occurred (ISO-8601).
    pub occurred_at: String,
    /// When the event was ingested into the ledger (ISO-8601).
    pub ingested_at: String,
}

/// Higher-level inferred events converted into `MemoryEventInput`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryInferenceEvent {
    /// A claim inferred from conversation or analysis.
    InferredClaim {
        entity_id: EntityId,
        slot_key: SlotKey,
        layer: MemoryLayer,
        value: String,
        confidence: Confidence,
        importance: Importance,
        privacy_level: PrivacyLevel,
        occurred_at: String,
    },
    /// A contradiction detected against an existing claim.
    ContradictionEvent {
        entity_id: EntityId,
        slot_key: SlotKey,
        layer: MemoryLayer,
        value: String,
        confidence: Confidence,
        importance: Importance,
        privacy_level: PrivacyLevel,
        occurred_at: String,
    },
}

impl MemoryInferenceEvent {
    /// Build an inferred-claim event with defaults.
    pub fn inferred_claim(
        entity_id: impl Into<EntityId>,
        slot_key: impl Into<SlotKey>,
        value: impl Into<String>,
    ) -> Self {
        Self::InferredClaim {
            entity_id: entity_id.into(),
            slot_key: slot_key.into(),
            layer: default_inferred_claim_layer(),
            value: value.into(),
            confidence: Confidence::new(0.7),
            importance: Importance::new(0.5),
            privacy_level: PrivacyLevel::Private,
            occurred_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Build a contradiction-marked event with defaults.
    pub fn contradiction_marked(
        entity_id: impl Into<EntityId>,
        slot_key: impl Into<SlotKey>,
        value: impl Into<String>,
    ) -> Self {
        Self::ContradictionEvent {
            entity_id: entity_id.into(),
            slot_key: slot_key.into(),
            layer: default_contradiction_layer(),
            value: value.into(),
            confidence: Confidence::new(0.85),
            importance: Importance::new(0.8),
            privacy_level: PrivacyLevel::Private,
            occurred_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Override confidence, clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        match &mut self {
            Self::InferredClaim {
                confidence: current,
                ..
            }
            | Self::ContradictionEvent {
                confidence: current,
                ..
            } => {
                *current = Confidence::new(confidence);
            }
        }
        self
    }

    /// Override importance, clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn with_importance(mut self, importance: f64) -> Self {
        match &mut self {
            Self::InferredClaim {
                importance: current,
                ..
            }
            | Self::ContradictionEvent {
                importance: current,
                ..
            } => {
                *current = Importance::new(importance);
            }
        }
        self
    }

    /// Override privacy level.
    #[must_use]
    pub fn with_privacy_level(mut self, privacy_level: PrivacyLevel) -> Self {
        match &mut self {
            Self::InferredClaim {
                privacy_level: current,
                ..
            }
            | Self::ContradictionEvent {
                privacy_level: current,
                ..
            } => {
                *current = privacy_level;
            }
        }
        self
    }

    /// Override occurrence timestamp.
    #[must_use]
    pub fn with_occurred_at(mut self, occurred_at: impl Into<String>) -> Self {
        let occurred_at = occurred_at.into();
        match &mut self {
            Self::InferredClaim {
                occurred_at: current,
                ..
            }
            | Self::ContradictionEvent {
                occurred_at: current,
                ..
            } => {
                *current = occurred_at;
            }
        }
        self
    }

    /// Override target layer.
    #[must_use]
    pub fn with_layer(mut self, layer: MemoryLayer) -> Self {
        match &mut self {
            Self::InferredClaim { layer: current, .. }
            | Self::ContradictionEvent { layer: current, .. } => {
                *current = layer;
            }
        }
        self
    }

    /// Convert into a concrete appendable memory event input payload.
    #[must_use]
    pub fn into_memory_event_input(self) -> MemoryEventInput {
        match self {
            Self::InferredClaim {
                entity_id,
                slot_key,
                layer,
                value,
                confidence,
                importance,
                privacy_level,
                occurred_at,
            } => MemoryEventInput {
                entity_id,
                slot_key,
                layer,
                event_type: MemoryEventType::InferredClaim,
                value,
                source: MemorySource::Inferred,
                confidence,
                importance,
                provenance: None,
                signal_tier: None,
                source_kind: None,
                source_ref: None,
                privacy_level,
                occurred_at,
                emotion_label: None,
                emotion_valence: None,
                emotion_arousal: None,
                emotion_confidence: None,
            },
            Self::ContradictionEvent {
                entity_id,
                slot_key,
                layer,
                value,
                confidence,
                importance,
                privacy_level,
                occurred_at,
            } => MemoryEventInput {
                entity_id,
                slot_key,
                layer,
                event_type: MemoryEventType::ContradictionMarked,
                value,
                source: MemorySource::System,
                confidence,
                importance,
                provenance: None,
                signal_tier: None,
                source_kind: None,
                source_ref: None,
                privacy_level,
                occurred_at,
                emotion_label: None,
                emotion_valence: None,
                emotion_arousal: None,
                emotion_confidence: None,
            },
        }
    }
}

/// Query payload for recall/search operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallQuery {
    /// Target entity to recall memories for.
    pub entity_id: EntityId,
    /// Free-text query for semantic search.
    pub query: String,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Tenant-scoped policy context for access control.
    #[serde(default)]
    pub policy_context: TenantPolicyContext,
    /// Optional layer restriction for identity/procedural/working read paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_filter: Option<MemoryLayer>,
}

impl RecallQuery {
    /// Construct a recall query with default policy context.
    pub fn new(entity_id: impl AsRef<str>, query: impl Into<String>, limit: usize) -> Self {
        Self {
            entity_id: EntityId::new(entity_id.as_ref()),
            query: query.into(),
            limit,
            policy_context: TenantPolicyContext::default(),
            layer_filter: None,
        }
    }

    /// Attach tenant policy context for scoped recall.
    #[must_use]
    pub fn with_policy_context(mut self, policy_context: TenantPolicyContext) -> Self {
        self.policy_context = policy_context;
        self
    }

    /// Restrict recall to a specific memory layer.
    #[must_use]
    pub fn with_layer_filter(mut self, layer: MemoryLayer) -> Self {
        self.layer_filter = Some(layer);
        self
    }

    /// # Errors
    /// Returns an error if tenant recall scope policy rejects the query.
    pub fn enforce_policy(&self) -> MemoryResult<()> {
        self.policy_context
            .enforce_recall_scope(self.entity_id.as_str())
            .map_err(MemoryError::policy)
    }
}

/// Scored recall result item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallEntry {
    /// Entity this item belongs to.
    pub entity_id: EntityId,
    /// Taxonomy-scoped slot key.
    pub slot_key: SlotKey,
    /// Resolved textual value.
    pub value: String,
    /// How this item was originally sourced.
    pub source: MemorySource,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: Importance,
    /// Privacy classification.
    pub privacy_level: PrivacyLevel,
    /// Relevance score from the recall query.
    pub score: f64,
    /// When the underlying event originally occurred (ISO-8601).
    pub occurred_at: String,
}

/// Single integrity issue discovered during ledger verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryIntegrityIssue {
    /// Which hash chain the issue was found in.
    pub chain: String,
    /// Row key of the affected record.
    pub row_key: String,
    /// Human-readable description of the integrity violation.
    pub reason: String,
}

/// Integrity check report for a memory backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryIntegrityReport {
    /// Backend name that was verified.
    pub backend: String,
    /// Whether the integrity check passed without issues.
    pub is_verified: bool,
    /// Number of memory event rows verified.
    pub checked_memory_events: usize,
    /// Number of deletion ledger rows verified.
    pub checked_deletion_ledger: usize,
    /// List of integrity issues found, if any.
    pub issues: Vec<MemoryIntegrityIssue>,
}

/// Graph entity node classification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum GraphEntityType {
    /// A person or user entity.
    Person,
    /// A knowledge slot node.
    Slot,
    /// A discrete event node.
    Event,
}

/// Graph edge relation classification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum GraphRelationType {
    /// Entity owns this slot.
    HasSlot,
    /// Entity recorded this event.
    RecordedEvent,
    /// A newer value supersedes an older one.
    Supersedes,
    /// A value was contradicted by another.
    ContradictedBy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeTier {
    Episode,
    #[default]
    Note,
}

/// Materialized graph entity representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEntity {
    /// Unique graph-level identifier.
    pub graph_entity_id: EntityId,
    /// Entity that owns this graph node.
    pub owner_entity_id: EntityId,
    /// Classification of this graph node.
    pub entity_type: GraphEntityType,
    /// Human-readable label.
    pub label: String,
    /// Resolved value payload.
    pub value: String,
    /// How this entity was sourced.
    pub source: MemorySource,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: Importance,
    pub access_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<String>,
    #[serde(default)]
    pub is_pinned: bool,
    pub temporal_decay_score: f64,
    #[serde(default)]
    pub node_tier: NodeTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_graph_entity_id: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_at: Option<String>,
    /// Privacy classification.
    pub privacy_level: PrivacyLevel,
    /// Last update timestamp (ISO-8601).
    pub updated_at: String,
}

/// Materialized graph edge representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Unique graph-level edge identifier.
    pub graph_edge_id: String,
    /// Entity that owns this edge.
    pub owner_entity_id: EntityId,
    /// Source node of the relation.
    pub from_entity_id: EntityId,
    /// Target node of the relation.
    pub to_entity_id: EntityId,
    /// Type of relation between the nodes.
    pub relation_type: GraphRelationType,
    /// Edge weight (e.g. confidence or strength).
    pub weight: f64,
    /// Originating event identifier, if any.
    pub event_id: Option<String>,
    /// Creation timestamp (ISO-8601).
    pub created_at: String,
}

/// Consolidated belief slot view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefSlot {
    /// Entity this belief belongs to.
    pub entity_id: EntityId,
    /// Taxonomy-scoped slot key.
    pub slot_key: SlotKey,
    /// Current resolved value.
    pub value: String,
    /// How this belief was sourced.
    pub source: MemorySource,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: Importance,
    /// Privacy classification.
    pub privacy_level: PrivacyLevel,
    /// Last update timestamp (ISO-8601).
    pub updated_at: String,
}

/// Support level for a backend capability.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    /// Fully supported with complete semantics.
    Supported,
    /// Partially supported with reduced guarantees.
    Degraded,
    /// Not supported by this backend.
    Unsupported,
}

/// Backend capability matrix for forget-mode support.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCapMatrix {
    /// Backend name (e.g. `"postgres"`, `"markdown"`).
    pub backend: &'static str,
    /// Support level for soft-delete forget mode.
    pub forget_soft: CapabilitySupport,
    /// Support level for hard-delete forget mode.
    pub forget_hard: CapabilitySupport,
    /// Support level for tombstone forget mode.
    pub forget_tombstone: CapabilitySupport,
    /// Contract text returned when a mode is unsupported.
    pub unsupported_contract: &'static str,
}

impl MemoryCapMatrix {
    /// Return support level for the given forget mode.
    #[must_use]
    pub fn support_for_forget_mode(&self, mode: ForgetMode) -> CapabilitySupport {
        match mode {
            ForgetMode::Soft => self.forget_soft,
            ForgetMode::Hard => self.forget_hard,
            ForgetMode::Tombstone => self.forget_tombstone,
        }
    }

    /// # Errors
    /// Returns an error if the requested forget mode is unsupported by this backend.
    pub fn require_forget_mode(&self, mode: ForgetMode) -> MemoryResult<()> {
        if self.support_for_forget_mode(mode) == CapabilitySupport::Unsupported {
            let mode = match mode {
                ForgetMode::Soft => "soft",
                ForgetMode::Hard => "hard",
                ForgetMode::Tombstone => "tombstone",
            };
            return Err(MemoryError::unsupported(format!(
                "memory backend '{}' does not support forget mode '{}'",
                self.backend, mode
            )));
        }
        Ok(())
    }
}

/// Memory categories for organization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MemoryCategory {
    /// Long-term facts, preferences, decisions
    Core,
    /// Daily session logs
    Daily,
    /// Conversation context
    Conversation,
    /// User-defined custom category
    #[strum(to_string = "{0}")]
    Custom(String),
}

impl MemoryCategory {
    /// Create a custom category name with conservative sanitization.
    pub fn custom(name: impl Into<String>) -> Self {
        let name = name.into();
        let sanitized: String = name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .take(128)
            .collect();
        Self::Custom(sanitized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_category_sanitizes_special_chars() {
        let result = MemoryCategory::custom("hello'; DROP TABLE");
        match result {
            MemoryCategory::Custom(name) => {
                assert_eq!(name, "helloDROPTABLE");
                assert!(!name.contains('\''));
                assert!(!name.contains(';'));
            }
            _ => panic!("Expected Custom variant"),
        }
    }

    #[test]
    fn custom_category_preserves_valid_chars() {
        let result = MemoryCategory::custom("my_custom-category.v1");
        match result {
            MemoryCategory::Custom(name) => {
                assert_eq!(name, "my_custom-category.v1");
            }
            _ => panic!("Expected Custom variant"),
        }
    }

    #[test]
    fn custom_category_caps_length() {
        let long_name = "a".repeat(200);
        let result = MemoryCategory::custom(&long_name);
        match result {
            MemoryCategory::Custom(name) => {
                assert!(
                    name.len() <= 128,
                    "Name should be capped at 128 chars, got {}",
                    name.len()
                );
            }
            _ => panic!("Expected Custom variant"),
        }
    }
}
