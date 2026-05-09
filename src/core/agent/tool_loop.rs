//! Provider-tool iteration loop.
//!
//! Drives the LLM inference / tool-call / tool-result cycle until
//! the model stops requesting tools, a hard cap is reached, or a
//! timeout fires.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::time::{Duration, timeout};

use super::checkpoint::{CompletedToolResult, TurnCheckpoint};

pub use super::tool_execution::augment_prompt_with_trust_boundary;
use super::tool_execution::{
    ToolLoopMetrics, build_result, classify_execute_error, format_tool_result_content,
    is_action_limit_message,
};
use super::tool_protocol::{
    ToolCallStrategy, parse_fallback_response, render_messages_for_fallback,
    render_tool_instructions,
};
pub use super::tool_types::{
    AgentStateNotifier, LoopDetectionEvent, LoopDetectionKind, LoopDetectionSeverity,
    LoopStopReason, ToolCallRecord, ToolLoop, ToolLoopResult, ToolLoopRunParams,
};
use super::tool_types::{ChatOnceInput, TOOL_LOOP_HARD_CAP, ToolUseExecutionOutcome};
use crate::core::providers::response::{
    ContentBlock, MessageRole, ProviderMessage, ProviderResponse, StopReason,
};
use crate::core::providers::streaming::{StreamCollector, StreamEvent};
use crate::core::providers::traits::Provider;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::registry::ToolRegistry;
use crate::core::tools::traits::{OutputAttachment, ToolResult, ToolSpec};

#[cfg(not(test))]
const PROVIDER_CALL_TIMEOUT_SECS: u64 = 120;
#[cfg(test)]
const PROVIDER_CALL_TIMEOUT_SECS: u64 = 2;

/// A compact fingerprint of one provider response used for loop detection.
///
/// `tool_signature` is `None` when the response contains no tool calls.
/// `text_signature` is the whitespace-normalised, lowercased display text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopFingerprint {
    tool_signature: Option<String>,
    text_signature: String,
}

/// Rolling accumulator for loop-detection hit counters across iterations.
#[derive(Default)]
struct LoopDetectionState {
    /// Sliding window of the last N fingerprints (bounded by `cfg.history_size`).
    history: VecDeque<LoopFingerprint>,
    /// Consecutive iterations where the response was identical to the previous.
    repeat_hits: u32,
    /// Consecutive iterations that exhibited an A/B/A/B alternating pattern.
    ping_pong_hits: u32,
    /// Consecutive iterations where the model called a tool but produced no text.
    no_progress_hits: u32,
}

impl LoopDetectionState {
    fn with_capacity(history_size: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(history_size),
            ..Default::default()
        }
    }
}

/// Result returned by `evaluate_loop_detection` when a threshold is crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopDetectionOutcome {
    /// Pattern crossed the warning threshold — log and continue.
    Warning { kind: LoopDetectionKind, count: u32 },
    /// Pattern crossed the critical threshold — halt the loop.
    Critical { kind: LoopDetectionKind, count: u32 },
}

/// Live checkpoint state threaded through tool execution.
///
/// Only constructed when the caller provides a `checkpoint_dir`.
struct CheckpointState {
    checkpoint: TurnCheckpoint,
    path: PathBuf,
}

/// Build a collision-resistant, single-component checkpoint filename.
///
/// Session IDs are transport-derived strings and are not guaranteed to be
/// filename-safe. Hash the exact raw ID instead of replacing characters so
/// distinct IDs such as `a/b` and `a_b` cannot collapse to the same file.
fn checkpoint_filename_for_session_id(session_id: &str) -> String {
    let digest = Sha256::digest(session_id.as_bytes());
    format!("{}.json", hex::encode(digest))
}

impl CheckpointState {
    /// Build from a checkpoint directory and session id.
    ///
    /// Creates the directory tree if it does not yet exist.
    fn new(checkpoint_dir: &std::path::Path, session_id: &str) -> anyhow::Result<Self> {
        std::fs::create_dir_all(checkpoint_dir)?;
        let path = checkpoint_dir.join(checkpoint_filename_for_session_id(session_id));
        Ok(Self {
            checkpoint: TurnCheckpoint::new(session_id),
            path,
        })
    }

    /// Load a prior checkpoint only if its embedded session matches this turn.
    fn load_prior(&self) -> anyhow::Result<Option<TurnCheckpoint>> {
        let Some(prior) = TurnCheckpoint::load_from_file(&self.path)? else {
            return Ok(None);
        };
        if prior.session_id != self.checkpoint.session_id {
            tracing::warn!(
                checkpoint_session_id = %prior.session_id,
                current_session_id = %self.checkpoint.session_id,
                path = %self.path.display(),
                "crash recovery: ignoring checkpoint for different session"
            );
            return Ok(None);
        }
        Ok(Some(prior))
    }

