use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::config::PersonaConfig;
use crate::contracts::channels::SurfaceRealizationPolicy;
use crate::contracts::ids::{EntityId, PersonId};
use crate::core::agent::response_audit::{
    BehaviorContract, ExposurePlanContract, ReplyShapeContract, ResponseContract,
};
use crate::core::agent::response_style::{ResponseMode, classify_response_mode};
use crate::core::agent::turn_executor::{
    TurnExecutionPlan, TurnPostExecutionSeed, TurnWorkingMemorySpec,
    execute_turn_plan_with_naturalness_context, run_enriched_turn,
};
use crate::core::agent::{
    AgentStateNotifier, PreTurnInput, TurnExecutionOutcome, TurnHistoryAdapter,
    TurnTranscriptAdapter, naturalness_relationship_distance_from_state,
    naturalness_relationship_surface_from_contract,
};
use crate::core::memory::Memory;
use crate::core::persona::person_identity::person_entity_id;
use crate::core::persona::relationship::load_relationship_for_entity;
use crate::core::persona::soul_core::SelfAmendmentCandidateSink;
use crate::core::providers::InferenceOpts;
use crate::core::providers::response::ContentBlock;
use crate::core::providers::response::ProviderMessage;
use crate::core::providers::streaming::StreamSink;
use crate::core::providers::traits::Provider;
use crate::core::sessions::SessionOrchestrator;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::registry::ToolRegistry;
use crate::security::policy::TenantPolicyContext;

use super::{PolicyAssemblyInput, build_policy_section};

pub(crate) struct CompanionTurnRuntimeDeps<'a> {
    pub(crate) mem: Arc<dyn Memory>,
    pub(crate) persona_config: &'a PersonaConfig,
    pub(crate) session_manager: Option<&'a SessionOrchestrator>,
    pub(crate) working_memory_capacity: usize,
    pub(crate) registry: Arc<ToolRegistry>,
    pub(crate) max_tool_iterations: u32,
    pub(crate) loop_detection: crate::config::LoopDetectionConfig,
    pub(crate) response_finalization_enabled: bool,
    pub(crate) naturalness_gate_enabled: bool,
    pub(crate) self_amendment_candidate_sink: Option<Arc<dyn SelfAmendmentCandidateSink>>,
}

pub(crate) struct CompanionTransportTurnRequest<'a> {
    pub(crate) runtime: CompanionTurnRuntimeDeps<'a>,
    pub(crate) workspace_dir: &'a Path,
    pub(crate) base_prompt: &'a str,
    pub(crate) user_message: &'a str,
    pub(crate) entity_id: &'a str,
    pub(crate) person_id: &'a str,
    pub(crate) base_temperature: f64,
    pub(crate) policy_context: &'a TenantPolicyContext,
    pub(crate) session_surface: Option<&'a str>,
    pub(crate) channel_context_hint: Option<&'a str>,
    pub(crate) surface_realization_policy: Option<&'a SurfaceRealizationPolicy>,
    pub(crate) session_owner_scope: Option<&'a str>,
    pub(crate) working_memory_session_id: &'a str,
    pub(crate) history_channel_name: &'a str,
    pub(crate) history_session_key: Option<&'a str>,
    pub(crate) history_tenant_id: Option<&'a str>,
    pub(crate) history_max_tokens: usize,
    pub(crate) fallback_history: &'a [ProviderMessage],
    pub(crate) provider: &'a dyn Provider,
    pub(crate) image_content: &'a [ContentBlock],
    pub(crate) model: &'a str,
    pub(crate) inference_options: Option<InferenceOpts>,
    pub(crate) ctx: &'a ExecutionContext,
    pub(crate) stream_sink: Option<Arc<dyn StreamSink>>,
    pub(crate) state_notifier: Option<Arc<dyn AgentStateNotifier>>,
    pub(crate) transcript_log_target: &'a str,
}

