use serde::{Deserialize, Serialize};

use crate::core::memory::influence::GroundingExposureMonitorSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryExposureStatusReadModel {
    pub observed_builds: u64,
    pub sensitive_counts_redacted: bool,
}

#[must_use]
pub fn build_memory_exposure_status_read_model(
    snapshot: &GroundingExposureMonitorSnapshot,
) -> MemoryExposureStatusReadModel {
    MemoryExposureStatusReadModel {
        observed_builds: snapshot.observed_builds,
        sensitive_counts_redacted: true,
    }
}
