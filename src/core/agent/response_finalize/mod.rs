//! Response finalization: audit → fix → safety-check pipeline.
//!
//! [`finalize_response_with_context`] is the main entry point. It first
//! preserves control output verbatim, then runs the naturalness gate's critical
//! block decision before any contract fallback or raw-text bypass can preserve unsafe text.
//! Only after those safety checks does it audit style, apply deterministic
//! repairs, and verify protected segments with [`protected_segments_match`].
//!
//! Several conditions bypass the fix stage entirely:
//! - Streaming is active (the text was already delivered to the client); critical
//!   naturalness blocking still runs before this bypass.
//! - `control_output` flag is set (internal plan/report output).
//! - Structured risk was detected by the audit (JSON, tables, etc.).
//! - Explicit reasoning tags (`<think>`, `<reasoning>`) are present.

use super::naturalness_gate::{
    AffectLevel, GateDecision, Locale, NaturalnessGate, NaturalnessInput, OutputProfile,
    RelationshipDistance, TurnContextView,
};
use super::response_audit::{
    ContractMismatchReason, ExposurePlanContract, ResponseAuditFindingKind, ResponseAuditReport,
    ResponseContract, audit_response, audit_response_against_contract, audit_response_contextual,
};
use super::response_fix::{ResponseFixResult, apply_deterministic_fixes};
use super::response_style::ResponseMode;
use crate::core::affect::{AffectLabel, AffectReading, RuleBasedDetector};
use crate::core::persona::relationship::RelationshipState;
use crate::core::providers::response::{ContentBlock, MessageRole, ProviderMessage};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResponseFinalizationRequest<'a> {
    raw_text: &'a str,
    output_mode: ResponseMode,
    streaming_active: bool,
    control_output: bool,
    contract: Option<&'a ResponseContract>,
    naturalness_gate_enabled: bool,
}

impl<'a> ResponseFinalizationRequest<'a> {
    #[must_use]
    pub(crate) const fn user_facing(
        raw_text: &'a str,
        output_mode: ResponseMode,
        streaming_active: bool,
        contract: Option<&'a ResponseContract>,
        naturalness_gate_enabled: bool,
    ) -> Self {
        Self {
            raw_text,
            output_mode,
            streaming_active,
            control_output: false,
            contract,
            naturalness_gate_enabled,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NaturalnessFinalizationContext<'a> {
    pub(crate) conversation_history: &'a [ProviderMessage],
    pub(crate) user_affect: AffectLevel,
    pub(crate) relationship_distance: RelationshipDistance,
}

impl Default for NaturalnessFinalizationContext<'_> {
    fn default() -> Self {
        Self {
            conversation_history: &[],
            user_affect: AffectLevel::Unknown,
            relationship_distance: RelationshipDistance::Unknown,
        }
    }
}

struct PreparedNaturalnessContext {
    recent_opening_phrases: Vec<String>,
    user_affect: AffectLevel,
    relationship_distance: RelationshipDistance,
}

impl PreparedNaturalnessContext {
    fn from_context(context: NaturalnessFinalizationContext<'_>) -> Self {
        Self {
            recent_opening_phrases: recent_assistant_opening_phrases(context.conversation_history),
            user_affect: context.user_affect,
            relationship_distance: context.relationship_distance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NaturalnessRelationshipSurface {
    Private,
    Public,
}

#[must_use]
pub(crate) fn naturalness_affect_from_reading(reading: &AffectReading) -> AffectLevel {
    if reading.is_ambiguous() {
        return AffectLevel::Unknown;
    }

    match reading.label {
        AffectLabel::Neutral => AffectLevel::Neutral,
        AffectLabel::Angry => AffectLevel::Angry,
        AffectLabel::Anxious => AffectLevel::Anxious,
        AffectLabel::Sad | AffectLabel::Frustrated | AffectLabel::Overwhelmed => {
            AffectLevel::StrongNegative
        }
        AffectLabel::Excited | AffectLabel::Grateful | AffectLabel::Curious => {
            AffectLevel::LightPositive
        }
        AffectLabel::Confused => AffectLevel::Unknown,
    }
}

#[must_use]
pub(crate) fn naturalness_affect_from_text(user_message: &str) -> AffectLevel {
    naturalness_affect_from_reading(&RuleBasedDetector::new().detect(user_message))
}

#[must_use]
pub(crate) fn naturalness_relationship_surface_from_contract(
    contract: Option<&ResponseContract>,
) -> NaturalnessRelationshipSurface {
    if contract
        .is_some_and(|contract| matches!(contract.exposure_plan, ExposurePlanContract::PublicSafe))
    {
        NaturalnessRelationshipSurface::Public
    } else {
        NaturalnessRelationshipSurface::Private
    }
}

#[must_use]
pub(crate) fn naturalness_relationship_distance_from_state(
    state: Option<&RelationshipState>,
    surface: NaturalnessRelationshipSurface,
) -> RelationshipDistance {
    let Some(state) = state else {
        return RelationshipDistance::Unknown;
    };

    if state.interaction_count < 3 {
        return RelationshipDistance::Unknown;
    }

    if matches!(surface, NaturalnessRelationshipSurface::Public)
        || state.repair_debt >= 0.40
        || state.unresolved_tension >= 0.45
        || state.trust_level < 0.45
        || state.rapport < 0.45
    {
        return RelationshipDistance::Formal;
    }

    if state.interaction_count >= 20
        && state.trust_level >= 0.75
        && state.rapport >= 0.70
        && state.disclosure_depth >= 0.60
        && state.attachment_security >= 0.65
        && state.repair_debt < 0.15
        && state.unresolved_tension < 0.20
    {
        return RelationshipDistance::Intimate;
    }

    if state.interaction_count >= 5
        && state.trust_level >= 0.58
        && state.rapport >= 0.55
        && state.disclosure_depth >= 0.30
        && state.repair_debt < 0.30
        && state.unresolved_tension < 0.35
    {
        return RelationshipDistance::Friendly;
    }

    RelationshipDistance::Formal
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseFinalizationResult {
    pub(crate) final_text: String,
    pub(crate) applied_actions: Vec<super::response_audit::ResponseFixHint>,
    pub(crate) contract_mismatch_reason: Option<ContractMismatchReason>,
    pub(crate) micro_rewrite_reason_codes: Vec<&'static str>,
    pub(crate) before_score: u32,
    pub(crate) after_score: u32,
    pub(crate) preserved: bool,
}

mod response_finalize_io;
mod response_finalize_pipeline;

use response_finalize_io::recent_assistant_opening_phrases;

#[cfg(test)]
pub(crate) use response_finalize_pipeline::{finalize_response, finalize_response_contextual};
pub(crate) use response_finalize_pipeline::{
    finalize_response_contextual_with_context, finalize_response_with_context,
};

#[cfg(test)]
mod tests;