pub(crate) struct CompanionTurnServiceRequest<'a> {
    pub(crate) mem: Arc<dyn Memory>,
    pub(crate) workspace_dir: &'a Path,
    pub(crate) base_prompt: &'a str,
    pub(crate) user_message: &'a str,
    pub(crate) entity_id: &'a str,
    pub(crate) person_id: &'a str,
    pub(crate) base_temperature: f64,
    pub(crate) policy_context: &'a TenantPolicyContext,
    pub(crate) persona_config: &'a PersonaConfig,
    pub(crate) session_manager: Option<&'a SessionOrchestrator>,
    pub(crate) session_surface: Option<&'a str>,
    pub(crate) channel_context_hint: Option<&'a str>,
    pub(crate) surface_realization_policy: Option<&'a SurfaceRealizationPolicy>,
    pub(crate) session_owner_scope: Option<&'a str>,
    pub(crate) policy_section: &'a str,
    pub(crate) working_memory_session_id: &'a str,
    pub(crate) working_memory_capacity: usize,
    pub(crate) registry: Arc<ToolRegistry>,
    pub(crate) max_tool_iterations: u32,
    pub(crate) loop_detection: crate::config::LoopDetectionConfig,
    pub(crate) history: TurnHistoryAdapter<'a>,
    pub(crate) provider: &'a dyn Provider,
    pub(crate) response_finalization_enabled: bool,
    pub(crate) naturalness_gate_enabled: bool,
    pub(crate) self_amendment_candidate_sink: Option<Arc<dyn SelfAmendmentCandidateSink>>,
    pub(crate) image_content: &'a [ContentBlock],
    pub(crate) model: &'a str,
    pub(crate) inference_options: Option<InferenceOpts>,
    pub(crate) ctx: &'a ExecutionContext,
    pub(crate) stream_sink: Option<Arc<dyn StreamSink>>,
    pub(crate) state_notifier: Option<Arc<dyn AgentStateNotifier>>,
    pub(crate) transcript_log_target: &'a str,
}

/// Run one companion turn through the shared transport-facing path.
///
/// This centralizes pre-turn enrichment, working-memory materialization,
/// shared `TurnExecutor` construction, and post-turn background hooks so
/// gateway HTTP, gateway WebSocket, and channel adapters stay behaviorally aligned.
#[allow(clippy::too_many_lines)]
pub(crate) async fn run_companion_turn(
    request: CompanionTurnServiceRequest<'_>,
) -> Result<TurnExecutionOutcome> {
    let response_contract = response_contract_for_surface(
        request.session_surface,
        request.channel_context_hint,
        request.surface_realization_policy,
        request.user_message,
    );
    let naturalness_relationship_distance =
        transport_naturalness_relationship_distance(&request, &response_contract).await;
    let pre_turn = PreTurnInput {
        mem: request.mem.as_ref(),
        workspace_dir: request.workspace_dir,
        base_prompt: request.base_prompt,
        user_message: request.user_message,
        entity_id: request.entity_id,
        person_id: request.person_id,
        base_temperature: request.base_temperature,
        policy_context: request.policy_context,
        recall_min_confidence: None,
        persona_config: Some(request.persona_config),
        session_manager: request.session_manager,
        session_surface: request.session_surface,
        is_direct_address: is_direct_address_context(
            request.session_surface,
            request.channel_context_hint,
        ),
        session_owner_scope: request.session_owner_scope,
        session_id: request.ctx.session_id.as_deref(),
        policy_section: request.policy_section,
        exposure_plan: Some(response_contract.exposure_plan),
        working_memory: None,
    };

    let working_memory_spec = TurnWorkingMemorySpec {
        mem: request.mem.as_ref(),
        session_id: request.working_memory_session_id,
        entity_id: request.entity_id,
        user_message: request.user_message,
        capacity: request.working_memory_capacity,
    };

    let post_turn_seed = TurnPostExecutionSeed {
        mem: Arc::clone(&request.mem),
        person_id: PersonId::new(request.person_id),
        person_entity_id: EntityId::new(
            request
                .policy_context
                .scope_entity_id(&person_entity_id(request.person_id)),
        ),
        user_message: request.user_message.to_string(),
        tenant_id: request.policy_context.tenant_id.clone(),
        surface: request
            .session_surface
            .or(request.channel_context_hint)
            .map(str::to_string),
        enable_self_amendment_candidates: request.persona_config.enable_self_amendment_candidates,
        self_amendment_candidate_sink: request.self_amendment_candidate_sink.clone(),
        contract: crate::core::agent::turn_contract::CompanionTurnContract::default(),
        observer: Arc::clone(&request.ctx.observer),
    };
    run_enriched_turn(
        pre_turn,
        Some(working_memory_spec),
        post_turn_seed,
        |system_prompt, temperature| async move {
            let turn_ctx = execution_delegation_context(request.ctx, &system_prompt);
            let stream_sink = stream_sink_for_response_contract(
                request.stream_sink.clone(),
                response_contract,
                request.response_finalization_enabled,
                request.naturalness_gate_enabled,
            );
            execute_turn_plan_with_naturalness_context(
                TurnExecutionPlan {
                    registry: Arc::clone(&request.registry),
                    max_iterations: request.max_tool_iterations,
                    loop_detection: request.loop_detection.clone(),
                    history: request.history,
                    provider: request.provider,
                    system_prompt,
                    user_message: request.user_message,
                    response_finalization_enabled: request.response_finalization_enabled,
                    response_contract: Some(&response_contract),
                    image_content: request.image_content,
                    model: request.model,
                    temperature,
                    inference_options: request.inference_options,
                    ctx: &turn_ctx,
                    stream_sink,
                    state_notifier: request.state_notifier.clone(),
                    transcript: TurnTranscriptAdapter {
                        session_manager: request.session_manager,
                        user_message: request.user_message,
                        log_target: request.transcript_log_target,
                    },
                },
                request.naturalness_gate_enabled,
                naturalness_relationship_distance,
            )
            .await
        },
    )
    .await
}