    /// Persist the checkpoint to disk, logging (but not propagating) errors.
    fn save(&self) {
        if let Err(e) = self.checkpoint.save_to_file(&self.path) {
            tracing::warn!(
                path = %self.path.display(),
                error = %e,
                "failed to persist turn checkpoint"
            );
        }
    }

    /// Clear the checkpoint file after a successful turn.
    fn clear(self) {
        TurnCheckpoint::clear_file(&self.path);
    }
}

/// Collapse consecutive whitespace and lowercase text for stable fingerprint comparison.
fn normalize_text_signature(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for segment in text.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        for c in segment.chars() {
            normalized.push(c.to_ascii_lowercase());
        }
    }
    normalized
}

/// Build a `LoopFingerprint` for `response`.
///
/// All `ToolUse` blocks are serialised as `name:input` pairs joined by `|`.
/// The text portion is normalised via [`normalize_text_signature`].
fn fingerprint_from_response(response: &ProviderResponse) -> LoopFingerprint {
    let mut tool_signature = String::new();
    for block in response.iter_tool_use_blocks() {
        if let ContentBlock::ToolUse { name, input, .. } = block {
            if !tool_signature.is_empty() {
                tool_signature.push('|');
            }
            let _ = write!(tool_signature, "{name}:{input}");
        }
    }

    LoopFingerprint {
        tool_signature: (!tool_signature.is_empty()).then_some(tool_signature),
        text_signature: normalize_text_signature(&response.text),
    }
}

fn evaluate_loop_detection(
    state: &mut LoopDetectionState,
    response: &ProviderResponse,
    cfg: &crate::config::LoopDetectionConfig,
) -> Option<LoopDetectionOutcome> {
    if !cfg.enabled {
        return None;
    }

    let current = fingerprint_from_response(response);
    let last = state.history.back();

    if cfg.repeat {
        if last.is_some_and(|previous| previous == &current) {
            state.repeat_hits = state.repeat_hits.saturating_add(1);
        } else {
            state.repeat_hits = 0;
        }
    } else {
        state.repeat_hits = 0;
    }

    if cfg.no_progress {
        let current_is_no_progress =
            current.tool_signature.is_some() && current.text_signature.is_empty();
        if current_is_no_progress
            && last.is_some_and(|previous| {
                previous.tool_signature == current.tool_signature
                    && previous.text_signature.is_empty()
            })
        {
            state.no_progress_hits = state.no_progress_hits.saturating_add(1);
        } else {
            state.no_progress_hits = 0;
        }
    } else {
        state.no_progress_hits = 0;
    }

    if cfg.ping_pong {
        if state.history.len() >= 3 {
            let len = state.history.len();
            let a = &state.history[len - 3];
            let b = &state.history[len - 2];
            let c = &state.history[len - 1];
            if a == c && b == &current && a != b {
                state.ping_pong_hits = state.ping_pong_hits.saturating_add(1);
            } else {
                state.ping_pong_hits = 0;
            }
        } else {
            state.ping_pong_hits = 0;
        }
    } else {
        state.ping_pong_hits = 0;
    }

    state.history.push_back(current);
    while state.history.len() > cfg.history_size {
        let _ = state.history.pop_front();
    }

    let patterns = [
        (LoopDetectionKind::Repeat, state.repeat_hits, cfg.repeat),
        (
            LoopDetectionKind::PingPong,
            state.ping_pong_hits,
            cfg.ping_pong,
        ),
        (
            LoopDetectionKind::NoProgress,
            state.no_progress_hits,
            cfg.no_progress,
        ),
    ];

    if let Some((kind, count, _)) = patterns
        .iter()
        .copied()
        .filter(|(_, _, enabled)| *enabled)
        .max_by_key(|(_, count, _)| *count)
    {
        if count >= cfg.critical_threshold {
            return Some(LoopDetectionOutcome::Critical { kind, count });
        }
        if count >= cfg.warning_threshold {
            return Some(LoopDetectionOutcome::Warning { kind, count });
        }
    }
    None
}

fn loop_detection_event(
    kind: LoopDetectionKind,
    severity: LoopDetectionSeverity,
    count: u32,
    threshold: u32,
    iteration: u32,
) -> LoopDetectionEvent {
    LoopDetectionEvent {
        kind,
        severity,
        count,
        threshold,
        iteration,
    }
}

/// Accumulated state across tool loop iterations.
struct LoopState {
    messages: Vec<ProviderMessage>,
    tool_calls: Vec<ToolCallRecord>,
    attachments: Vec<OutputAttachment>,
    loop_detection_events: Vec<LoopDetectionEvent>,
    iterations: u32,
    token_sum: u64,
    saw_tokens: bool,
    last_logprobs: Option<Vec<crate::core::providers::response::TokenLogprob>>,
    streaming_delivered: bool,
}

