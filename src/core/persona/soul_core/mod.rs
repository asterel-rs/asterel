//! Read-only soul pressure derivation.
//!
//! This module does not claim subjective consciousness and does not own prompt
//! policy. It produces a small posture signal from existing persona, memory,
//! relationship, and turn inputs.

use serde::{Deserialize, Serialize};

use crate::contracts::affect::{AffectLabel, AffectReading};
use crate::core::persona::continuity_v2::DialogueAct;
use crate::core::persona::relationship::RelationshipState;
use crate::core::persona::user_model::{EmotionalNeed, UserIntent, UserMentalModel};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SoulRecallExposure {
    pub public: usize,
    pub private: usize,
    pub secret: usize,
}

impl SoulRecallExposure {
    #[must_use]
    pub(crate) const fn has_fragile_recall(self) -> bool {
        self.private > 0 || self.secret > 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SoulSurfaceExposure {
    #[default]
    Unknown,
    PublicSafe,
    PrivateAllowed,
}

impl SoulSurfaceExposure {
    const fn as_note_value(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::PublicSafe => "public_safe",
            Self::PrivateAllowed => "private_allowed",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SoulIdentityCues<'a> {
    pub soul_root_sentence: &'a str,
    pub values: &'a [String],
    pub negative_identity: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SoulPressure {
    pub truth: f32,
    pub care: f32,
    pub restraint: f32,
    pub memory_discretion: f32,
    pub continuity: f32,
    pub repair: f32,
    pub autonomy: f32,
    pub wonder: f32,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SoulTopologyCues {
    pub surfaced_curiosity: f32,
    pub surfaced_guardedness: f32,
    pub surfaced_anxiety: f32,
    pub surfaced_attachment: f32,
    pub surfaced_shame: f32,
    pub surfaced_irony: f32,
    pub suppressed_internal: f32,
}

impl SoulTopologyCues {
    #[must_use]
    pub(crate) fn has_signal(self) -> bool {
        self.surfaced_curiosity > 0.01
            || self.surfaced_guardedness > 0.01
            || self.surfaced_anxiety > 0.01
            || self.surfaced_attachment > 0.01
            || self.surfaced_shame > 0.01
            || self.surfaced_irony > 0.01
            || self.suppressed_internal > 0.01
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelfAmendmentCandidateKind {
    RepairPractice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelfAmendmentCandidateStatus {
    DryRunOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelfAmendmentPrivacy {
    PrivateInternal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SelfAmendmentCandidate {
    pub candidate_id: String,
    pub kind: SelfAmendmentCandidateKind,
    pub status: SelfAmendmentCandidateStatus,
    pub tenant_id: Option<String>,
    pub person_id: String,
    pub surface: String,
    pub privacy: SelfAmendmentPrivacy,
    pub evidence_ids: Vec<String>,
    pub reason: String,
    pub proposed_amendment: String,
}

pub(crate) trait SelfAmendmentCandidateSink: Send + Sync {
    fn record_self_amendment_candidates(&self, candidates: &[SelfAmendmentCandidate]);
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SelfAmendmentCandidateInput<'a> {
    pub user_message: &'a str,
    pub assistant_response: &'a str,
    pub soul_pressure: &'a SoulPressure,
    pub tenant_id: Option<&'a str>,
    pub person_id: &'a str,
    pub surface: Option<&'a str>,
    pub evidence_ids: &'a [&'a str],
}

impl Default for SoulPressure {
    fn default() -> Self {
        Self {
            truth: 0.2,
            care: 0.2,
            restraint: 0.2,
            memory_discretion: 0.2,
            continuity: 0.2,
            repair: 0.0,
            autonomy: 0.2,
            wonder: 0.0,
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SoulPressureInput<'a> {
    pub user_message: &'a str,
    pub identity: SoulIdentityCues<'a>,
    pub affect: &'a AffectReading,
    pub dialogue_act: DialogueAct,
    pub user_model: &'a UserMentalModel,
    pub relationship: Option<&'a RelationshipState>,
    pub recall_exposure: SoulRecallExposure,
    pub surface_exposure: SoulSurfaceExposure,
}

#[must_use]
pub(crate) fn derive_soul_pressure(input: SoulPressureInput<'_>) -> SoulPressure {
    derive_soul_pressure_with_topology(input, None)
}

#[must_use]
pub(crate) fn derive_soul_pressure_with_topology(
    input: SoulPressureInput<'_>,
    topology: Option<SoulTopologyCues>,
) -> SoulPressure {
    let lower = input.user_message.to_lowercase();
    let relationship = input.relationship;
    let repair_debt = relationship.map_or(0.0, |state| state.repair_debt);
    let unresolved_tension = relationship.map_or(0.0, |state| state.unresolved_tension);
    let affect = topology.is_none().then_some(input.affect);

    let mut pressure = SoulPressure {
        truth: if asks_identity_or_consciousness(&lower) {
            0.85
        } else {
            0.25
        },
        care: care_pressure(&lower, affect, input.user_model),
        restraint: restraint_pressure(&lower, affect, repair_debt, unresolved_tension),
        memory_discretion: memory_discretion_pressure(
            input.recall_exposure,
            input.surface_exposure,
        ),
        continuity: continuity_pressure(input.recall_exposure, relationship),
        repair: repair_pressure(&lower, input.dialogue_act, repair_debt, unresolved_tension),
        autonomy: autonomy_pressure(&lower, input.recall_exposure),
        wonder: wonder_pressure(&lower, input.user_model),
        notes: Vec::new(),
    };

    apply_identity_cues(&mut pressure, input.identity, &lower);
    if let Some(topology) = topology {
        apply_topology_cues(&mut pressure, topology);
    }
    pressure.notes = pressure_notes(&pressure, input.surface_exposure);
    pressure
}

#[must_use]
pub(crate) fn render_soul_pressure_block(pressure: &SoulPressure) -> String {
    if pressure.notes.is_empty() {
        return String::new();
    }

    let mut out = String::from("### Soul Pressure\n");
    for note in pressure.notes.iter().take(5) {
        out.push_str("- ");
        out.push_str(note);
        out.push('\n');
    }
    out
}

#[must_use]
pub(crate) fn generate_self_amendment_candidates(
    input: SelfAmendmentCandidateInput<'_>,
) -> Vec<SelfAmendmentCandidate> {
    let user_message = input.user_message.to_lowercase();
    if input.soul_pressure.repair < 0.85
        || input.assistant_response.trim().is_empty()
        || forget_or_reset_request(&user_message)
        || !explicit_assistant_correction_signal(&user_message)
    {
        return Vec::new();
    }

    let evidence_ids = sanitized_evidence_ids(input.evidence_ids);
    let candidate_id = format!(
        "self_amendment:{}:{}:{}:{}",
        input.tenant_id.unwrap_or("global"),
        sanitize_candidate_token(input.person_id),
        sanitize_candidate_token(input.surface.unwrap_or("unknown_surface")),
        "repair_practice"
    );

    vec![SelfAmendmentCandidate {
        candidate_id,
        kind: SelfAmendmentCandidateKind::RepairPractice,
        status: SelfAmendmentCandidateStatus::DryRunOnly,
        tenant_id: input.tenant_id.map(str::to_string),
        person_id: input.person_id.to_string(),
        surface: input.surface.unwrap_or("unknown_surface").to_string(),
        privacy: SelfAmendmentPrivacy::PrivateInternal,
        evidence_ids,
        reason: "explicit correction raised repair pressure; candidate is dry-run only".to_string(),
        proposed_amendment: "When corrected, acknowledge briefly, avoid defending, and preserve relationship distance."
            .to_string(),
    }]
}

mod soul_core_policy;
use soul_core_policy::{
    apply_identity_cues, apply_topology_cues, asks_identity_or_consciousness, autonomy_pressure,
    care_pressure, continuity_pressure, explicit_assistant_correction_signal,
    forget_or_reset_request, memory_discretion_pressure, pressure_notes, repair_pressure,
    restraint_pressure, sanitize_candidate_token, sanitized_evidence_ids, wonder_pressure,
};

#[cfg(test)]
mod tests;
