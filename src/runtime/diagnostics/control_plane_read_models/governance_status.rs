//! Governance status read models for companion-safe operator inspection.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceRuntimeReadModel {
    pub memory_backend: String,
    pub memory_review: bool,
    pub companion_surface_windows: usize,
    pub companion_surface_scopes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceDomainTrustReadModel {
    pub domain: String,
    pub score: f32,
    pub autonomy: String,
    pub success_count: u32,
    pub violation_count: u32,
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernancePendingWindowReadModel {
    pub scope: String,
    pub window_id: String,
    pub requested_action: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceSummaryReadModel {
    pub runtime: GovernanceRuntimeReadModel,
    pub domain_trust: Vec<GovernanceDomainTrustReadModel>,
    pub pending_windows: Vec<GovernancePendingWindowReadModel>,
}

#[must_use]
pub fn build_governance_summary_read_model(
    runtime: GovernanceRuntimeReadModel,
    domain_trust: Vec<GovernanceDomainTrustReadModel>,
    pending_windows: Vec<GovernancePendingWindowReadModel>,
) -> GovernanceSummaryReadModel {
    GovernanceSummaryReadModel {
        runtime,
        domain_trust,
        pending_windows,
    }
}
