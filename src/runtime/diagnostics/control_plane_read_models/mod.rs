//! Read models for the control-plane HTTP API.
//!
//! This module contains flat, serializable DTOs that are assembled from the
//! internal domain objects and returned by the control-plane endpoints.
//! Every struct here is output-only: they are never parsed from user input.
//!
//! # Design notes
//! - "Preview" types expose a compact summary suitable for list views.
//! - "Detail" types include all sub-objects needed by a full trace view.
//! - Builder functions (`build_*`) do the projection from domain objects.
//! - The `SessionSummarySource` and `SessionMessageSource` traits decouple
//!   the builders from concrete storage types (in-memory vs. `PostgreSQL`).

/// Governance/trust/operator read models.
pub mod governance_status;
/// Memory consolidation worker diagnostics read models.
pub mod memory_consolidation;
/// Memory grounding exposure diagnostics read models.
pub mod memory_exposure;
/// Memory review and correction read models.
pub mod memory_review;
/// Runtime status and capability read models.
pub mod runtime_status;
/// Dry-run self-amendment candidate review read models.
pub mod self_amendment_review;
/// Session and session-message read models.
pub mod session;
/// Tenant inventory/context read models.
pub mod tenant;

pub use governance_status::{
    GovernanceDomainTrustReadModel, GovernancePendingWindowReadModel, GovernanceRuntimeReadModel,
    GovernanceSummaryReadModel, build_governance_summary_read_model,
};
pub use memory_consolidation::{
    MemoryConsolidationStatusReadModel, MemoryConsolidationWorkerReadModel,
    build_memory_consolidation_status_read_model,
};
pub use memory_exposure::{MemoryExposureStatusReadModel, build_memory_exposure_status_read_model};
pub use memory_review::{
    MemoryCorrectionReadModel, MemoryEntityListReadModel, MemoryEntitySummaryReadModel,
    MemorySlotListReadModel, MemorySlotProvenanceReadModel, MemorySlotSummaryReadModel,
    build_memory_correction_read_model, build_memory_entity_list_read_model,
    build_memory_slot_list_read_model,
};
pub use runtime_status::{
    RuntimeCapabilitiesReadModel, RuntimeCapabilityDetailReadModel, RuntimeDbStatusReadModel,
    RuntimeGatewayStatusReadModel, RuntimeStatusReadModel, build_runtime_status_read_model,
};
pub use self_amendment_review::{
    SelfAmendmentApprovalReadModel, SelfAmendmentCandidateReviewReadModel,
    SelfAmendmentCandidateReviewSource, SelfAmendmentReviewReadModel,
    build_self_amendment_approval_read_model, build_self_amendment_review_read_model,
};
pub use session::{
    SessionListReadModel, SessionMessageListReadModel, SessionMessageReadModel,
    SessionMessageSource, SessionSummaryReadModel, SessionSummarySource,
    build_session_list_read_model, build_session_message_list_read_model,
};
pub use tenant::{
    TenantContextReadModel, TenantInventoryReadModel, TenantRegistryRowReadModel,
    build_tenant_context_read_model, build_tenant_inventory_read_model,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_status_read_model_roundtrip() {
        let model = build_runtime_status_read_model(
            "ok",
            "postgres".to_string(),
            "connected",
            "claude".to_string(),
            3,
            128,
            RuntimeCapabilitiesReadModel {
                companion: true,
                governance: true,
                memory_review: true,
                channel_posture: true,
                session_review: true,
                a2a: true,
                multi_tenant: true,
            },
            vec![RuntimeCapabilityDetailReadModel {
                name: "observability".to_string(),
                status: "degraded".to_string(),
                reason: Some("OpenTelemetry backend is counter-only stub".to_string()),
            }],
        );

        let json = serde_json::to_string(&model).expect("serialize runtime status read model");
        let decoded: RuntimeStatusReadModel =
            serde_json::from_str(&json).expect("deserialize runtime status read model");

        assert_eq!(decoded.status, "ok");
        assert_eq!(decoded.gateway.ws_connections, 3);
        assert!(decoded.capabilities.governance);
        assert!(decoded.capabilities.memory_review);
        assert_eq!(decoded.capability_details[0].name, "observability");
    }

    #[test]
    fn governance_summary_read_model_roundtrip() {
        let model = build_governance_summary_read_model(
            GovernanceRuntimeReadModel {
                memory_backend: "postgres".to_string(),
                memory_review: true,
                companion_surface_windows: 1,
                companion_surface_scopes: 1,
            },
            vec![GovernanceDomainTrustReadModel {
                domain: "shell".to_string(),
                score: 0.8,
                autonomy: "supervised".to_string(),
                success_count: 4,
                violation_count: 1,
                last_updated: "2026-04-10T00:00:00Z".to_string(),
            }],
            vec![GovernancePendingWindowReadModel {
                scope: "global".to_string(),
                window_id: "win-1".to_string(),
                requested_action: "confirm memory correction".to_string(),
                created_at: "2026-04-10T00:00:00Z".to_string(),
                expires_at: "2026-04-10T00:10:00Z".to_string(),
            }],
        );

        let json = serde_json::to_string(&model).expect("serialize governance summary read model");
        let decoded: GovernanceSummaryReadModel =
            serde_json::from_str(&json).expect("deserialize governance summary read model");

        assert_eq!(decoded.runtime.memory_backend, "postgres");
        assert_eq!(decoded.domain_trust.len(), 1);
        assert_eq!(decoded.pending_windows.len(), 1);
    }

    #[test]
    fn memory_consolidation_status_read_model_roundtrip() {
        let model = build_memory_consolidation_status_read_model(vec![
            crate::core::memory::ConsolidationWorkerStatus {
                entity_id: crate::contracts::ids::EntityId::new("tenant:user"),
                checkpoint_event_count: 7,
                phase: crate::core::memory::ConsolidationWorkerPhase::Completed,
                disposition: Some(crate::core::memory::ConsolidationDisposition::Consolidated),
                previous_watermark: Some(3),
                applied_watermark: Some(7),
                started_at: Some("2026-04-24T00:00:00Z".to_string()),
                finished_at: Some("2026-04-24T00:00:01Z".to_string()),
                last_error: None,
            },
        ]);

        let json =
            serde_json::to_string(&model).expect("serialize memory consolidation read model");
        let decoded: MemoryConsolidationStatusReadModel =
            serde_json::from_str(&json).expect("deserialize memory consolidation read model");

        assert_eq!(decoded.count, 1);
        assert_eq!(decoded.items[0].entity_id.as_str(), "tenant:user");
        assert_eq!(decoded.items[0].phase, "completed");
        assert_eq!(
            decoded.items[0].disposition.as_deref(),
            Some("consolidated")
        );
    }

    #[test]
    fn memory_exposure_status_read_model_roundtrip() {
        let model = build_memory_exposure_status_read_model(
            &crate::core::memory::influence::GroundingExposureMonitorSnapshot {
                observed_builds: 2,
                public_visible_total: 3,
                private_internal_total: 4,
                secret_suppressed_total: 1,
                last_projection: crate::core::memory::influence::GroundingExposureProjection {
                    public_visible: 1,
                    private_internal: 2,
                    secret_suppressed: 1,
                },
            },
        );

        let json = serde_json::to_string(&model).expect("serialize memory exposure read model");
        let decoded: MemoryExposureStatusReadModel =
            serde_json::from_str(&json).expect("deserialize memory exposure read model");

        assert_eq!(decoded.observed_builds, 2);
        assert!(decoded.sensitive_counts_redacted);
        assert!(!json.contains("private_internal"));
        assert!(!json.contains("secret_suppressed"));
        assert!(!json.contains("last_projection"));
    }

    #[test]
    fn self_amendment_review_read_model_handles_empty_candidates() {
        let candidates: Vec<crate::core::persona::soul_core::SelfAmendmentCandidate> = Vec::new();
        let model = build_self_amendment_review_read_model(&candidates);

        assert_eq!(model.count, 0);
        assert!(model.items.is_empty());
        assert!(model.raw_payloads_redacted);
        assert!(!model.durable_writes_enabled);
    }

    #[test]
    fn self_amendment_review_read_model_redacts_payloads() {
        let pressure = crate::core::persona::soul_core::SoulPressure {
            repair: 0.95,
            ..crate::core::persona::soul_core::SoulPressure::default()
        };
        let candidates = crate::core::persona::soul_core::generate_self_amendment_candidates(
            crate::core::persona::soul_core::SelfAmendmentCandidateInput {
                user_message: "your answer is wrong; do not keep the amethyst anchor",
                assistant_response: "I repeated the amethyst anchor.",
                soul_pressure: &pressure,
                tenant_id: Some("tenant-alpha"),
                person_id: "person-test",
                surface: Some("discord"),
                evidence_ids: &["post_turn:user_message", "post_turn:assistant_response"],
            },
        );

        let model = build_self_amendment_review_read_model(&candidates);
        let json =
            serde_json::to_string(&model).expect("serialize self-amendment review read model");
        let decoded: SelfAmendmentReviewReadModel =
            serde_json::from_str(&json).expect("deserialize self-amendment review read model");

        assert_eq!(decoded.count, 1);
        assert!(decoded.raw_payloads_redacted);
        assert!(!decoded.durable_writes_enabled);
        assert!(decoded.items[0].raw_payload_redacted);
        assert_eq!(decoded.items[0].kind, "repair_practice");
        assert_eq!(decoded.items[0].status, "dry_run_only");
        assert_eq!(decoded.items[0].privacy, "private_internal");
        assert_eq!(decoded.items[0].tenant_id.as_deref(), Some("tenant-alpha"));
        assert_eq!(
            decoded.items[0].evidence_ids,
            ["post_turn:user_message", "post_turn:assistant_response"]
        );
        assert!(!json.contains("your answer is wrong"));
        assert!(!json.contains("amethyst anchor"));
    }

    #[test]
    fn self_amendment_review_read_model_bounds_and_deduplicates_evidence() {
        let long_reason = format!("{}\n{}", "reason".repeat(50), "private raw line".repeat(20));
        let long_amendment = format!("{}\t{}", "amend".repeat(60), "tail".repeat(30));
        let mut evidence_ids = vec!["event:duplicate".to_string(), "event:duplicate".to_string()];
        evidence_ids.extend((0..20).map(|idx| format!("event:{idx}")));
        let candidate = crate::core::persona::soul_core::SelfAmendmentCandidate {
            candidate_id: "self_amendment:tenant-alpha:person-test:discord:repair_practice"
                .to_string(),
            kind: crate::core::persona::soul_core::SelfAmendmentCandidateKind::RepairPractice,
            status: crate::core::persona::soul_core::SelfAmendmentCandidateStatus::DryRunOnly,
            tenant_id: None,
            person_id: "person-test".to_string(),
            surface: "discord".to_string(),
            privacy: crate::core::persona::soul_core::SelfAmendmentPrivacy::PrivateInternal,
            evidence_ids,
            reason: long_reason,
            proposed_amendment: long_amendment,
        };

        let model = build_self_amendment_review_read_model(&[candidate]);
        let item = &model.items[0];

        assert_eq!(model.count, 1);
        assert_eq!(item.evidence_ids.len(), 12);
        assert_eq!(
            item.evidence_ids
                .iter()
                .filter(|id| id.as_str() == "event:duplicate")
                .count(),
            1
        );
        assert!(!item.reason.contains('\n'));
        assert!(!item.proposed_amendment.contains('\t'));
        assert!(item.reason.chars().count() <= 163);
        assert!(item.proposed_amendment.chars().count() <= 203);
    }
}