impl LoopState {
    fn new(messages: Vec<ProviderMessage>) -> Self {
        Self {
            messages,
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            loop_detection_events: Vec::new(),
            iterations: 0,
            token_sum: 0,
            saw_tokens: false,
            last_logprobs: None,
            streaming_delivered: false,
        }
    }

    fn into_result(self, stop_reason: LoopStopReason) -> ToolLoopResult {
        build_result(
            &self.messages,
            self.tool_calls,
            self.attachments,
            stop_reason,
            ToolLoopMetrics {
                loop_detection_events: self.loop_detection_events,
                iterations: self.iterations,
                token_sum: self.token_sum,
                saw_tokens: self.saw_tokens,
                logprobs: self.last_logprobs,
                streaming_delivered: self.streaming_delivered,
            },
        )
    }

    fn record_tokens(&mut self, response: &ProviderResponse) {
        if let Some(tokens) = response.total_tokens() {
            self.token_sum = self.token_sum.saturating_add(tokens);
            self.saw_tokens = true;
        }
        if response.logprobs.is_some() {
            self.last_logprobs.clone_from(&response.logprobs);
        }
    }
}

fn build_initial_messages(
    conversation_history: &[ProviderMessage],
    user_message: &str,
    image_content: &[ContentBlock],
) -> Vec<ProviderMessage> {
    let initial_message = if image_content.is_empty() {
        ProviderMessage::user(user_message)
    } else {
        let mut content = vec![ContentBlock::Text {
            text: user_message.to_string(),
        }];
        content.extend(image_content.iter().cloned());
        ProviderMessage {
            role: MessageRole::User,
            content,
        }
    };
    let mut messages = Vec::with_capacity(conversation_history.len() + 1);
    messages.extend_from_slice(conversation_history);
    messages.push(initial_message);
    messages
}

fn build_turn_context(ctx: &ExecutionContext, turn_number: u32) -> ExecutionContext {
    let mut turn_ctx = ctx.clone();
    turn_ctx.turn_number = turn_number;
    turn_ctx
}

fn handle_loop_detection_outcome(
    state: &mut LoopState,
    cfg: &crate::config::LoopDetectionConfig,
    outcome: LoopDetectionOutcome,
) -> Option<LoopStopReason> {
    match outcome {
        LoopDetectionOutcome::Warning { kind, count } => {
            let event = loop_detection_event(
                kind,
                LoopDetectionSeverity::Warning,
                count,
                cfg.warning_threshold,
                state.iterations,
            );
            tracing::warn!(
                target: "tools.loop_detection",
                pattern = kind.as_str(),
                severity = event.severity.as_str(),
                count = event.count,
                threshold = event.threshold,
                iteration = event.iteration,
                "tool loop pattern detected"
            );
            state.loop_detection_events.push(event);
            None
        }
        LoopDetectionOutcome::Critical { kind, count } => {
            let event = loop_detection_event(
                kind,
                LoopDetectionSeverity::Critical,
                count,
                cfg.critical_threshold,
                state.iterations,
            );
            tracing::warn!(
                target: "tools.loop_detection",
                pattern = kind.as_str(),
                severity = event.severity.as_str(),
                count = event.count,
                threshold = event.threshold,
                iteration = event.iteration,
                "tool loop halted by loop detection"
            );
            state.loop_detection_events.push(event);
            Some(LoopStopReason::Error(format!(
                "tool loop halted by loop_detection ({}, count={count}, critical_threshold={})",
                kind.as_str(),
                cfg.critical_threshold
            )))
        }
    }
}

/// Outcome of a single provider call attempt within the tool loop.
enum ProviderCallOutcome {
    /// The provider returned a well-formed response.
    Success(ProviderResponse),
    /// The call failed or timed out; the loop should stop with this reason.
    Stop(LoopStopReason),
}

/// What the tool loop should do after processing one batch of tool uses.
enum ToolUseDisposition {
    /// At least one tool ran; go back to the top of the loop for the next iteration.
    Continue,
    /// A fatal error occurred during tool execution; stop the loop.
    Stop(LoopStopReason),
    /// The response had no tool uses; treat as a final answer and exit the loop.
    Complete,
}

/// Unified input for a single provider call, regardless of which calling strategy is active.
struct StrategyCallInput<'a> {
    prompt: &'a str,
    messages: &'a [ProviderMessage],
    tool_specs: &'a [ToolSpec],
    model: &'a str,
    temperature: f64,
    inference_options: Option<&'a crate::core::providers::InferenceOpts>,
    stream_sink: Option<&'a dyn crate::core::providers::streaming::StreamSink>,
}

