//! Companion turn contract types.
//!
//! This schema captures pre-turn planning signals that can be compiled into
//! transport-ready execution inputs.

/// Canonical contract compiled before each companion turn.
#[derive(Debug, Clone, Default)]
pub struct CompanionTurnContract {
    /// Whether this incoming message should be picked up as a companion turn.
    pub pickup_decision: PickupDecision,
    /// The high-level conversation mode for this turn.
    pub conversation_mode: ConversationMode,
    /// Persona projection scaffold used when rendering the prompt.
    pub persona_projection: PersonaProjection,
    /// Behavior selection directives derived for this turn.
    pub behavior_selection: BehaviorSelection,
    /// Expected reply-shape guidance for the model output.
    pub reply_shape: ReplyShape,
    /// Verifier expectations for post-generation checks.
    pub verifier: VerifierContract,
    /// Planned writeback scope for post-turn memory updates.
    pub writeback_plan: WritebackPlan,
    /// Policy rails active for this turn, separated by intervention point.
    pub policy_rails: PolicyRailSet,
    /// Typed evidence emitted while compiling the turn contract.
    pub evidence: TurnEvidence,
    /// Prompt rendered from this contract.
    pub rendered_prompt: String,
    /// Effective sampling temperature selected for the turn.
    pub temperature: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickupDecision {
    #[default]
    Engage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConversationMode {
    #[default]
    Conversation,
}

#[derive(Debug, Clone, Default)]
pub struct PersonaProjection {
    /// Whether persona context blocks were available for this turn.
    pub has_persona_context: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BehaviorSelection {
    /// Existing behavior policy rendered into the prompt.
    pub policy: BehaviorPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BehaviorPolicy {
    #[default]
    ExistingPromptBlocks,
}

#[derive(Debug, Clone, Default)]
pub struct ReplyShape {
    /// Existing response baseline shape currently enforced by prompt blocks.
    pub shape: ReplyShapeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplyShapeKind {
    #[default]
    ExistingPromptBaseline,
}

#[derive(Debug, Clone, Default)]
pub struct VerifierContract {
    /// Existing verifier approach for this phase.
    pub strategy: VerifierStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerifierStrategy {
    #[default]
    ExistingPostProcessing,
}

/// Candidate slot definition for provisional writeback planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritebackPlanSlot {
    /// Candidate slot key or prefix (`*` suffix means prefix match).
    pub slot: String,
    /// Why this slot is part of the writeback contract.
    pub rationale: String,
    /// Whether this slot can be exposed to user-facing surfaces.
    pub public: bool,
    /// Whether this slot is required (`true`) or optional (`false`).
    pub required: bool,
}

/// Provisional pre-turn writeback plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WritebackPlan {
    /// Candidate slots and contract metadata.
    pub slots: Vec<WritebackPlanSlot>,
}

/// Policy rails attached to the turn contract.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PolicyRailSet {
    /// Rails in canonical intervention order.
    pub rails: Vec<PolicyRail>,
}

impl PolicyRailSet {
    #[must_use]
    pub fn new(rails: Vec<PolicyRail>) -> Self {
        Self { rails }
    }

    #[must_use]
    pub fn has_phase(&self, phase: TurnEvidencePhase) -> bool {
        self.rails.iter().any(|rail| rail.phase == phase)
    }
}

/// One policy rail bound to a canonical intervention point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRail {
    pub phase: TurnEvidencePhase,
    pub enforcement: PolicyRailEnforcement,
    pub reason_code: &'static str,
}

impl PolicyRail {
    #[must_use]
    pub const fn new(
        phase: TurnEvidencePhase,
        enforcement: PolicyRailEnforcement,
        reason_code: &'static str,
    ) -> Self {
        Self {
            phase,
            enforcement,
            reason_code,
        }
    }
}

/// How a policy rail is enforced at this point in the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyRailEnforcement {
    PromptGuidance,
    RuntimeGuard,
    Deferred,
}

impl PolicyRailEnforcement {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PromptGuidance => "prompt_guidance",
            Self::RuntimeGuard => "runtime_guard",
            Self::Deferred => "deferred",
        }
    }
}

/// Typed evidence attached to a companion turn contract.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnEvidence {
    /// Phase-level evidence records in emission order.
    pub records: Vec<TurnEvidenceRecord>,
}

impl TurnEvidence {
    #[must_use]
    pub fn new(records: Vec<TurnEvidenceRecord>) -> Self {
        Self { records }
    }

