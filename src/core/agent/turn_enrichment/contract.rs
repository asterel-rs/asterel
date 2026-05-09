use crate::contracts::strings::data_model::{
    SLOT_CONVERSATION_ASSISTANT_RESP, SLOT_CONVERSATION_USER_MSG,
};
use crate::core::agent::turn_contract::{
    BehaviorPolicy, BehaviorSelection, CompanionTurnContract, ConversationMode, PersonaProjection,
    PickupDecision, PolicyRail, PolicyRailEnforcement, PolicyRailSet, ReplyShape, ReplyShapeKind,
    TurnEvidence, TurnEvidenceDecision, TurnEvidencePhase, TurnEvidenceProvenance,
    TurnEvidenceRecord, VerifierContract, VerifierStrategy, WritebackPlan, WritebackPlanSlot,
};

#[must_use]
pub fn compile_turn_contract(
    base_prompt: &str,
    policy_section: &str,
    persona_context: Option<&str>,
    decision_core_block: &str,
    temperature: f64,
) -> CompanionTurnContract {
    let rendered_prompt = assemble_system_prompt(
        base_prompt,
        policy_section,
        persona_context,
        decision_core_block,
    );

    CompanionTurnContract {
        pickup_decision: PickupDecision::Engage,
        conversation_mode: ConversationMode::Conversation,
        persona_projection: PersonaProjection {
            has_persona_context: persona_context.is_some(),
        },
        behavior_selection: BehaviorSelection {
            policy: BehaviorPolicy::ExistingPromptBlocks,
        },
        reply_shape: ReplyShape {
            shape: ReplyShapeKind::ExistingPromptBaseline,
        },
        verifier: VerifierContract {
            strategy: VerifierStrategy::ExistingPostProcessing,
        },
        writeback_plan: build_provisional_writeback_plan(),
        policy_rails: build_initial_policy_rails(),
        evidence: build_initial_turn_evidence(persona_context.is_some()),
        rendered_prompt,
        temperature,
    }
}

fn build_initial_policy_rails() -> PolicyRailSet {
    PolicyRailSet::new(vec![
        PolicyRail::new(
            TurnEvidencePhase::InputPickup,
            PolicyRailEnforcement::RuntimeGuard,
            "pickup_engage",
        ),
        PolicyRail::new(
            TurnEvidencePhase::Context,
            PolicyRailEnforcement::PromptGuidance,
            "context_minimization",
        ),
        PolicyRail::new(
            TurnEvidencePhase::Exposure,
            PolicyRailEnforcement::RuntimeGuard,
            "private_by_default",
        ),
        PolicyRail::new(
            TurnEvidencePhase::ToolAction,
            PolicyRailEnforcement::RuntimeGuard,
            "tool_middleware_policy",
        ),
        PolicyRail::new(
            TurnEvidencePhase::Output,
            PolicyRailEnforcement::RuntimeGuard,
            "response_finalizer",
        ),
    ])
}

fn build_initial_turn_evidence(has_persona_context: bool) -> TurnEvidence {
    TurnEvidence::new(vec![
        TurnEvidenceRecord::new(
            TurnEvidencePhase::InputPickup,
            TurnEvidenceDecision::Allow,
            "engage",
            "shared companion turn path engaged this message",
            TurnEvidenceProvenance::ContractCompiler,
        ),
        TurnEvidenceRecord::new(
            TurnEvidencePhase::Context,
            if has_persona_context {
                TurnEvidenceDecision::Allow
            } else {
                TurnEvidenceDecision::Defer
            },
            if has_persona_context {
                "persona_context_available"
            } else {
                "base_context_only"
            },
            if has_persona_context {
                "persona, memory, affect, or session context was available for this turn"
            } else {
                "no extra context block was available; base prompt remains the source"
            },
            TurnEvidenceProvenance::ContractCompiler,
        ),
        TurnEvidenceRecord::new(
            TurnEvidencePhase::Exposure,
            TurnEvidenceDecision::Allow,
            "private_by_default",
            "companion memory writeback plan defaults to private slots",
            TurnEvidenceProvenance::RuntimePolicy,
        ),
        TurnEvidenceRecord::new(
            TurnEvidencePhase::ToolAction,
            TurnEvidenceDecision::NotEvaluated,
            "tool_loop_policy_deferred",
            "tool execution policy is evaluated by the tool loop at call time",
            TurnEvidenceProvenance::ToolLoop,
        ),
        TurnEvidenceRecord::new(
            TurnEvidencePhase::Output,
            TurnEvidenceDecision::Defer,
            "existing_post_processing",
            "response finalization and verifier checks run after draft generation",
            TurnEvidenceProvenance::Verifier,
        ),
    ])
}

fn build_provisional_writeback_plan() -> WritebackPlan {
    WritebackPlan {
        slots: vec![
            WritebackPlanSlot {
                slot: SLOT_CONVERSATION_USER_MSG.to_string(),
                rationale: "conversation working-memory capture for latest user turn".to_string(),
                public: false,
                required: true,
            },
            WritebackPlanSlot {
                slot: SLOT_CONVERSATION_ASSISTANT_RESP.to_string(),
                rationale: "conversation working-memory capture for latest assistant turn"
                    .to_string(),
                public: false,
                required: true,
            },
            WritebackPlanSlot {
                slot: "user.*".to_string(),
                rationale: "persona writeback user-inference slots (phase-1 scoped)".to_string(),
                public: false,
                required: false,
            },
            WritebackPlanSlot {
                slot: "language.current".to_string(),
                rationale: "persona writeback memory inference (conversation language)".to_string(),
                public: false,
                required: false,
            },
            WritebackPlanSlot {
                slot: "topic.active".to_string(),
                rationale: "persona writeback memory inference (conversation topic)".to_string(),
                public: false,
                required: false,
            },
            WritebackPlanSlot {
                slot: "timezone.current".to_string(),
                rationale: "persona writeback memory inference (conversation timezone)".to_string(),
                public: false,
                required: false,
            },
        ],
    }
}

#[must_use]
pub fn render_system_prompt_from_contract(contract: &CompanionTurnContract) -> String {
    contract.rendered_prompt.clone()
}

fn assemble_system_prompt(
    base_prompt: &str,
    policy_section: &str,
    persona_context: Option<&str>,
    decision_core_block: &str,
) -> String {
    let capacity = base_prompt.len()
        + 2
        + policy_section.len()
        + 2
        + persona_context.map_or(0, |context| context.len() + 1)
        + decision_core_block.len();
    let mut system_prompt = String::with_capacity(capacity);
    system_prompt.push_str(base_prompt);
    system_prompt.push_str("\n\n");

    if !policy_section.is_empty() {
        system_prompt.push_str(policy_section);
        system_prompt.push('\n');
        if !policy_section.ends_with('\n') {
            system_prompt.push('\n');
        }
    }

    if let Some(context) = persona_context {
        system_prompt.push_str(context);
        system_prompt.push('\n');
    }
    system_prompt.push_str(decision_core_block);
    system_prompt
}
