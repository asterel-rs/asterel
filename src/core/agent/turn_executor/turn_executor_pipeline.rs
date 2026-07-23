use super::turn_executor_metrics::{
    emit_turn_evidence_trace, emit_turn_output_trace, run_post_turn_processing,
};
use super::{
    AgentStateNotifier, Arc, CompanionTurnContract, ContentBlock, ContractMismatchReason, EntityId,
    ExecutionContext, Future, InferenceOpts, LoopDetectionConfig, Memory,
    NaturalnessFinalizationContext, Observer, PersonId, PreTurnInput, Provider, ProviderMessage,
    RelationshipDistance, ResponseContract, ResponseFinalizationRequest, ResponseMode, Result,
    SelfAmendmentCandidateSink, SessionId, SessionOrchestrator, StreamSink, ToolLoop,
    ToolLoopResult, ToolLoopRunParams, ToolRegistry, classify_response_mode, enrich_pre_turn,
    finalize_response_contextual_with_context, finalize_response_with_context,
    load_provider_history_async, materialize_working_memory, naturalness_affect_from_text,
    persist_tool_loop_turn_async,
};

/// Reusable turn execution engine shared across all delivery surfaces.
pub struct TurnExecutor {
    registry: Arc<ToolRegistry>,
    max_iterations: u32,
    loop_detection: LoopDetectionConfig,
    naturalness_gate_enabled: bool,
    naturalness_relationship_distance: RelationshipDistance,
}

/// Full input bundle for a single turn execution.
pub struct TurnExecutionRequest<'a> {
    /// History loading configuration for this turn.
    pub history: TurnHistoryAdapter<'a>,
    /// Inference and tool-loop configuration for this turn.
    pub run: TurnRunAdapter<'a>,
    /// Transcript persistence configuration for this turn.
    pub transcript: TurnTranscriptAdapter<'a>,
}

/// Controls how prior conversation history is loaded for a turn.
pub struct TurnHistoryAdapter<'a> {
    /// Optional session orchestrator; if `None`, history is skipped.
    pub session_manager: Option<&'a SessionOrchestrator>,
    /// Channel name used as the session lookup key prefix.
    pub channel_name: &'a str,
    /// Per-conversation session key (e.g. a user or channel ID).
    pub session_key: Option<&'a str>,
    /// Tenant scope used for multi-tenant session isolation.
    pub tenant_id: Option<&'a str>,
    /// Token budget cap applied when trimming loaded history.
    pub max_tokens: usize,
    /// Preloaded conversation history used when no session-backed history exists.
    pub fallback_history: &'a [ProviderMessage],
}

/// The inference parameters passed to the tool loop for a turn.
pub struct TurnRunAdapter<'a> {
    /// Provider used for assistant inference calls.
    pub provider: &'a dyn Provider,
    /// Base system prompt (may be further enriched before use).
    pub system_prompt: &'a str,
    /// Raw user message text.
    pub user_message: &'a str,
    /// When `true`, the response is passed through `finalize_response` before
    /// being persisted and returned to the caller.
    pub response_finalization_enabled: bool,
    /// Optional verifier contract applied before style audit/micro-rewrite.
    pub response_contract: Option<&'a ResponseContract>,
    /// Optional image content attached to the user turn.
    pub image_content: &'a [ContentBlock],
    /// Model identifier string.
    pub model: &'a str,
    /// Sampling temperature.
    pub temperature: f64,
    /// Provider-specific inference overrides.
    pub inference_options: Option<InferenceOpts>,
    /// Tool execution and security context.
    pub ctx: &'a ExecutionContext,
    /// Optional streaming sink for incremental output delivery.
    pub stream_sink: Option<Arc<dyn StreamSink>>,
    /// Optional notifier for broadcasting agent state transitions.
    pub state_notifier: Option<Arc<dyn AgentStateNotifier>>,
}

/// Controls how the completed turn is persisted to the transcript store.
pub struct TurnTranscriptAdapter<'a> {
    /// Optional session orchestrator; if `None`, persistence is skipped.
    pub session_manager: Option<&'a SessionOrchestrator>,
    /// Raw user message text to record in the transcript.
    pub user_message: &'a str,
    /// Structured log target label used in tracing spans.
    pub log_target: &'a str,
}

