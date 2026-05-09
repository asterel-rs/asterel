//! Shared tool-loop types used by the agent runtime.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::LoopDetectionConfig;
use crate::core::providers::InferenceOpts;
use crate::core::providers::response::{ContentBlock, ProviderMessage};
use crate::core::providers::streaming::StreamSink;
use crate::core::providers::traits::Provider;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::registry::ToolRegistry;
use crate::core::tools::traits::{OutputAttachment, ToolResult, ToolSpec};

/// Safety cap for tool-loop iterations regardless of configured limits.
pub(crate) const TOOL_LOOP_HARD_CAP: u32 = 25;

/// Runtime handle for executing tools through the registry.
pub struct ToolLoop {
    /// Tool registry containing executable tool implementations.
    pub(crate) registry: Arc<ToolRegistry>,
    /// Maximum iterations allowed for this loop instance.
    pub(crate) max_iterations: u32,
    /// Loop-pattern detection controls.
    pub(crate) loop_detection: LoopDetectionConfig,
}

/// Notifier for agent state transitions during tool loop execution.
///
/// Implementations bridge the core agent loop to external state
/// consumers (e.g. gateway WebSocket broadcasts) without introducing
/// direct dependencies on transport types.
pub trait AgentStateNotifier: Send + Sync {
    /// Called when the agent transitions to a new execution phase.
    fn notify_state(&self, state: &str, detail: Option<&str>);

    fn notify_tool_call(
        &self,
        _tool_call_id: &str,
        _tool_name: &str,
        _status: &str,
        _detail: Option<&str>,
    ) {
    }
}

/// Inputs required to run one tool loop cycle.
pub struct ToolLoopRunParams<'a> {
    /// Provider used for assistant inference calls.
    pub provider: &'a dyn Provider,
    /// System prompt used for this run.
    pub system_prompt: &'a str,
    /// User message for the current turn.
    pub user_message: &'a str,
    /// Optional image content attached to the user turn.
    pub image_content: &'a [ContentBlock],
    /// Model identifier to execute against.
    pub model: &'a str,
    /// Sampling temperature for inference calls.
    pub temperature: f64,
    /// Optional provider-specific inference options.
    pub inference_options: Option<InferenceOpts>,
    /// Tool execution/security context.
    pub ctx: &'a ExecutionContext,
    /// Optional streaming sink for incremental output.
    pub stream_sink: Option<Arc<dyn StreamSink>>,
    /// Prior conversation history to include in context.
    pub conversation_history: &'a [ProviderMessage],
    /// Optional notifier for broadcasting agent state transitions.
    pub state_notifier: Option<Arc<dyn AgentStateNotifier>>,
    /// Optional directory for turn checkpoint files.
    ///
    /// When set, the tool loop persists a [`TurnCheckpoint`] before each
    /// tool dispatch and clears it after the turn completes. The checkpoint
    /// file is written to a hash-derived filename under `checkpoint_dir`.
    pub checkpoint_dir: Option<PathBuf>,
}

/// Audit record for a single tool invocation in a loop run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Executed tool name.
    pub tool_name: String,
    /// Tool arguments sent to the tool.
    pub args: serde_json::Value,
    /// Execution result returned by the tool.
    pub result: ToolResult,
    /// Loop iteration in which the tool was invoked.
    pub iteration: u32,
}

/// Terminal reason for ending a tool loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStopReason {
    /// Completed normally with a final assistant response.
    Completed,
    /// Hit the configured maximum iteration count.
    MaxIterations,
    /// Ended due to a runtime/provider error.
    Error(String),
    /// Approval flow denied the requested tool action.
    ApprovalDenied,
    /// Ended due to rate-limit enforcement.
    RateLimited,
}

/// Loop-pattern category detected while the tool loop is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDetectionKind {
    /// Consecutive identical responses.
    Repeat,
    /// Alternating A/B/A/B response pattern.
    PingPong,
    /// Repeated tool use without textual progress.
    NoProgress,
}

impl LoopDetectionKind {
    /// Stable label for logs and transcript metadata.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repeat => "repeat",
            Self::PingPong => "ping_pong",
            Self::NoProgress => "no_progress",
        }
    }
}

/// Severity for loop-detection telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDetectionSeverity {
    /// Pattern crossed the warning threshold.
    Warning,
    /// Pattern crossed the critical threshold and halted the loop.
    Critical,
}

impl LoopDetectionSeverity {
    /// Stable label for logs and transcript metadata.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// Structured telemetry for a loop-detection event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopDetectionEvent {
    /// Detected loop-pattern category.
    pub kind: LoopDetectionKind,
    /// Severity of the event.
    pub severity: LoopDetectionSeverity,
    /// Consecutive hit count observed.
    pub count: u32,
    /// Threshold crossed to emit this event.
    pub threshold: u32,
    /// Iteration where the event was observed.
    pub iteration: u32,
}

/// Final output from a tool loop run.
pub struct ToolLoopResult {
    /// Final assistant text output.
    pub final_text: String,
    /// Ordered list of executed tool calls.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Files/links emitted by tools during the run.
    pub attachments: Vec<OutputAttachment>,
    /// Structured loop-detection events observed during execution.
    pub loop_detection_events: Vec<LoopDetectionEvent>,
    /// Iteration count consumed.
    pub iterations: u32,
    /// Total tokens used when available from provider.
    pub tokens_used: Option<u64>,
    /// Reason the loop terminated.
    pub stop_reason: LoopStopReason,
    /// Token-level log-probabilities from the final response, when
    /// available from the provider.
    pub logprobs: Option<Vec<crate::core::providers::response::TokenLogprob>>,
    /// True when final assistant text was already emitted through a stream sink.
    pub streaming_delivered: bool,
}

/// All parameters needed for a single `chat_once` provider call.
pub(crate) struct ChatOnceInput<'a> {
    /// Optional system prompt, passed as-is to the provider.
    pub(crate) system_prompt: Option<&'a str>,
    /// Conversation history plus the current user turn.
    pub(crate) messages: &'a [ProviderMessage],
    /// Tool definitions visible to the model for this call.
    pub(crate) tool_specs: &'a [ToolSpec],
    /// Model identifier string (e.g. `"claude-3-5-sonnet-20241022"`).
    pub(crate) model: &'a str,
    /// Sampling temperature for this inference call.
    pub(crate) temperature: f64,
    /// Provider-specific inference overrides (thinking budget, etc.).
    pub(crate) inference_options: Option<InferenceOpts>,
    /// Optional streaming sink; `None` means non-streaming.
    pub(crate) stream_sink: Option<&'a dyn StreamSink>,
}

/// The combined result of executing all tool-use blocks from one provider response.
pub(crate) struct ToolUseExecutionOutcome {
    /// `true` if at least one `ToolUse` block was processed.
    pub(crate) had_tool_use: bool,
    /// Non-`None` when a terminal error occurred mid-execution (e.g. rate limit
    /// or approval denied); remaining tool calls are abandoned.
    pub(crate) stop_reason: Option<LoopStopReason>,
    /// `ToolResult` messages ready to be appended to the conversation history.
    pub(crate) tool_result_messages: Vec<ProviderMessage>,
    /// Audit records for each tool call executed in this batch.
    pub(crate) tool_calls: Vec<ToolCallRecord>,
    /// File/link attachments emitted by tools during this batch.
    pub(crate) attachments: Vec<OutputAttachment>,
}