async fn call_provider_with_timeout(
    tool_loop: &ToolLoop,
    provider: &dyn Provider,
    input: ChatOnceInput<'_>,
) -> ProviderCallOutcome {
    match timeout(
        Duration::from_secs(PROVIDER_CALL_TIMEOUT_SECS),
        tool_loop.chat_once(provider, input),
    )
    .await
    {
        Ok(Ok(response)) => ProviderCallOutcome::Success(response),
        Ok(Err(error)) => ProviderCallOutcome::Stop(LoopStopReason::Error(error.to_string())),
        Err(_) => ProviderCallOutcome::Stop(LoopStopReason::Error(format!(
            "provider call timed out after {PROVIDER_CALL_TIMEOUT_SECS}s"
        ))),
    }
}

/// Issue a plain text provider call for the `PromptFallback` strategy.
///
/// Flattens the structured `messages` list into a single text string via
/// [`render_messages_for_fallback`], then calls `chat_with_system_full_opts`.
/// Tool instructions are prepended to the system prompt by the caller.
async fn call_provider_text_only(
    _tool_loop: &ToolLoop,
    provider: &dyn Provider,
    system_prompt: &str,
    messages: &[ProviderMessage],
    model: &str,
    temperature: f64,
    inference_options: Option<&crate::core::providers::InferenceOpts>,
) -> crate::core::providers::ProviderResult<ProviderResponse> {
    let text = render_messages_for_fallback(messages);
    provider
        .chat_with_system_full_opts(
            Some(system_prompt),
            &text,
            model,
            temperature,
            inference_options,
        )
        .await
}

async fn call_provider_text_only_with_timeout(
    tool_loop: &ToolLoop,
    provider: &dyn Provider,
    system_prompt: &str,
    messages: &[ProviderMessage],
    model: &str,
    temperature: f64,
    inference_options: Option<&crate::core::providers::InferenceOpts>,
) -> ProviderCallOutcome {
    match timeout(
        Duration::from_secs(PROVIDER_CALL_TIMEOUT_SECS),
        call_provider_text_only(
            tool_loop,
            provider,
            system_prompt,
            messages,
            model,
            temperature,
            inference_options,
        ),
    )
    .await
    {
        Ok(Ok(response)) => ProviderCallOutcome::Success(response),
        Ok(Err(error)) => ProviderCallOutcome::Stop(LoopStopReason::Error(error.to_string())),
        Err(_) => ProviderCallOutcome::Stop(LoopStopReason::Error(format!(
            "provider call timed out after {PROVIDER_CALL_TIMEOUT_SECS}s"
        ))),
    }
}

async fn call_provider_for_strategy(
    tool_loop: &ToolLoop,
    provider: &dyn Provider,
    strategy: ToolCallStrategy,
    input: StrategyCallInput<'_>,
) -> ProviderCallOutcome {
    match strategy {
        ToolCallStrategy::Native => {
            call_provider_with_timeout(
                tool_loop,
                provider,
                ChatOnceInput {
                    system_prompt: Some(input.prompt),
                    messages: input.messages,
                    tool_specs: input.tool_specs,
                    model: input.model,
                    temperature: input.temperature,
                    inference_options: input.inference_options.copied(),
                    stream_sink: input.stream_sink,
                },
            )
            .await
        }
        ToolCallStrategy::PromptFallback => {
            let augmented = format!(
                "{}\n{}",
                input.prompt,
                render_tool_instructions(input.tool_specs)
            );
            match call_provider_text_only_with_timeout(
                tool_loop,
                provider,
                &augmented,
                input.messages,
                input.model,
                input.temperature,
                input.inference_options,
            )
            .await
            {
                ProviderCallOutcome::Success(raw_response) => {
                    ProviderCallOutcome::Success(build_fallback_provider_response(raw_response))
                }
                ProviderCallOutcome::Stop(reason) => ProviderCallOutcome::Stop(reason),
            }
        }
    }
}

/// Convert a raw text response from a `PromptFallback` call into the
/// standard `ProviderResponse` shape with parsed `ToolUse` content blocks.
///
/// If the parsed result contains tool calls, the stop reason is overridden
/// to `StopReason::ToolUse` so the loop's tool-dispatch logic fires normally.
fn build_fallback_provider_response(raw_response: ProviderResponse) -> ProviderResponse {
    let parsed = parse_fallback_response(&raw_response.text);
    let mut content_blocks = Vec::new();
    if !parsed.display_text.is_empty() {
        content_blocks.push(ContentBlock::Text {
            text: parsed.display_text.clone(),
        });
    }
    for call in &parsed.tool_calls {
        content_blocks.push(ContentBlock::ToolUse {
            id: call.id.clone(),
            name: call.name.clone(),
            input: call.input.clone(),
        });
    }
    let stop_reason = if parsed.tool_calls.is_empty() {
        raw_response.stop_reason
    } else {
        Some(StopReason::ToolUse)
    };

    ProviderResponse {
        text: parsed.display_text,
        content_blocks,
        stop_reason,
        input_tokens: raw_response.input_tokens,
        output_tokens: raw_response.output_tokens,
        model: raw_response.model,
        logprobs: raw_response.logprobs,
    }
}

