//! Forget-related data types: outcomes, statuses, modes, and
//! artifact verification records.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, SlotKey};

/// Result of a forget operation, including artifact verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetOutcome {
    /// Entity whose slot was targeted for deletion.
    pub entity_id: EntityId,
    /// Slot key that was targeted.
    pub slot_key: SlotKey,
    /// Forget mode that was requested.
    pub mode: ForgetMode,
    /// Whether the backend applied the deletion.
    pub was_applied: bool,
    /// Whether all artifact checks passed.
    pub is_complete: bool,
    /// Whether the backend operated in degraded mode.
    pub is_degraded: bool,
    /// Overall status summarising the outcome.
    pub status: ForgetStatus,
    /// Per-artifact verification results.
    pub artifact_checks: Vec<ForgetArtifactCheck>,
}

/// Summary status of a forget operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForgetStatus {
    /// All artifacts verified and deletion fully applied.
    Complete,
    /// Deletion applied but some artifact checks failed.
    Incomplete,
    /// Backend operated in degraded mode; not all checks pass.
    DegradedNonComplete,
    /// Deletion was not applied at all.
    NotApplied,
}

/// Storage artifact type that may be affected by a forget operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForgetArtifact {
    /// The belief slot record itself.
    Slot,
    /// Retrieval units (embeddings / search index entries).
    RetrievalUnits,
    /// Source documents backing the retrieval units.
    RetrievalDocs,
    /// In-memory or on-disk caches.
    Caches,
    /// Deletion audit ledger entry.
    Ledger,
}

/// What a forget mode requires of a particular artifact.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForgetRequirement {
    /// The forget mode does not govern this artifact.
    NotGoverned,
    /// The artifact must exist (e.g. a ledger entry).
    MustExist,
    /// The artifact must be physically absent.
    MustBeAbsent,
    /// The artifact may exist but must not be retrievable.
    MustBeNonRetrievable,
}

/// Observed state of an artifact after a forget operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForgetObservation {
    /// The artifact is physically absent.
    Absent,
    /// The artifact exists but is not retrievable.
    PresentNonRetrievable,
    /// The artifact exists and is still retrievable.
    PresentRetrievable,
}

/// A single artifact verification result within a forget outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForgetArtifactCheck {
    /// Which artifact was checked.
    pub artifact: ForgetArtifact,
    /// What the forget mode required of this artifact.
    pub requirement: ForgetRequirement,
    /// What was actually observed after the operation.
    pub observed: ForgetObservation,
    /// Whether the observation satisfies the requirement.
    pub is_satisfied: bool,
}

impl ForgetArtifactCheck {
    /// Create a check, automatically computing `satisfied`.
    #[must_use]
    pub fn new(
        artifact: ForgetArtifact,
        requirement: ForgetRequirement,
        observed: ForgetObservation,
    ) -> Self {
        Self {
            artifact,
            requirement,
            observed,
            is_satisfied: requirement.is_satisfied_by(observed),
        }
    }
}

impl ForgetRequirement {
    /// Check whether an observation satisfies this requirement.
    #[must_use]
    pub const fn is_satisfied_by(self, observed: ForgetObservation) -> bool {
        match self {
            Self::NotGoverned => true,
            Self::MustExist => !matches!(observed, ForgetObservation::Absent),
            Self::MustBeAbsent => matches!(observed, ForgetObservation::Absent),
            Self::MustBeNonRetrievable => {
                !matches!(observed, ForgetObservation::PresentRetrievable)
            }
        }
    }
}

