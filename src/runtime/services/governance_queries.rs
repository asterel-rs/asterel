use crate::core::tools::middleware::global_trust_tracker;
use crate::runtime::diagnostics::control_plane_read_models::{
    GovernanceDomainTrustReadModel, GovernancePendingWindowReadModel, GovernanceRuntimeReadModel,
    GovernanceSummaryReadModel, build_governance_summary_read_model,
};
use crate::security::domain_trust::DomainTrust;

#[derive(Debug, Clone)]
pub struct PendingWindowSnapshot {
    pub scope: String,
    pub window_id: String,
    pub requested_action: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct GovernanceSummarySnapshot {
    pub runtime: GovernanceRuntimeReadModel,
    pub domain_trust: Vec<DomainTrust>,
    pub pending_windows: Vec<GovernancePendingWindowReadModel>,
}

#[must_use]
pub fn load_admin_governance_summary(
    snapshot: GovernanceSummarySnapshot,
) -> GovernanceSummaryReadModel {
    build_governance_summary_read_model(
        snapshot.runtime,
        snapshot
            .domain_trust
            .into_iter()
            .map(|entry| GovernanceDomainTrustReadModel {
                domain: entry.domain,
                score: entry.score,
                autonomy: format!("{:?}", entry.autonomy).to_ascii_lowercase(),
                success_count: entry.success_count,
                violation_count: entry.violation_count,
                last_updated: entry.last_updated.to_rfc3339(),
            })
            .collect(),
        snapshot.pending_windows,
    )
}

#[must_use]
pub fn load_admin_governance_summary_with_runtime_trust(
    memory_backend: String,
    memory_review: bool,
    companion_surface_scopes: usize,
    pending_windows: Vec<PendingWindowSnapshot>,
) -> GovernanceSummaryReadModel {
    load_admin_governance_summary(GovernanceSummarySnapshot {
        runtime: GovernanceRuntimeReadModel {
            memory_backend,
            memory_review,
            companion_surface_windows: pending_windows.len(),
            companion_surface_scopes,
        },
        domain_trust: global_trust_tracker().all_domains(),
        pending_windows: pending_windows
            .into_iter()
            .map(|entry| GovernancePendingWindowReadModel {
                scope: entry.scope,
                window_id: entry.window_id,
                requested_action: entry.requested_action,
                created_at: entry.created_at,
                expires_at: entry.expires_at,
            })
            .collect(),
    })
}