fn stop_from_provider_outcome(
    state_notifier: Option<&Arc<dyn AgentStateNotifier>>,
    state: LoopState,
    reason: LoopStopReason,
) -> ToolLoopResult {
    notify_state(state_notifier, "error", Some(&format!("{reason:?}")));
    state.into_result(reason)
}

fn log_tool_specs(tool_specs: &[ToolSpec]) {
    if !tracing::enabled!(tracing::Level::INFO) {
        return;
    }
    let mut tool_names = String::new();
    for t in tool_specs {
        if !tool_names.is_empty() {
            tool_names.push_str(", ");
        }
        tool_names.push_str(&t.name);
    }
    tracing::info!(
        tool_count = tool_specs.len(),
        tool_names = %tool_names,
        "tool loop: registered tools for this turn"
    );
}

fn notify_state(notifier: Option<&Arc<dyn AgentStateNotifier>>, state: &str, detail: Option<&str>) {
    if let Some(notifier) = notifier {
        notifier.notify_state(state, detail);
    }
}

fn log_provider_response(response: &ProviderResponse) {
    if !tracing::enabled!(tracing::Level::INFO) {
        return;
    }
    let text_preview: String = response.text.chars().take(200).collect();
    tracing::info!(
        stop_reason = ?response.stop_reason,
        has_tool_use = response.has_tool_use(),
        text_len = response.text.len(),
        text_preview = %text_preview,
        "tool loop: provider response"
    );
}

/// Choose whether to use native function calling or the text-based fallback.
///
/// Falls back to `PromptFallback` only when the provider lacks native tool
/// support and at least one tool is available. When no tools are registered
/// we always use `Native` regardless (the provider may still route the call
/// through its own function-calling path if it wishes).
fn select_tool_call_strategy(
    provider: &dyn Provider,
    model: &str,
    tool_specs: &[ToolSpec],
) -> ToolCallStrategy {
    if provider
        .capability_profile(model)
        .native
        .native_tool_calling
        && !tool_specs.is_empty()
    {
        ToolCallStrategy::Native
    } else if !tool_specs.is_empty() {
        ToolCallStrategy::PromptFallback
    } else {
        ToolCallStrategy::Native
    }
}

/// Prepare the augmented system prompt, initial message list, and loop detection
/// state to begin a new tool loop run.
///
/// The system prompt is augmented with the trust-boundary policy block when tools
/// are active (see [`augment_prompt_with_trust_boundary`]).
fn build_loop_prompt_and_state(
    system_prompt: &str,
    tool_specs: &[ToolSpec],
    conversation_history: &[ProviderMessage],
    user_message: &str,
    image_content: &[ContentBlock],
    history_size: usize,
) -> (String, LoopState, LoopDetectionState) {
    (
        augment_prompt_with_trust_boundary(system_prompt, !tool_specs.is_empty()),
        LoopState::new(build_initial_messages(
            conversation_history,
            user_message,
            image_content,
        )),
        LoopDetectionState::with_capacity(history_size),
    )
}

fn stop_from_tool_execution(
    state_notifier: Option<&Arc<dyn AgentStateNotifier>>,
    state: LoopState,
    reason: LoopStopReason,
) -> ToolLoopResult {
    notify_state(state_notifier, "idle", Some(&format!("{reason:?}")));
    state.into_result(reason)
}

fn max_iteration_result(
    state_notifier: Option<&Arc<dyn AgentStateNotifier>>,
    state: LoopState,
) -> ToolLoopResult {
    notify_state(state_notifier, "idle", Some("max_iterations"));
    state.into_result(LoopStopReason::MaxIterations)
}

fn init_checkpoint(
    checkpoint_dir: Option<&PathBuf>,
    session_id: Option<&String>,
) -> Option<CheckpointState> {
    let (Some(dir), Some(sid)) = (checkpoint_dir, session_id) else {
        return None;
    };
    match CheckpointState::new(dir, sid) {
        Ok(cs) => Some(cs),
        Err(e) => {
            tracing::warn!(
                checkpoint_dir = %dir.display(),
                error = %e,
                "unable to initialise turn checkpoint; continuing without"
            );
            None
        }
    }
}