impl ForgetMode {
    /// Return the requirement this forget mode places on the given artifact.
    ///
    /// The decision matrix below is the single authoritative source of truth
    /// for what each `ForgetMode` demands of each `ForgetArtifact`. Callers
    /// use the returned `ForgetRequirement` to verify post-deletion state via
    /// `ForgetRequirement::is_satisfied_by`.
    ///
    /// ```text
    /// ┌──────────────┬──────────────────┬───────────────┬───────────────┬──────────┬──────────────────┐
    /// │ Mode         │ Slot             │ RetrievalUnits│ RetrievalDocs │ Caches   │ Ledger           │
    /// ├──────────────┼──────────────────┼───────────────┼───────────────┼──────────┼──────────────────┤
    /// │ Soft         │ MustBeNonRetriev.│ MustBeNonRetr.│ MustBeNonRetr.│ NotGov.  │ MustExist        │
    /// │ Hard         │ MustBeAbsent     │ MustBeAbsent  │ MustBeAbsent  │ MustBeAbs│ MustExist        │
    /// │ Tombstone    │ MustBeNonRetriev.│ MustBeAbsent  │ MustBeAbsent  │ MustBeAbs│ MustExist        │
    /// └──────────────┴──────────────────┴───────────────┴───────────────┴──────────┴──────────────────┘
    /// ```
    ///
    /// **Soft** — data is hidden but physical bytes may remain. The slot and
    /// retrieval index entries must be non-retrievable (e.g., flagged as
    /// deleted in the index), but caches are left unmanaged because they will
    /// expire naturally. A ledger entry must be created to record the event.
    ///
    /// **Hard** — full physical removal. Every artifact except the ledger
    /// must be completely absent after the operation. Used for GDPR erasure
    /// and explicit user data-deletion requests.
    ///
    /// **Tombstone** — hybrid: the slot is replaced by a tombstone record
    /// (non-retrievable in queries but present as a marker), while retrieval
    /// units, docs, and caches are physically deleted. Used when the system
    /// needs evidence that a slot *existed* without exposing its content.
    /// The ledger entry is always required to satisfy the audit trail.
    #[must_use]
    pub const fn artifact_requirement(self, artifact: ForgetArtifact) -> ForgetRequirement {
        match (self, artifact) {
            (
                Self::Soft,
                ForgetArtifact::Slot
                | ForgetArtifact::RetrievalUnits
                | ForgetArtifact::RetrievalDocs,
            )
            | (Self::Tombstone, ForgetArtifact::Slot) => ForgetRequirement::MustBeNonRetrievable,
            (Self::Soft, ForgetArtifact::Caches) => ForgetRequirement::NotGoverned,
            (Self::Soft | Self::Hard | Self::Tombstone, ForgetArtifact::Ledger) => {
                ForgetRequirement::MustExist
            }
            (
                Self::Hard,
                ForgetArtifact::Slot
                | ForgetArtifact::RetrievalUnits
                | ForgetArtifact::RetrievalDocs
                | ForgetArtifact::Caches,
            )
            | (
                Self::Tombstone,
                ForgetArtifact::RetrievalUnits
                | ForgetArtifact::RetrievalDocs
                | ForgetArtifact::Caches,
            ) => ForgetRequirement::MustBeAbsent,
        }
    }
}

impl ForgetOutcome {
    /// Build a `ForgetOutcome` from individual artifact checks.
    #[must_use]
    pub fn from_checks(
        entity_id: impl Into<String>,
        slot_key: impl Into<String>,
        mode: ForgetMode,
        applied: bool,
        degraded: bool,
        artifact_checks: Vec<ForgetArtifactCheck>,
    ) -> Self {
        let complete = applied && artifact_checks.iter().all(|check| check.is_satisfied);
        let status = if complete {
            ForgetStatus::Complete
        } else if degraded {
            ForgetStatus::DegradedNonComplete
        } else if !applied {
            ForgetStatus::NotApplied
        } else {
            ForgetStatus::Incomplete
        };

        Self {
            entity_id: EntityId::new(entity_id),
            slot_key: SlotKey::new(slot_key),
            mode,
            was_applied: applied,
            is_complete: complete,
            is_degraded: degraded,
            status,
            artifact_checks,
        }
    }
}

/// Deletion mode controlling how aggressively data is removed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForgetMode {
    /// Mark as non-retrievable; physical data may remain.
    Soft,
    /// Physically delete all artifacts except the ledger entry.
    Hard,
    /// Replace slot with a tombstone; delete other artifacts.
    Tombstone,
}