async fn transport_naturalness_relationship_distance(
    request: &CompanionTurnServiceRequest<'_>,
    response_contract: &ResponseContract,
) -> crate::core::agent::naturalness_gate::RelationshipDistance {
    if !request.naturalness_gate_enabled {
        return crate::core::agent::naturalness_gate::RelationshipDistance::Unknown;
    }

    let scoped_person_entity = request
        .policy_context
        .scope_entity_id(&person_entity_id(request.person_id));
    let relationship = load_relationship_for_entity(
        request.mem.as_ref(),
        &scoped_person_entity,
        request.person_id,
    )
    .await
    .ok()
    .flatten();
    naturalness_relationship_distance_from_state(
        relationship.as_ref(),
        naturalness_relationship_surface_from_contract(Some(response_contract)),
    )
}

pub(crate) async fn run_transport_companion_turn(
    request: CompanionTransportTurnRequest<'_>,
) -> Result<TurnExecutionOutcome> {
    let response_contract = response_contract_for_surface(
        request.session_surface,
        request.channel_context_hint,
        request.surface_realization_policy,
        request.user_message,
    );
    let policy_section = build_policy_section(&PolicyAssemblyInput {
        session_control_block: "",
        character_summary: None,
        surface_context_hint: request.channel_context_hint,
        surface_realization_policy: request.surface_realization_policy,
        response_contract: Some(&response_contract),
    });

    run_companion_turn(CompanionTurnServiceRequest {
        mem: Arc::clone(&request.runtime.mem),
        workspace_dir: request.workspace_dir,
        base_prompt: request.base_prompt,
        user_message: request.user_message,
        entity_id: request.entity_id,
        person_id: request.person_id,
        base_temperature: request.base_temperature,
        policy_context: request.policy_context,
        persona_config: request.runtime.persona_config,
        session_manager: request.runtime.session_manager,
        session_surface: request.session_surface,
        channel_context_hint: request.channel_context_hint,
        surface_realization_policy: request.surface_realization_policy,
        session_owner_scope: request.session_owner_scope,
        policy_section: &policy_section,
        working_memory_session_id: request.working_memory_session_id,
        working_memory_capacity: request.runtime.working_memory_capacity,
        registry: Arc::clone(&request.runtime.registry),
        max_tool_iterations: request.runtime.max_tool_iterations,
        loop_detection: request.runtime.loop_detection,
        history: TurnHistoryAdapter {
            session_manager: request.runtime.session_manager,
            channel_name: request.history_channel_name,
            session_key: request.history_session_key,
            tenant_id: request.history_tenant_id,
            max_tokens: request.history_max_tokens,
            fallback_history: request.fallback_history,
        },
        provider: request.provider,
        response_finalization_enabled: request.runtime.response_finalization_enabled,
        naturalness_gate_enabled: request.runtime.naturalness_gate_enabled,
        self_amendment_candidate_sink: request.runtime.self_amendment_candidate_sink.clone(),
        image_content: request.image_content,
        model: request.model,
        inference_options: request.inference_options,
        ctx: request.ctx,
        stream_sink: request.stream_sink,
        state_notifier: request.state_notifier,
        transcript_log_target: request.transcript_log_target,
    })
    .await
}

