//! Memory layer contracts shared between `security`, `core`, and `config`.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, SlotKey};
use crate::contracts::scores::{Confidence, Importance};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLayer {
    Working,
    Episodic,
    Semantic,
    Procedural,
    Identity,
}

/// Provenance class of a memory event/value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// Directly provided by the user.
    ExplicitUser,
    /// Verified by a trusted tool.
    ToolVerified,
    /// Produced by internal system components.
    System,
    /// Inferred from existing memories/context.
    Inferred,
    /// External high-confidence source.
    ExternalPrimary,
    /// External low-confidence source.
    ExternalSecondary,
}

impl MemorySource {
    /// Default confidence assigned when a source is first recorded.
    #[must_use]
    pub const fn default_confidence(self) -> f64 {
        match self {
            Self::ExplicitUser => 0.95,
            Self::ToolVerified => 0.9,
            Self::System => 0.8,
            Self::Inferred => 0.7,
            Self::ExternalPrimary => 0.75,
            Self::ExternalSecondary => 0.5,
        }
    }
}

/// Provenance metadata attached to memory records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProvenance {
    /// Classification of the memory source.
    pub source_class: MemorySource,
    /// Human-readable reference describing the origin.
    pub reference: String,
    /// Optional URI linking to supporting evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_uri: Option<String>,
}

impl MemoryProvenance {
    /// Build provenance from source class and source reference string.
    pub fn source_reference(source_class: MemorySource, reference: impl Into<String>) -> Self {
        Self {
            source_class,
            reference: reference.into(),
            evidence_uri: None,
        }
    }

    /// Attach optional evidence URI (e.g., document or message permalink).
    #[must_use]
    pub fn with_evidence_uri(mut self, evidence_uri: impl Into<String>) -> Self {
        self.evidence_uri = Some(evidence_uri.into());
        self
    }
}

/// Privacy tier assigned to memory values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyLevel {
    /// Can be surfaced broadly.
    Public,
    /// Default private memory.
    Private,
    /// Sensitive memory requiring stricter handling.
    Secret,
}

/// Ingestion signal tier used for policy/governance decisions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SignalTier {
    /// Unprocessed raw signal from a source.
    Raw,
    /// Signal elevated to a belief state.
    Belief,
    /// Signal derived by inference from other data.
    Inferred,
    /// Signal relevant to governance/policy decisions.
    Governance,
}

/// Origin domain for a memory signal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SourceKind {
    /// Interactive conversation turn.
    Conversation,
    /// Discord channel message.
    Discord,
    /// Telegram chat message.
    Telegram,
    /// Slack workspace message.
    Slack,
    /// HTTP/REST API call.
    Api,
    /// News feed or RSS item.
    News,
    /// Ingested document.
    Document,
    /// Explicit operator/user action.
    Manual,
}

/// Event type recorded in the memory event ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MemoryEventType {
    /// A new fact was added to memory.
    FactAdded,
    /// An existing fact was updated.
    FactUpdated,
    /// A tool was invoked during a run.
    ///
    /// Recorded so that the memory layer can maintain a causal chain between
    /// observations and the tool calls that produced them. Useful for
    /// replaying or auditing a run's reasoning trajectory.
    ToolExecuted,
    /// The agent's active objective in the identity layer was replaced.
    ///
    /// The identity layer holds a stable description of the agent's purpose
    /// and long-term goals. This event fires when an operator or configuration
    /// change installs a new objective string, replacing the previous one.
    /// It should be rare: frequent objective changes indicate instability in
    /// the deployment configuration rather than normal agent operation.
    IdentityObjectiveChanged,
    /// A new commitment was added to the agent's adaptive layer.
    IdentityCommitmentAdded,
    /// A commitment was fulfilled and removed.
    IdentityCommitmentCompleted,
    /// Identity drift was detected in the stable layer.
    IdentityDriftDetected,
    /// A user preference was set.
    PreferenceSet,
    /// A user preference was removed.
    PreferenceUnset,
    /// A claim was inferred from context.
    InferredClaim,
    /// Two or more facts in memory were found to be mutually inconsistent.
    ///
    /// A contradiction exists when the same slot (or logically related slots)
    /// holds values that cannot both be true simultaneously — for example,
    /// `user.location = "Paris"` and `user.location = "Tokyo"` recorded from
    /// different sources with no subsequent reconciliation. This event is
    /// written by the consistency checker when it detects such a conflict so
    /// that the reasoning layer can request clarification or apply a
    /// resolution strategy rather than silently using stale data.
    ContradictionMarked,
    /// A record was soft-deleted (reversible).
    SoftDeleted,
    /// A record was permanently deleted.
    HardDeleted,
    /// A tombstone marker was written for deletion.
    TombstoneWritten,
    /// Multiple records were compacted into a summary.
    SummaryCompacted,
}

impl std::str::FromStr for MemoryEventType {
    type Err = crate::contracts::memory_error::MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_lowercase();
        let parsed = match normalized.as_str() {
            "fact_added" => Self::FactAdded,
            "fact_updated" => Self::FactUpdated,
            "tool_executed" => Self::ToolExecuted,
            "identity_objective_changed" => Self::IdentityObjectiveChanged,
            "identity_commitment_added" => Self::IdentityCommitmentAdded,
            "identity_commitment_completed" => Self::IdentityCommitmentCompleted,
            "identity_drift_detected" => Self::IdentityDriftDetected,
            "preference_set" => Self::PreferenceSet,
            "preference_unset" => Self::PreferenceUnset,
            "inferred_claim" => Self::InferredClaim,
            "contradiction_marked" => Self::ContradictionMarked,
            "soft_deleted" => Self::SoftDeleted,
            "hard_deleted" => Self::HardDeleted,
            "tombstone_written" => Self::TombstoneWritten,
            "summary_compacted" => Self::SummaryCompacted,
            _ => {
                return Err(crate::contracts::memory_error::MemoryError::validation(
                    format!("invalid memory event_type: {value}"),
                ));
            }
        };
        Ok(parsed)
    }
}

