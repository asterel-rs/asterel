//! Self-amendment candidate read models for operator inspection.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, EventId, SlotKey};
use crate::core::persona::soul_core::{
    SelfAmendmentCandidate, SelfAmendmentCandidateKind, SelfAmendmentCandidateStatus,
    SelfAmendmentPrivacy,
};
use crate::utils::text::{sanitize_prompt_line, truncate_ellipsis};

const MAX_REASON_CHARS: usize = 160;
const MAX_AMENDMENT_CHARS: usize = 200;
const MAX_EVIDENCE_IDS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfAmendmentReviewReadModel {
    pub count: usize,
    pub items: Vec<SelfAmendmentCandidateReviewReadModel>,
    pub raw_payloads_redacted: bool,
    pub durable_writes_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfAmendmentCandidateReviewReadModel {
    pub candidate_id: String,
    pub kind: String,
    pub status: String,
    pub tenant_id: Option<String>,
    pub person_id: String,
    pub surface: String,
    pub privacy: String,
    pub evidence_ids: Vec<String>,
    pub reason: String,
    pub proposed_amendment: String,
    pub raw_payload_redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfAmendmentApprovalReadModel {
    pub status: String,
    pub candidate_id: String,
    pub entity_id: EntityId,
    pub slot_key: SlotKey,
    pub event_id: EventId,
    pub persisted: bool,
    pub raw_payload_redacted: bool,
}

pub trait SelfAmendmentCandidateReviewSource {
    fn candidate_id(&self) -> &str;
    fn kind(&self) -> &'static str;
    fn status(&self) -> &'static str;
    fn tenant_id(&self) -> Option<&str>;
    fn person_id(&self) -> &str;
    fn surface(&self) -> &str;
    fn privacy(&self) -> &'static str;
    fn evidence_ids(&self) -> &[String];
    fn reason(&self) -> &str;
    fn proposed_amendment(&self) -> &str;
}

impl SelfAmendmentCandidateReviewSource for SelfAmendmentCandidate {
    fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    fn kind(&self) -> &'static str {
        candidate_kind_label(self.kind)
    }

    fn status(&self) -> &'static str {
        candidate_status_label(self.status)
    }

    fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    fn person_id(&self) -> &str {
        &self.person_id
    }

    fn surface(&self) -> &str {
        &self.surface
    }

    fn privacy(&self) -> &'static str {
        candidate_privacy_label(self.privacy)
    }

    fn evidence_ids(&self) -> &[String] {
        &self.evidence_ids
    }

    fn reason(&self) -> &str {
        &self.reason
    }

    fn proposed_amendment(&self) -> &str {
        &self.proposed_amendment
    }
}

#[must_use]
pub fn build_self_amendment_review_read_model<T: SelfAmendmentCandidateReviewSource>(
    candidates: &[T],
) -> SelfAmendmentReviewReadModel {
    let items = candidates
        .iter()
        .map(|candidate| SelfAmendmentCandidateReviewReadModel {
            candidate_id: project_line(candidate.candidate_id(), 256),
            kind: candidate.kind().to_string(),
            status: candidate.status().to_string(),
            tenant_id: candidate
                .tenant_id()
                .map(|tenant_id| project_line(tenant_id, 96)),
            person_id: project_line(candidate.person_id(), 96),
            surface: project_line(candidate.surface(), 64),
            privacy: candidate.privacy().to_string(),
            evidence_ids: project_evidence_ids(candidate.evidence_ids()),
            reason: project_line(candidate.reason(), MAX_REASON_CHARS),
            proposed_amendment: project_line(candidate.proposed_amendment(), MAX_AMENDMENT_CHARS),
            raw_payload_redacted: true,
        })
        .collect::<Vec<_>>();

    SelfAmendmentReviewReadModel {
        count: items.len(),
        items,
        raw_payloads_redacted: true,
        durable_writes_enabled: false,
    }
}

#[must_use]
pub fn build_self_amendment_approval_read_model(
    candidate_id: &str,
    entity_id: EntityId,
    slot_key: SlotKey,
    event_id: EventId,
) -> SelfAmendmentApprovalReadModel {
    SelfAmendmentApprovalReadModel {
        status: "persisted".to_string(),
        candidate_id: project_line(candidate_id, 256),
        entity_id,
        slot_key,
        event_id,
        persisted: true,
        raw_payload_redacted: true,
    }
}

fn project_evidence_ids(evidence_ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for evidence_id in evidence_ids {
        let projected = project_line(evidence_id, 96);
        if !projected.is_empty() && !out.contains(&projected) {
            out.push(projected);
        }
        if out.len() >= MAX_EVIDENCE_IDS {
            break;
        }
    }
    out
}

fn project_line(value: &str, max_chars: usize) -> String {
    truncate_ellipsis(&sanitize_prompt_line(value), max_chars)
}

const fn candidate_kind_label(kind: SelfAmendmentCandidateKind) -> &'static str {
    match kind {
        SelfAmendmentCandidateKind::RepairPractice => "repair_practice",
    }
}

const fn candidate_status_label(status: SelfAmendmentCandidateStatus) -> &'static str {
    match status {
        SelfAmendmentCandidateStatus::DryRunOnly => "dry_run_only",
    }
}

const fn candidate_privacy_label(privacy: SelfAmendmentPrivacy) -> &'static str {
    match privacy {
        SelfAmendmentPrivacy::PrivateInternal => "private_internal",
    }
}