fn response_contract_for_surface(
    session_surface: Option<&str>,
    channel_context_hint: Option<&str>,
    surface_realization_policy: Option<&SurfaceRealizationPolicy>,
    user_message: &str,
) -> ResponseContract {
    let private_context = is_private_companion_context(
        session_surface,
        channel_context_hint,
        surface_realization_policy,
    );
    ResponseContract {
        reply_shape: if private_context {
            ReplyShapeContract::Standard
        } else {
            ReplyShapeContract::Compact
        },
        exposure_plan: if private_context {
            ExposurePlanContract::PrivateAllowed
        } else {
            ExposurePlanContract::PublicSafe
        },
        behavior: match classify_response_mode(user_message) {
            ResponseMode::Conversation => BehaviorContract::Conversational,
            ResponseMode::Explanation | ResponseMode::Task | ResponseMode::Report => {
                BehaviorContract::Explanatory
            }
        },
    }
}

fn is_private_companion_context(
    session_surface: Option<&str>,
    channel_context_hint: Option<&str>,
    surface_realization_policy: Option<&SurfaceRealizationPolicy>,
) -> bool {
    if let Some(policy) = surface_realization_policy {
        return !policy.is_public;
    }
    if matches!(session_surface, Some("gateway_http" | "gateway_ws")) {
        return true;
    }
    if channel_context_hint.is_some_and(|hint| {
        let normalized = hint.to_ascii_lowercase();
        hint.contains("Channel Context: DM")
            || normalized.trim() == "dm"
            || normalized.contains("direct message")
    }) {
        return true;
    }
    false
}

fn is_direct_address_context(
    session_surface: Option<&str>,
    channel_context_hint: Option<&str>,
) -> bool {
    if matches!(session_surface, Some("gateway_http" | "gateway_ws")) {
        return true;
    }
    let Some(hint) = channel_context_hint else {
        return true;
    };
    let normalized = hint.to_ascii_lowercase();
    !(normalized.contains("ambient")
        || normalized.contains("thread continuation")
        || normalized.contains("passive"))
}

fn stream_sink_for_response_contract(
    stream_sink: Option<Arc<dyn StreamSink>>,
    response_contract: ResponseContract,
    response_finalization_enabled: bool,
    naturalness_gate_enabled: bool,
) -> Option<Arc<dyn StreamSink>> {
    if response_finalization_enabled
        || naturalness_gate_enabled
        || matches!(
            response_contract.exposure_plan,
            ExposurePlanContract::PublicSafe
        )
    {
        None
    } else {
        stream_sink
    }
}

fn execution_delegation_context(ctx: &ExecutionContext, system_prompt: &str) -> ExecutionContext {
    let mut transport_ctx = ctx.clone();
    if !system_prompt.trim().is_empty() {
        transport_ctx.delegation_system_prompt = Some(system_prompt.to_string());
    }
    transport_ctx
}

#[cfg(test)]
mod tests {
    use super::{
        SurfaceRealizationPolicy, execution_delegation_context, is_direct_address_context,
        is_private_companion_context, response_contract_for_surface,
        stream_sink_for_response_contract,
    };
    use crate::core::agent::response_audit::{
        BehaviorContract, ExposurePlanContract, ReplyShapeContract,
    };
    use crate::core::providers::streaming::{StreamEvent, StreamSink};
    use crate::core::tools::middleware::ExecutionContext;
    use crate::security::SecurityPolicy;
    use std::sync::Arc;

    struct TestStreamSink;

    impl StreamSink for TestStreamSink {
        fn on_event<'a>(
            &'a self,
            _event: &'a StreamEvent,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    #[test]
    fn execution_delegation_context_sets_final_prompt_when_missing() {
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));

        let scoped = execution_delegation_context(&ctx, "gateway final prompt");