/// Returned by `TurnExecutor::execute` after a turn completes.
pub struct TurnExecutionOutcome {
    /// Session ID assigned or resolved during this turn.
    pub session_id: Option<SessionId>,
    /// Full tool loop result including the final text and tool call records.
    pub result: ToolLoopResult,
}

/// Parameters for materialising the working memory view before a turn.
pub struct TurnWorkingMemorySpec<'a> {
    /// Memory backend to recall from.
    pub mem: &'a dyn Memory,
    /// Session ID used as a working-memory namespace.
    pub session_id: &'a str,
    /// Entity ID for scoped recall.
    pub entity_id: &'a str,
    /// User message used as the recall query.
    pub user_message: &'a str,
    /// Maximum number of items to materialise.
    pub capacity: usize,
}

/// Data needed to complete post-turn processing.
pub struct TurnPostExecutionSeed {
    /// Memory backend for persisting post-turn events.
    pub mem: Arc<dyn Memory>,
    /// Whether conversation-derived memory writes are enabled.
    pub auto_save: bool,
    /// Person ID for relationship and working-memory updates.
    pub person_id: PersonId,
    /// Canonical person entity ID for transport-facing post-turn memory writes.
    pub person_entity_id: EntityId,
    /// User message recorded as a working-memory event.
    pub user_message: String,
    /// Tenant scope attached to this transport turn, if tenant mode is active.
    pub tenant_id: Option<String>,
    /// Surface/channel where post-turn dry-run hooks run.
    pub surface: Option<String>,
    /// Whether dry-run self-amendment candidate generation is enabled.
    pub enable_self_amendment_candidates: bool,
    /// Optional ephemeral review sink for dry-run self-amendment candidates.
    pub(crate) self_amendment_candidate_sink: Option<Arc<dyn SelfAmendmentCandidateSink>>,
    /// Provisional contract metadata built at pre-turn.
    pub contract: CompanionTurnContract,
    /// Runtime observer for structured turn trace events.
    pub observer: Arc<dyn Observer>,
}

/// Pre-assembled execution plan passed to [`execute_turn_plan`].
///
/// Combines all configuration that would otherwise be spread across multiple
/// adapter structs when the caller needs to build the plan before executing.
pub struct TurnExecutionPlan<'a> {
    pub registry: Arc<ToolRegistry>,
    pub max_iterations: u32,
    pub loop_detection: LoopDetectionConfig,
    pub history: TurnHistoryAdapter<'a>,
    pub provider: &'a dyn Provider,
    /// Fully assembled system prompt (enriched by the caller before passing).
    pub system_prompt: String,
    pub user_message: &'a str,
    pub response_finalization_enabled: bool,
    pub response_contract: Option<&'a ResponseContract>,
    pub image_content: &'a [ContentBlock],
    pub model: &'a str,
    pub temperature: f64,
    pub inference_options: Option<InferenceOpts>,
    pub ctx: &'a ExecutionContext,
    pub stream_sink: Option<Arc<dyn StreamSink>>,
    pub state_notifier: Option<Arc<dyn AgentStateNotifier>>,
    pub transcript: TurnTranscriptAdapter<'a>,
}