    #[must_use]
    pub fn has_phase(&self, phase: TurnEvidencePhase) -> bool {
        self.records.iter().any(|record| record.phase == phase)
    }
}

/// One phase-level evidence record for the companion turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEvidenceRecord {
    pub phase: TurnEvidencePhase,
    pub decision: TurnEvidenceDecision,
    pub reason_code: &'static str,
    pub summary: String,
    pub provenance: TurnEvidenceProvenance,
}

impl TurnEvidenceRecord {
    #[must_use]
    pub fn new(
        phase: TurnEvidencePhase,
        decision: TurnEvidenceDecision,
        reason_code: &'static str,
        summary: impl Into<String>,
        provenance: TurnEvidenceProvenance,
    ) -> Self {
        Self {
            phase,
            decision,
            reason_code,
            summary: summary.into(),
            provenance,
        }
    }
}

/// Canonical phase where evidence was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEvidencePhase {
    InputPickup,
    Context,
    Exposure,
    ToolAction,
    Output,
}

impl TurnEvidencePhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InputPickup => "input_pickup",
            Self::Context => "context",
            Self::Exposure => "exposure",
            Self::ToolAction => "tool_action",
            Self::Output => "output",
        }
    }
}

/// Decision represented by a phase evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEvidenceDecision {
    Allow,
    Defer,
    Deny,
    NotEvaluated,
}

impl TurnEvidenceDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Defer => "defer",
            Self::Deny => "deny",
            Self::NotEvaluated => "not_evaluated",
        }
    }
}

/// Source of the evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEvidenceProvenance {
    ContractCompiler,
    RuntimePolicy,
    MemoryRecall,
    AffectRuntime,
    Verifier,
    ToolLoop,
}

impl TurnEvidenceProvenance {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContractCompiler => "contract_compiler",
            Self::RuntimePolicy => "runtime_policy",
            Self::MemoryRecall => "memory_recall",
            Self::AffectRuntime => "affect_runtime",
            Self::Verifier => "verifier",
            Self::ToolLoop => "tool_loop",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PolicyRail, PolicyRailEnforcement, PolicyRailSet, TurnEvidence, TurnEvidenceDecision,
        TurnEvidencePhase, TurnEvidenceProvenance,
    };

    #[test]
    fn turn_evidence_tracks_phase_coverage() {
        let evidence = TurnEvidence::new(vec![
            super::TurnEvidenceRecord::new(
                TurnEvidencePhase::InputPickup,
                TurnEvidenceDecision::Allow,
                "direct_request",
                "direct user turn",
                TurnEvidenceProvenance::ContractCompiler,
            ),
            super::TurnEvidenceRecord::new(
                TurnEvidencePhase::Output,
                TurnEvidenceDecision::Defer,
                "existing_post_processing",
                "response finalizer owns output verification",
                TurnEvidenceProvenance::Verifier,
            ),
        ]);

        assert!(evidence.has_phase(TurnEvidencePhase::InputPickup));
        assert!(evidence.has_phase(TurnEvidencePhase::Output));
        assert!(!evidence.has_phase(TurnEvidencePhase::Context));
    }

    #[test]
    fn evidence_enums_have_stable_trace_labels() {
        assert_eq!(TurnEvidencePhase::InputPickup.as_str(), "input_pickup");
        assert_eq!(TurnEvidenceDecision::NotEvaluated.as_str(), "not_evaluated");
        assert_eq!(
            TurnEvidenceProvenance::ContractCompiler.as_str(),
            "contract_compiler"
        );
        assert_eq!(
            PolicyRailEnforcement::RuntimeGuard.as_str(),
            "runtime_guard"
        );
    }

    #[test]
    fn policy_rail_set_tracks_intervention_points() {
        let rails = PolicyRailSet::new(vec![
            PolicyRail::new(
                TurnEvidencePhase::InputPickup,
                PolicyRailEnforcement::RuntimeGuard,
                "pickup_gate",
            ),
            PolicyRail::new(
                TurnEvidencePhase::Output,
                PolicyRailEnforcement::RuntimeGuard,
                "response_finalizer",
            ),
        ]);

        assert!(rails.has_phase(TurnEvidencePhase::InputPickup));
        assert!(rails.has_phase(TurnEvidencePhase::Output));
        assert!(!rails.has_phase(TurnEvidencePhase::ToolAction));
    }
}