/// Build synthetic messages that replay a prior crashed turn (phase-H).
///
/// Reconstructs the assistant tool-use blocks and their corresponding results
/// so that the model can see what was completed and what was interrupted.
/// Completed results are replayed verbatim; interrupted calls get an error
/// marker so the model knows the tool did not finish.
fn replay_checkpoint_messages(prior: &super::checkpoint::TurnCheckpoint) -> Vec<ProviderMessage> {
    if prior.completed_tool_results.is_empty() && prior.pending_tool_call_ids.is_empty() {
        return vec![];
    }

    // Reconstruct the assistant message that triggered the tool calls.
    let mut assistant_content: Vec<ContentBlock> = Vec::with_capacity(
        1 + prior.completed_tool_results.len() + prior.pending_tool_call_ids.len(),
    );
    if !prior.assistant_message.is_empty() {
        assistant_content.push(ContentBlock::Text {
            text: prior.assistant_message.clone(),
        });
    }
    for completed in &prior.completed_tool_results {
        assistant_content.push(ContentBlock::ToolUse {
            id: completed.tool_call_id.clone(),
            name: completed.tool_name.clone(),
            input: serde_json::Value::Null,
        });
    }
    for pending_id in prior.interrupted_tools() {
        assistant_content.push(ContentBlock::ToolUse {
            id: pending_id.clone(),
            name: "interrupted_tool".to_string(),
            input: serde_json::Value::Null,
        });
    }

    let mut messages = vec![ProviderMessage {
        role: MessageRole::Assistant,
        content: assistant_content,
    }];

    for completed in &prior.completed_tool_results {
        messages.push(ProviderMessage::tool_result(
            &completed.tool_call_id,
            &completed.output,
            !completed.success,
        ));
    }
    for pending_id in prior.interrupted_tools() {
        messages.push(ProviderMessage::tool_result(
            pending_id,
            "tool execution was interrupted by a process crash; result is unknown",
            true,
        ));
    }

    messages
}

/// Return `true` when the provider response asks for tool execution.
///
/// Checks both the explicit `ToolUse` stop reason and the presence of at
/// least one `ToolUse` content block, because some providers emit one
/// without the other.
fn should_execute_tool_uses(response: &ProviderResponse) -> bool {
    matches!(response.stop_reason, Some(StopReason::ToolUse)) || response.has_tool_use()
}

impl ToolLoop {
    #[must_use]
    pub fn new(registry: Arc<ToolRegistry>, max_iterations: u32) -> Self {
        Self {
            registry,
            max_iterations: max_iterations.min(TOOL_LOOP_HARD_CAP),
            loop_detection: crate::config::LoopDetectionConfig::default(),
        }
    }

    #[must_use]
    pub fn with_loop_detection(
        mut self,
        loop_detection: crate::config::LoopDetectionConfig,
    ) -> Self {
        self.loop_detection = loop_detection;
        self
    }

    /// # Errors
    ///
    /// Returns an error when a provider call cannot be constructed or awaited.
    #[allow(clippy::too_many_lines)]
    pub async fn run(&self, params: ToolLoopRunParams<'_>) -> anyhow::Result<ToolLoopResult> {
        let ToolLoopRunParams {
            provider,
            system_prompt,
            user_message,
            image_content,
            model,
            temperature,
            inference_options,
            ctx,
            stream_sink,
            conversation_history,
            state_notifier,
            checkpoint_dir,
        } = params;
        let tool_specs: Vec<ToolSpec> = self.registry.specs_for_context(ctx);
        log_tool_specs(&tool_specs);
        let strategy = select_tool_call_strategy(provider, model, &tool_specs);
        let (prompt, mut state, mut loop_detection_state) = build_loop_prompt_and_state(
            system_prompt,
            &tool_specs,
            conversation_history,
            user_message,
            image_content,
            self.loop_detection.history_size,
        );

        let mut ckpt_state = init_checkpoint(checkpoint_dir.as_ref(), ctx.session_id.as_ref());

        // Phase-H: crash recovery — if a checkpoint file exists from a previous
        // crashed turn, inject its completed results and error markers for
        // interrupted tools so the model can continue without redoing work.
        if let Some(ref cs) = ckpt_state {
            match cs.load_prior() {
                Ok(Some(prior)) => {
                    tracing::warn!(
                        session_id = %prior.session_id,
                        completed = prior.completed_tool_results.len(),
                        interrupted = prior.pending_tool_call_ids.len(),
                        "crash recovery: replaying prior turn checkpoint"
                    );
                    state.messages.extend(replay_checkpoint_messages(&prior));
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "crash recovery: failed to load checkpoint; starting fresh");
                }
            }
        }