/// Execute a turn after enriching the system prompt and temperature, then
/// complete post-turn hooks before returning delivery-ready output.
///
/// `execute_turn` is a closure that receives the enriched `(system_prompt,
/// temperature)` pair and drives the actual tool loop. This separation lets
/// callers plug in custom enrichment without reimplementing the hook logic.
///
/// # Errors
/// Returns an error if pre-turn execution or the provided execution future fails.
pub async fn run_enriched_turn<F, Fut>(
    mut pre_turn: PreTurnInput<'_>,
    working_memory_spec: Option<TurnWorkingMemorySpec<'_>>,
    post_turn_seed: TurnPostExecutionSeed,
    execute_turn: F,
) -> Result<TurnExecutionOutcome>
where
    F: FnOnce(String, f64) -> Fut,
    Fut: Future<Output = Result<TurnExecutionOutcome>>,
{
    let working_memory = match working_memory_spec {
        Some(spec) => Some(
            materialize_working_memory(
                spec.mem,
                spec.session_id,
                spec.entity_id,
                spec.user_message,
                spec.capacity,
                pre_turn.policy_context,
            )
            .await,
        ),
        None => None,
    };
    pre_turn.working_memory = working_memory.as_ref();
    let enrichment = enrich_pre_turn(&pre_turn).await;
    emit_turn_evidence_trace(
        &pre_turn,
        &enrichment.contract,
        post_turn_seed.observer.as_ref(),
    );
    let outcome = execute_turn(
        enrichment.contract.rendered_prompt.clone(),
        enrichment.contract.temperature,
    )
    .await?;
    emit_turn_output_trace(&pre_turn, &outcome.result, post_turn_seed.observer.as_ref());
    let mut post_turn_seed = post_turn_seed;
    post_turn_seed.contract = enrichment.contract;
    run_post_turn_processing(
        post_turn_seed,
        &enrichment.affect,
        &outcome.result,
        working_memory,
    )
    .await?;
    Ok(outcome)
}

/// Execute a pre-assembled turn plan through a temporary [`TurnExecutor`].
///
/// # Errors
/// Returns an error if history loading, provider execution, tool execution,
/// response finalization, or transcript persistence fails.
pub async fn execute_turn_plan(plan: TurnExecutionPlan<'_>) -> Result<TurnExecutionOutcome> {
    execute_turn_plan_with_naturalness_context(plan, false, RelationshipDistance::Unknown).await
}

pub(crate) async fn execute_turn_plan_with_naturalness_context(
    plan: TurnExecutionPlan<'_>,
    naturalness_gate_enabled: bool,
    naturalness_relationship_distance: RelationshipDistance,
) -> Result<TurnExecutionOutcome> {
    TurnExecutor::new(plan.registry, plan.max_iterations, plan.loop_detection)
        .with_naturalness_gate(naturalness_gate_enabled)
        .with_naturalness_relationship_distance(naturalness_relationship_distance)
        .execute(TurnExecutionRequest {
            history: plan.history,
            run: TurnRunAdapter {
                provider: plan.provider,
                system_prompt: &plan.system_prompt,
                user_message: plan.user_message,
                response_finalization_enabled: plan.response_finalization_enabled,
                response_contract: plan.response_contract,
                image_content: plan.image_content,
                model: plan.model,
                temperature: plan.temperature,
                inference_options: plan.inference_options,
                ctx: plan.ctx,
                stream_sink: plan.stream_sink,
                state_notifier: plan.state_notifier,
            },
            transcript: plan.transcript,
        })
        .await
}

impl TurnExecutor {
    #[must_use]
    pub fn new(
        registry: Arc<ToolRegistry>,
        max_iterations: u32,
        loop_detection: LoopDetectionConfig,
    ) -> Self {
        Self {
            registry,
            max_iterations,
            loop_detection,
            naturalness_gate_enabled: false,
            naturalness_relationship_distance: RelationshipDistance::Unknown,
        }
    }

    #[must_use]
    pub fn with_naturalness_gate(mut self, enabled: bool) -> Self {
        self.naturalness_gate_enabled = enabled;
        self
    }

    #[must_use]
    pub(crate) fn with_naturalness_relationship_distance(
        mut self,
        distance: RelationshipDistance,
    ) -> Self {
        self.naturalness_relationship_distance = distance;
        self
    }

