use serde::{Deserialize, Serialize};

use crate::contracts::ids::EntityId;
use crate::core::memory::{
    ConsolidationDisposition, ConsolidationWorkerPhase, ConsolidationWorkerStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryConsolidationWorkerReadModel {
    pub entity_id: EntityId,
    pub checkpoint_event_count: usize,
    pub phase: String,
    pub disposition: Option<String>,
    pub previous_watermark: Option<usize>,
    pub applied_watermark: Option<usize>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryConsolidationStatusReadModel {
    pub count: usize,
    pub items: Vec<MemoryConsolidationWorkerReadModel>,
}

#[must_use]
pub fn build_memory_consolidation_status_read_model(
    statuses: Vec<ConsolidationWorkerStatus>,
) -> MemoryConsolidationStatusReadModel {
    let mut items: Vec<_> = statuses
        .into_iter()
        .map(|status| MemoryConsolidationWorkerReadModel {
            entity_id: status.entity_id,
            checkpoint_event_count: status.checkpoint_event_count,
            phase: phase_label(status.phase).to_string(),
            disposition: status
                .disposition
                .map(disposition_label)
                .map(str::to_string),
            previous_watermark: status.previous_watermark,
            applied_watermark: status.applied_watermark,
            started_at: status.started_at,
            finished_at: status.finished_at,
            last_error: status.last_error,
        })
        .collect();
    items.sort_by(|a, b| a.entity_id.as_str().cmp(b.entity_id.as_str()));
    MemoryConsolidationStatusReadModel {
        count: items.len(),
        items,
    }
}

const fn phase_label(phase: ConsolidationWorkerPhase) -> &'static str {
    match phase {
        ConsolidationWorkerPhase::Queued => "queued",
        ConsolidationWorkerPhase::Running => "running",
        ConsolidationWorkerPhase::Completed => "completed",
        ConsolidationWorkerPhase::Failed => "failed",
    }
}

const fn disposition_label(disposition: ConsolidationDisposition) -> &'static str {
    match disposition {
        ConsolidationDisposition::Consolidated => "consolidated",
        ConsolidationDisposition::SkippedNoSignal => "skipped_no_signal",
        ConsolidationDisposition::SkippedCheckpoint => "skipped_checkpoint",
    }
}