        assert_eq!(
            scoped.delegation_system_prompt.as_deref(),
            Some("gateway final prompt")
        );
    }

    #[test]
    fn execution_delegation_context_overrides_existing_prompt_with_final_prompt() {
        let mut ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        ctx.delegation_system_prompt = Some("base prompt".to_string());

        let scoped = execution_delegation_context(&ctx, "gateway final prompt");

        assert_eq!(
            scoped.delegation_system_prompt.as_deref(),
            Some("gateway final prompt")
        );
    }

    #[test]
    fn response_contract_allows_private_memory_only_in_dm_or_gateway_contexts() {
        assert!(is_private_companion_context(
            Some("discord"),
            Some("[Channel Context: DM — conversational tone, natural length]"),
            None
        ));
        assert!(is_private_companion_context(
            Some("gateway_http"),
            None,
            None
        ));
        assert!(!is_private_companion_context(
            Some("discord"),
            Some("[Channel Context: Ambient pickup — brief, useful, and easy to ignore]"),
            None
        ));
        let private_policy = SurfaceRealizationPolicy::discord_dm();
        assert!(is_private_companion_context(
            Some("discord"),
            None,
            Some(&private_policy)
        ));

        let dm = response_contract_for_surface(
            Some("discord"),
            Some("[Channel Context: DM — conversational tone, natural length]"),
            None,
            "なるほど",
        );
        assert_eq!(dm.exposure_plan, ExposurePlanContract::PrivateAllowed);
        assert_eq!(dm.reply_shape, ReplyShapeContract::Standard);

        let public_policy = SurfaceRealizationPolicy::discord_public();
        let public = response_contract_for_surface(
            Some("discord"),
            Some("[Channel Context: Direct mention — concise, relevant to channel topic]"),
            Some(&public_policy),
            "why did this fail?",
        );
        assert_eq!(public.exposure_plan, ExposurePlanContract::PublicSafe);
        assert_eq!(public.reply_shape, ReplyShapeContract::Compact);
        assert_eq!(public.behavior, BehaviorContract::Explanatory);

        let typed_public_over_legacy_dm_hint = response_contract_for_surface(
            Some("discord"),
            Some("[Channel Context: DM — legacy text hint]"),
            Some(&public_policy),
            "hello",
        );
        assert_eq!(
            typed_public_over_legacy_dm_hint.exposure_plan,
            ExposurePlanContract::PublicSafe
        );
    }

    #[test]
    fn direct_address_context_distinguishes_ambient_channel_turns() {
        assert!(is_direct_address_context(Some("gateway_http"), None));
        assert!(is_direct_address_context(
            Some("discord"),
            Some("[Channel Context: DM — conversational tone, natural length]")
        ));
        assert!(is_direct_address_context(
            Some("discord"),
            Some("[Channel Context: Direct mention — concise, relevant to channel topic]")
        ));
        assert!(is_direct_address_context(Some("twitter"), Some("mention")));
        assert!(is_direct_address_context(
            Some("discord"),
            Some("discord:context_menu:summarize")
        ));
        assert!(!is_direct_address_context(
            Some("discord"),
            Some("[Channel Context: Thread continuation — stay on topic, build on prior context]")
        ));
        assert!(!is_direct_address_context(
            Some("discord"),
            Some("[Channel Context: Ambient pickup — brief, useful, and easy to ignore]")
        ));
        assert!(!is_direct_address_context(
            Some("discord"),
            Some(
                "[Channel Context: Ambient event — react only if useful, brief, and easy to ignore]"
            )
        ));
    }

    #[test]
    fn pre_send_verification_contract_disables_streaming_until_after_verification() {
        let sink: Arc<dyn StreamSink> = Arc::new(TestStreamSink);
        let public = response_contract_for_surface(
            Some("discord"),
            Some("[Channel Context: Direct mention — concise, relevant to channel topic]"),
            None,
            "hello",
        );
        let private = response_contract_for_surface(Some("gateway_ws"), None, None, "hello");

        assert!(
            stream_sink_for_response_contract(Some(Arc::clone(&sink)), public, false, false)
                .is_none()
        );
        assert!(
            stream_sink_for_response_contract(Some(Arc::clone(&sink)), private, false, true)
                .is_none()
        );
        assert!(
            stream_sink_for_response_contract(Some(Arc::clone(&sink)), private, true, false)
                .is_none()
        );
        assert!(stream_sink_for_response_contract(Some(sink), private, false, false).is_some());
    }
}