    /// # Errors
    /// Returns an error if the shared tool loop execution fails.
    pub async fn execute(&self, request: TurnExecutionRequest<'_>) -> Result<TurnExecutionOutcome> {
        let (session_id, conversation_history) = self.load_history(&request.history).await;
        let mut result = self.run_turn(&request.run, &conversation_history).await?;
        let output_mode = classify_response_mode(request.run.user_message);
        if request.run.response_finalization_enabled || self.naturalness_gate_enabled {
            let fin_request = ResponseFinalizationRequest::user_facing(
                &result.final_text,
                output_mode,
                result.streaming_delivered,
                request.run.response_contract,
                self.naturalness_gate_enabled,
            );
            let naturalness_context = NaturalnessFinalizationContext {
                conversation_history: &conversation_history,
                user_affect: naturalness_affect_from_text(request.run.user_message),
                relationship_distance: self.naturalness_relationship_distance,
            };
            let finalization = if matches!(output_mode, ResponseMode::Conversation) {
                finalize_response_contextual_with_context(
                    fin_request,
                    request.run.user_message,
                    naturalness_context,
                )
            } else {
                finalize_response_with_context(fin_request, naturalness_context)
            };
            if !finalization.applied_actions.is_empty() || !finalization.preserved {
                tracing::debug!(
                    target: "core::agent::turn_executor",
                    log_target = request.transcript.log_target,
                    surface = "shared_turn",
                    output_mode = ?output_mode,
                    before_score = finalization.before_score,
                    after_score = finalization.after_score,
                    preserved = finalization.preserved,
                    actions = ?finalization.applied_actions,
                    contract_mismatch_reason = finalization.contract_mismatch_reason.map(ContractMismatchReason::code),
                    micro_rewrite_reason_codes = ?finalization.micro_rewrite_reason_codes,
                    "response finalization evaluated"
                );
            }
            if let Some(reason) = finalization.contract_mismatch_reason {
                tracing::info!(
                    target: "core::agent::turn_executor::metrics",
                    verifier_kpi = "micro-rewrite",
                    mismatch_reason = reason.code(),
                    "verifier contract mismatch recorded"
                );
            } else if !finalization.micro_rewrite_reason_codes.is_empty() {
                tracing::info!(
                    target: "core::agent::turn_executor::metrics",
                    verifier_kpi = "pass-rate",
                    micro_rewrite_reason_codes = ?finalization.micro_rewrite_reason_codes,
                    "verifier pass with optional micro-rewrite reasons recorded"
                );
            }
            result.final_text = finalization.final_text;
        } else {
            tracing::debug!(
                target: "core::agent::turn_executor",
                log_target = request.transcript.log_target,
                surface = "shared_turn",
                output_mode = ?output_mode,
                "response finalization disabled"
            );
        }
        self.persist_turn(&request.transcript, session_id.as_ref(), &result)
            .await;
        Ok(TurnExecutionOutcome { session_id, result })
    }

    async fn load_history(
        &self,
        adapter: &TurnHistoryAdapter<'_>,
    ) -> (Option<SessionId>, Vec<ProviderMessage>) {
        let (session_id, history) = load_provider_history_async(
            adapter.session_manager,
            adapter.channel_name,
            adapter.session_key,
            adapter.tenant_id,
            adapter.max_tokens,
        )
        .await;

        if history.is_empty() && !adapter.fallback_history.is_empty() {
            (session_id, adapter.fallback_history.to_vec())
        } else {
            (session_id, history)
        }
    }

    async fn run_turn(
        &self,
        adapter: &TurnRunAdapter<'_>,
        conversation_history: &[ProviderMessage],
    ) -> Result<ToolLoopResult> {
        let checkpoint_dir = Some(
            adapter
                .ctx
                .workspace_dir
                .join(".asterel")
                .join("checkpoints"),
        );
        ToolLoop::new(Arc::clone(&self.registry), self.max_iterations)
            .with_loop_detection(self.loop_detection.clone())
            .run(ToolLoopRunParams {
                provider: adapter.provider,
                system_prompt: adapter.system_prompt,
                user_message: adapter.user_message,
                image_content: adapter.image_content,
                model: adapter.model,
                temperature: adapter.temperature,
                inference_options: adapter.inference_options,
                ctx: adapter.ctx,
                stream_sink: adapter.stream_sink.clone(),
                conversation_history,
                state_notifier: adapter.state_notifier.clone(),
                checkpoint_dir,
            })
            .await
    }

    async fn persist_turn(
        &self,
        adapter: &TurnTranscriptAdapter<'_>,
        session_id: Option<&SessionId>,
        result: &ToolLoopResult,
    ) {
        if let Err(error) = persist_tool_loop_turn_async(
            adapter.session_manager,
            session_id,
            adapter.user_message,
            result,
        )
        .await
        {
            tracing::warn!(
                log_target = adapter.log_target,
                session_id = ?session_id,
                error = %error,
                "failed to persist transcript turn"
            );
        }
    }
}