/// Input payload for appending a memory event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEventInput {
    /// Entity (user/tenant) this event belongs to.
    pub entity_id: EntityId,
    /// Namespaced key identifying the memory slot.
    pub slot_key: SlotKey,
    /// Target storage layer for this event.
    pub layer: MemoryLayer,
    /// Type of memory mutation being recorded.
    pub event_type: MemoryEventType,
    /// Serialized value payload.
    pub value: String,
    /// Provenance class of the event source.
    pub source: MemorySource,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: Importance,
    /// Detailed provenance metadata, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<MemoryProvenance>,
    /// Ingestion signal tier for policy decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_tier: Option<SignalTier>,
    /// Origin domain of the memory signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<SourceKind>,
    /// Opaque reference identifying the source context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// Privacy tier governing access to this value.
    pub privacy_level: PrivacyLevel,
    /// RFC 3339 timestamp of when the event occurred.
    pub occurred_at: String,
    /// Coarse emotion category inferred from interaction text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion_label: Option<String>,
    /// Valence score for inferred emotion in `[-1.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion_valence: Option<f64>,
    /// Arousal score for inferred emotion in `[0.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion_arousal: Option<f64>,
    /// Confidence score for inferred emotion in `[0.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion_confidence: Option<f64>,
}

impl MemoryEventInput {
    /// Construct a new memory event input with sane defaults.
    pub fn new(
        entity_id: impl AsRef<str>,
        slot_key: impl AsRef<str>,
        event_type: MemoryEventType,
        value: impl Into<String>,
        source: MemorySource,
        privacy_level: PrivacyLevel,
    ) -> Self {
        Self {
            entity_id: EntityId::new(entity_id.as_ref()),
            slot_key: SlotKey::new(slot_key.as_ref()),
            layer: MemoryLayer::Working,
            event_type,
            value: value.into(),
            source,
            confidence: Confidence::new(source.default_confidence()),
            importance: Importance::new(0.5),
            provenance: None,
            signal_tier: None,
            source_kind: None,
            source_ref: None,
            privacy_level,
            occurred_at: chrono::Utc::now().to_rfc3339(),
            emotion_label: None,
            emotion_valence: None,
            emotion_arousal: None,
            emotion_confidence: None,
        }
    }

    /// Override confidence, clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Confidence::new(confidence);
        self
    }

    /// Override importance, clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn with_importance(mut self, importance: f64) -> Self {
        self.importance = Importance::new(importance);
        self
    }

    /// Override occurrence timestamp.
    #[must_use]
    pub fn with_occurred_at(mut self, occurred_at: impl Into<String>) -> Self {
        self.occurred_at = occurred_at.into();
        self
    }

    /// Override target memory layer.
    #[must_use]
    pub fn with_layer(mut self, layer: MemoryLayer) -> Self {
        self.layer = layer;
        self
    }

    /// Attach provenance metadata.
    #[must_use]
    pub fn with_provenance(mut self, provenance: MemoryProvenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    /// Attach signal tier metadata.
    #[must_use]
    pub fn with_signal_tier(mut self, signal_tier: SignalTier) -> Self {
        self.signal_tier = Some(signal_tier);
        self
    }

    /// Attach source-kind metadata.
    #[must_use]
    pub fn with_source_kind(mut self, source_kind: SourceKind) -> Self {
        self.source_kind = Some(source_kind);
        self
    }

    /// Attach source reference metadata.
    #[must_use]
    pub fn with_source_ref(mut self, source_ref: impl Into<String>) -> Self {
        self.source_ref = Some(source_ref.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_layer_serde_roundtrip_all_variants() {
        let cases = [
            (MemoryLayer::Working, "working"),
            (MemoryLayer::Episodic, "episodic"),
            (MemoryLayer::Semantic, "semantic"),
            (MemoryLayer::Procedural, "procedural"),
            (MemoryLayer::Identity, "identity"),
        ];

        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let parsed: MemoryLayer = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn memory_event_type_from_str_accepts_tool_executed() {
        let parsed = "tool_executed".parse::<MemoryEventType>().unwrap();
        assert_eq!(parsed, MemoryEventType::ToolExecuted);
    }

    #[test]
    fn memory_event_type_serde_roundtrip_includes_tool_executed() {
        let json = serde_json::to_string(&MemoryEventType::ToolExecuted).unwrap();
        assert_eq!(json, "\"tool_executed\"");

        let parsed: MemoryEventType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, MemoryEventType::ToolExecuted);
    }

    #[test]
    fn memory_event_type_from_str_parses_identity_variants() {
        let cases = [
            (
                "identity_objective_changed",
                MemoryEventType::IdentityObjectiveChanged,
            ),
            (
                "identity_commitment_added",
                MemoryEventType::IdentityCommitmentAdded,
            ),
            (
                "identity_commitment_completed",
                MemoryEventType::IdentityCommitmentCompleted,
            ),
            (
                "identity_drift_detected",
                MemoryEventType::IdentityDriftDetected,
            ),
        ];

        for (raw, expected) in cases {
            let parsed = raw.parse::<MemoryEventType>().unwrap();
            assert_eq!(parsed, expected);
        }
    }
}