        loop {
            state.iterations = state.iterations.saturating_add(1);
            if state.iterations > self.max_iterations {
                state.iterations = state.iterations.saturating_sub(1);
                return Ok(max_iteration_result(state_notifier.as_ref(), state));
            }

            notify_state(state_notifier.as_ref(), "thinking", None);
            let response = match call_provider_for_strategy(
                self,
                provider,
                strategy,
                StrategyCallInput {
                    prompt: prompt.as_str(),
                    messages: &state.messages,
                    tool_specs: &tool_specs,
                    model,
                    temperature,
                    inference_options: inference_options.as_ref(),
                    stream_sink: stream_sink.as_deref(),
                },
            )
            .await
            {
                ProviderCallOutcome::Success(response) => response,
                ProviderCallOutcome::Stop(reason) => {
                    return Ok(stop_from_provider_outcome(
                        state_notifier.as_ref(),
                        state,
                        reason,
                    ));
                }
            };
            let response_streaming_delivered = matches!(strategy, ToolCallStrategy::Native)
                && stream_sink.is_some()
                && provider.supports_streaming();

            log_provider_response(&response);
            state.record_tokens(&response);
            if let Some(outcome) =
                evaluate_loop_detection(&mut loop_detection_state, &response, &self.loop_detection)
                && let Some(stop_reason) =
                    handle_loop_detection_outcome(&mut state, &self.loop_detection, outcome)
            {
                notify_state(state_notifier.as_ref(), "idle", Some("loop_detected"));
                return Ok(state.into_result(stop_reason));
            }
            if should_execute_tool_uses(&response) {
                match self
                    .handle_tool_use_response(
                        response,
                        ctx,
                        state.iterations,
                        &mut state,
                        state_notifier.as_ref(),
                        &mut ckpt_state,
                        provider,
                        model,
                        user_message,
                    )
                    .await
                {
                    ToolUseDisposition::Continue => continue,
                    ToolUseDisposition::Stop(reason) => {
                        return Ok(stop_from_tool_execution(
                            state_notifier.as_ref(),
                            state,
                            reason,
                        ));
                    }
                    ToolUseDisposition::Complete => {}
                }
            } else {
                state.messages.push(response.into_assistant_message());
            }
            state.streaming_delivered = response_streaming_delivered;

            // Turn completed successfully — clear the checkpoint file.
            if let Some(cs) = ckpt_state.take() {
                cs.clear();
            }
            notify_state(state_notifier.as_ref(), "idle", Some("completed"));
            return Ok(state.into_result(LoopStopReason::Completed));
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_tool_use_response(
        &self,
        response: ProviderResponse,
        ctx: &ExecutionContext,
        iteration: u32,
        state: &mut LoopState,
        state_notifier: Option<&Arc<dyn AgentStateNotifier>>,
        ckpt_state: &mut Option<CheckpointState>,
        provider: &dyn Provider,
        model: &str,
        user_message: &str,
    ) -> ToolUseDisposition {
        let turn_ctx = build_turn_context(ctx, iteration);
        notify_state(state_notifier, "tool_use", None);
        let ToolUseExecutionOutcome {
            had_tool_use,
            stop_reason,
            tool_result_messages,
            tool_calls,
            attachments,
        } = self
            .execute_tool_uses(&response, &turn_ctx, iteration, state_notifier, ckpt_state)
            .await;

        // Self-filter oversized tool results before injecting into context.
        let filtered_messages = self
            .filter_tool_results(tool_result_messages, provider, model, user_message)
            .await;

        state.messages.push(response.into_assistant_message());
        state.messages.extend(filtered_messages);
        state.tool_calls.extend(tool_calls);
        state.attachments.extend(attachments);

        if let Some(reason) = stop_reason {
            ToolUseDisposition::Stop(reason)
        } else if had_tool_use {
            ToolUseDisposition::Continue
        } else {
            ToolUseDisposition::Complete
        }
    }

    async fn chat_once(
        &self,
        provider: &dyn Provider,
        input: ChatOnceInput<'_>,
    ) -> anyhow::Result<ProviderResponse> {
        if provider.supports_streaming() {
            let mut stream = provider
                .chat_with_tools_stream_opts(
                    input.system_prompt,
                    input.messages,
                    input.tool_specs,
                    input.model,
                    input.temperature,
                    input.inference_options.as_ref(),
                )
                .await?;
            let mut collector = StreamCollector::new();
            while let Some(event_result) = stream.next().await {
                let event = event_result?;
                if let Some(sink) = input.stream_sink {
                    sink.on_event(&event).await;
                }
                collector.feed(&event);
            }
            Ok(collector.finish())
        } else {
            let response = provider
                .chat_with_tools_opts(
                    input.system_prompt,
                    input.messages,
                    input.tool_specs,
                    input.model,
                    input.temperature,
                    input.inference_options.as_ref(),
                )
                .await
                .map_err(anyhow::Error::from)?;
            if let Some(sink) = input.stream_sink {
                let done_event = StreamEvent::Done {
                    stop_reason: response.stop_reason,
                    input_tokens: response.input_tokens,
                    output_tokens: response.output_tokens,
                };
                sink.on_event(&done_event).await;
            }
            Ok(response)
        }
    }

    async fn execute_tool_uses(
        &self,
        response: &ProviderResponse,
        ctx: &ExecutionContext,
        iteration: u32,
        state_notifier: Option<&Arc<dyn AgentStateNotifier>>,
        ckpt_state: &mut Option<CheckpointState>,
    ) -> ToolUseExecutionOutcome {
        let mut had_tool_use = false;
        let mut tool_result_messages = Vec::new();
        let mut tool_calls = Vec::new();
        let mut attachments = Vec::new();

        for block in response.iter_tool_use_blocks() {
            if let ContentBlock::ToolUse { id, name, input } = block {
                had_tool_use = true;
                if let Some(notifier) = state_notifier {
                    notifier.notify_tool_call(id, name, "running", None);
                }

                // Checkpoint: mark tool as dispatched before execution.
                if let Some(cs) = ckpt_state.as_mut() {
                    cs.checkpoint.mark_dispatched(id);
                    cs.save();
                }

                let tool_result = match self.registry.execute(name, input.clone(), ctx).await {
                    Ok(result) => result,
                    Err(error) => {
                        let error_message = error.to_string();
                        if let Some(stop_reason) = classify_execute_error(&error_message) {
                            if let Some(notifier) = state_notifier {
                                notifier.notify_tool_call(id, name, "failed", Some(&error_message));
                            }
                            return ToolUseExecutionOutcome {
                                had_tool_use,
                                stop_reason: Some(stop_reason),
                                tool_result_messages,
                                tool_calls,
                                attachments,
                            };
                        }
                        ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some(error_message),
                            attachments: Vec::new(),
                            taint_labels: Vec::new(),
                            semantic:
                                crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                        }
                    }
                };

                // Checkpoint: mark tool as completed after execution.
                if let Some(cs) = ckpt_state.as_mut() {
                    cs.checkpoint.mark_completed(CompletedToolResult {
                        tool_call_id: id.clone(),
                        tool_name: name.clone(),
                        output: tool_result.output.clone(),
                        success: tool_result.success,
                    });
                    cs.save();
                }

                if let Some(notifier) = state_notifier {
                    let status = if tool_result.success {
                        "completed"
                    } else {
                        "failed"
                    };
                    notifier.notify_tool_call(id, name, status, tool_result.error.as_deref());
                }

                if tool_result
                    .error
                    .as_deref()
                    .is_some_and(is_action_limit_message)
                {
                    return ToolUseExecutionOutcome {
                        had_tool_use,
                        stop_reason: Some(LoopStopReason::RateLimited),
                        tool_result_messages,
                        tool_calls,
                        attachments,
                    };
                }

                let tool_result_content = format_tool_result_content(&tool_result);
                let is_error = !tool_result.success;
                attachments.extend(tool_result.attachments.iter().cloned());
                tool_calls.push(ToolCallRecord {
                    tool_name: name.clone(),
                    args: input.clone(),
                    result: tool_result,
                    iteration,
                });
                tool_result_messages.push(ProviderMessage::tool_result(
                    id.clone(),
                    tool_result_content,
                    is_error,
                ));
            }
        }

        ToolUseExecutionOutcome {
            had_tool_use,
            stop_reason: None,
            tool_result_messages,
            tool_calls,
            attachments,
        }
    }

    /// Apply self-filtering to oversized tool results before context injection.
    ///
    /// Iterates through tool result messages. For each `ToolResult` content
    /// block that exceeds the filter threshold, makes a lightweight LLM call
    /// to extract only task-relevant information.
    async fn filter_tool_results(
        &self,
        messages: Vec<ProviderMessage>,
        provider: &dyn Provider,
        model: &str,
        task_context: &str,
    ) -> Vec<ProviderMessage> {
        let mut result = Vec::with_capacity(messages.len());
        for msg in messages {
            let mut new_content = Vec::with_capacity(msg.content.len());
            for block in msg.content {
                match block {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } if !is_error
                        && content.len() > super::result_filter::FILTER_THRESHOLD_CHARS =>
                    {
                        let (filtered, outcome) = super::result_filter::filter_tool_result(
                            provider,
                            model,
                            &tool_use_id,
                            task_context,
                            &content,
                        )
                        .await;
                        match &outcome {
                            super::result_filter::FilterOutcome::Filtered {
                                original_chars,
                                filtered_chars,
                            } => {
                                let orig = *original_chars;
                                let filt = *filtered_chars;
                                let saved_pct =
                                    orig.saturating_sub(filt).saturating_mul(100) / orig.max(1);
                                tracing::debug!(
                                    tool_use_id = %tool_use_id,
                                    original_chars = orig,
                                    filtered_chars = filt,
                                    saved_pct,
                                    "tool result self-filtered"
                                );
                            }
                            super::result_filter::FilterOutcome::Failed(err) => {
                                tracing::warn!(
                                    tool_use_id = %tool_use_id,
                                    error = %err,
                                    "tool result self-filtering failed; using original"
                                );
                            }
                            super::result_filter::FilterOutcome::BelowThreshold => {}
                        }
                        new_content.push(ContentBlock::ToolResult {
                            tool_use_id,
                            content: filtered,
                            is_error,
                        });
                    }
                    other => new_content.push(other),
                }
            }
            result.push(ProviderMessage {
                role: msg.role,
                content: new_content,
            });
        }
        result
    }

    #[cfg(test)]
    fn max_iterations(&self) -> u32 {
        self.max_iterations
    }
}

#[cfg(test)]
mod tests;
