//! Unit tests for the tool loop iteration logic.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use futures_util::stream;
use serde_json::{Value, json};

use super::*;
use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::core::providers::ProviderResult;
use crate::core::providers::response::{ProviderResponse, StopReason};
use crate::core::providers::streaming::{StreamEvent, StreamSink};
use crate::core::tools::middleware::{MiddlewareDecision, ToolMiddleware};
use crate::core::tools::traits::{AttachmentSource, OutputAttachment, Tool};
use crate::security::SecurityPolicy;

#[test]
fn checkpoint_filename_hashes_raw_session_id_without_sanitizer_collisions() {
    let from_slash = checkpoint_filename_for_session_id("a/b");
    let from_underscore = checkpoint_filename_for_session_id("a_b");

    assert_ne!(from_slash, from_underscore);
    assert!(!from_slash.contains('/'));
    assert!(!from_slash.contains('\\'));

    let stem = from_slash
        .strip_suffix(".json")
        .expect("checkpoint filename should keep json suffix");
    assert_eq!(stem.len(), 64);
    assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn checkpoint_state_keeps_traversal_like_session_ids_inside_checkpoint_dir() {
    let dir = tempfile::tempdir().unwrap();

    let state = CheckpointState::new(dir.path(), "../escape").unwrap();

    assert_eq!(state.path.parent(), Some(dir.path()));
    assert_ne!(state.path.file_name().unwrap(), "../escape.json");
}

#[test]
fn checkpoint_load_rejects_mismatched_embedded_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let state = CheckpointState::new(dir.path(), "current-session").unwrap();
    let mut foreign = TurnCheckpoint::new("other-session");
    foreign.mark_dispatched("call-1");
    foreign.save_to_file(&state.path).unwrap();

    assert!(state.load_prior().unwrap().is_none());
}

#[derive(Debug)]
struct EchoTool;

#[derive(Debug)]
struct AttachmentTool;

impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo_tool"
    }

    fn description(&self) -> &'static str {
        "Echo tool"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ToolResult {
                success: true,
                output: args.to_string(),
                error: None,

                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}

impl Tool for AttachmentTool {
    fn name(&self) -> &'static str {
        "attachment_tool"
    }

    fn description(&self) -> &'static str {
        "Attachment tool"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let index = args.get("index").and_then(Value::as_u64).unwrap_or(0);
            Ok(ToolResult {
                success: true,
                output: format!("attachment {index}"),
                error: None,
                attachments: vec![OutputAttachment::from_path(
                    "image/png",
                    format!("/tmp/generated-{index}.png"),
                    Some(format!("generated-{index}.png")),
                )],
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}

#[derive(Debug)]
struct CountingMiddleware {
    count: Arc<std::sync::atomic::AtomicUsize>,
}

impl ToolMiddleware for CountingMiddleware {
    fn before_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _args: &'a Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(MiddlewareDecision::Continue)
        })
    }

    fn after_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _result: &'a mut ToolResult,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {})
    }
}

#[derive(Debug)]
struct RateLimitMiddleware;

impl ToolMiddleware for RateLimitMiddleware {
    fn before_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _args: &'a Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            Ok(MiddlewareDecision::Block(format!(
                "{SECURITY_POLICY_BLOCK_PREFIX}entity action limit exceeded for 'test'"
            )))
        })
    }

    fn after_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _result: &'a mut ToolResult,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {})
    }
}

struct MockProvider {
    responses: Mutex<VecDeque<ProviderResponse>>,
}

impl Provider for MockProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok(String::new()) })
    }

    fn chat_with_tools<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _messages: &'a [ProviderMessage],
        _tools: &'a [ToolSpec],
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let mut guard = self
                .responses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(guard
                .pop_front()
                .unwrap_or_else(|| ProviderResponse::text_only("done".to_string())))
        })
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn capabilities(&self, _model: &str) -> crate::contracts::provider::ProviderCapabilities {
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: true,
            streaming: false,
            vision: false,
        }
    }
}

struct EffectiveToolsOnlyProvider;

impl Provider for EffectiveToolsOnlyProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok(String::new()) })
    }

    fn capabilities(&self, _model: &str) -> crate::contracts::provider::ProviderCapabilities {
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: true,
            streaming: false,
            vision: false,
        }
    }

    fn capability_profile(
        &self,
        _model: &str,
    ) -> crate::contracts::provider::ProviderCapabilityProfile {
        crate::contracts::provider::ProviderCapabilityProfile {
            native: crate::contracts::provider::ProviderCapabilities::default(),
            effective: self.capabilities(""),
        }
    }
}

struct MockStreamingProvider {
    events: Vec<StreamEvent>,
}

impl Provider for MockStreamingProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok(String::new()) })
    }

    fn chat_with_tools<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _messages: &'a [ProviderMessage],
        _tools: &'a [ToolSpec],
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move { Ok(ProviderResponse::text_only("fallback".to_string())) })
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _messages: &'a [ProviderMessage],
        _tools: &'a [ToolSpec],
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<
        Box<
            dyn Future<Output = ProviderResult<crate::core::providers::streaming::ProviderStream>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let items = self
                .events
                .iter()
                .cloned()
                .map(Ok::<_, anyhow::Error>)
                .collect::<Vec<_>>();
            Ok(Box::pin(stream::iter(items)) as crate::core::providers::streaming::ProviderStream)
        })
    }
}

struct NeverResolveProvider;

impl Provider for NeverResolveProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok(String::new()) })
    }

    fn chat_with_tools<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _messages: &'a [ProviderMessage],
        _tools: &'a [ToolSpec],
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}

#[derive(Default)]
struct RecordingSink {
    labels: Mutex<Vec<String>>,
    deltas: Mutex<Vec<String>>,
}

type ToolCallRecord = (String, String, String, Option<String>);

fn test_tool_spec() -> ToolSpec {
    ToolSpec::with_auto_effect(
        "echo_tool".to_string(),
        "Echo tool".to_string(),
        json!({"type": "object"}),
        Vec::new(),
    )
}

#[test]
fn tool_call_strategy_uses_native_not_effective_capability() {
    let provider = EffectiveToolsOnlyProvider;
    let tool_specs = vec![test_tool_spec()];

    let strategy = select_tool_call_strategy(&provider, "test-model", &tool_specs);

    assert_eq!(strategy, ToolCallStrategy::PromptFallback);
}

#[derive(Default)]
struct RecordingNotifier {
    states: Mutex<Vec<String>>,
    tool_calls: Mutex<Vec<ToolCallRecord>>,
}

impl AgentStateNotifier for RecordingNotifier {
    fn notify_state(&self, state: &str, _detail: Option<&str>) {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(state.to_string());
    }

    fn notify_tool_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        status: &str,
        detail: Option<&str>,
    ) {
        self.tool_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((
                tool_call_id.to_string(),
                tool_name.to_string(),
                status.to_string(),
                detail.map(str::to_string),
            ));
    }
}

impl StreamSink for RecordingSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let label = match event {
                StreamEvent::ResponseStart { .. } => "response_start",
                StreamEvent::TextDelta { .. } => "text_delta",
                StreamEvent::ToolCallDelta { .. } => "tool_call_delta",
                StreamEvent::ToolCallComplete { .. } => "tool_call_complete",
                StreamEvent::Done { .. } => "done",
            };
            self.labels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(label.to_string());
            if let StreamEvent::TextDelta { text } = event {
                self.deltas
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(text.clone());
            }
        })
    }
}

fn test_ctx() -> ExecutionContext {
    let security = Arc::new(SecurityPolicy::default());
    ExecutionContext::test_default(security)
}

#[tokio::test]
async fn loop_iterates_tool_use_then_end_turn() {
    let mut registry = ToolRegistry::new(vec![]);
    registry.register(Box::new(EchoTool));
    let registry = Arc::new(registry);
    let loop_ = ToolLoop::new(registry, 10);

    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            ProviderResponse {
                text: String::new(),
                input_tokens: Some(10),
                output_tokens: Some(5),
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "echo_tool".to_string(),
                    input: json!({"value": "ok"}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: "final answer".to_string(),
                input_tokens: Some(8),
                output_tokens: Some(4),
                model: None,
                content_blocks: vec![],
                stop_reason: Some(StopReason::EndTurn),
                logprobs: None,
            },
        ])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.final_text, "final answer");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tokens_used, Some(27));
}

#[tokio::test]
async fn loop_notifies_tool_call_status_changes() {
    let mut registry = ToolRegistry::new(vec![]);
    registry.register(Box::new(EchoTool));
    let registry = Arc::new(registry);
    let loop_ = ToolLoop::new(registry, 10);
    let notifier = Arc::new(RecordingNotifier::default());

    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_notify".to_string(),
                    name: "echo_tool".to_string(),
                    input: json!({"value": "ok"}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: "done".to_string(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![],
                stop_reason: Some(StopReason::EndTurn),
                logprobs: None,
            },
        ])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: Some(Arc::clone(&notifier) as Arc<dyn AgentStateNotifier>),
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(
        notifier
            .tool_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
        vec![
            (
                "toolu_notify".to_string(),
                "echo_tool".to_string(),
                "running".to_string(),
                None,
            ),
            (
                "toolu_notify".to_string(),
                "echo_tool".to_string(),
                "completed".to_string(),
                None,
            ),
        ]
    );
}

#[tokio::test]
async fn loop_detection_repeat_halts_with_error_when_critical_threshold_reached() {
    let mut registry = ToolRegistry::new(vec![]);
    registry.register(Box::new(EchoTool));
    let loop_ = ToolLoop::new(Arc::new(registry), 10).with_loop_detection(
        crate::config::LoopDetectionConfig {
            enabled: true,
            history_size: 8,
            warning_threshold: 1,
            critical_threshold: 2,
            repeat: true,
            ping_pong: false,
            no_progress: false,
        },
    );

    let repeat_tool_use = ProviderResponse {
        text: String::new(),
        input_tokens: None,
        output_tokens: None,
        model: None,
        content_blocks: vec![ContentBlock::ToolUse {
            id: "toolu_repeat".to_string(),
            name: "echo_tool".to_string(),
            input: json!({"value": "same"}),
        }],
        stop_reason: Some(StopReason::ToolUse),
        logprobs: None,
    };

    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            repeat_tool_use.clone(),
            repeat_tool_use.clone(),
            repeat_tool_use,
        ])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.iterations, 3);
    assert!(matches!(
        result.stop_reason,
        LoopStopReason::Error(ref error) if error.contains("loop_detection (repeat")
    ));
    assert_eq!(result.loop_detection_events.len(), 2);
    assert_eq!(
        result.loop_detection_events[0].kind,
        LoopDetectionKind::Repeat
    );
    assert_eq!(
        result.loop_detection_events[0].severity,
        LoopDetectionSeverity::Warning
    );
    assert_eq!(
        result.loop_detection_events[1].severity,
        LoopDetectionSeverity::Critical
    );
}

#[tokio::test]
async fn loop_stops_at_max_iterations_when_tool_use_continues() {
    let mut registry = ToolRegistry::new(vec![]);
    registry.register(Box::new(EchoTool));
    let loop_ = ToolLoop::new(Arc::new(registry), 2);
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "echo_tool".to_string(),
                    input: json!({}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_2".to_string(),
                    name: "echo_tool".to_string(),
                    input: json!({}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_3".to_string(),
                    name: "echo_tool".to_string(),
                    input: json!({}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
        ])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.stop_reason, LoopStopReason::MaxIterations);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls.len(), 2);
}

#[tokio::test]
async fn provider_timeout_stops_loop_with_error() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 1);
    let provider = NeverResolveProvider;

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should return timeout stop reason");

    match result.stop_reason {
        LoopStopReason::Error(reason) => {
            assert!(
                reason.contains("timed out"),
                "expected timeout error, got: {reason}"
            );
        }
        other => panic!("expected timeout error stop reason, got: {other:?}"),
    }
}

#[test]
fn hard_cap_is_enforced() {
    let registry = Arc::new(ToolRegistry::new(vec![]));
    let loop_ = ToolLoop::new(registry, 100);
    assert_eq!(loop_.max_iterations(), 25);
}

#[tokio::test]
async fn loop_executes_tools_through_registry_middleware_chain() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new(vec![Arc::new(CountingMiddleware {
        count: Arc::clone(&count),
    })]);
    registry.register(Box::new(EchoTool));
    let loop_ = ToolLoop::new(Arc::new(registry), 5);

    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "echo_tool".to_string(),
                    input: json!({"a": 1}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: "done".to_string(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![],
                stop_reason: Some(StopReason::EndTurn),
                logprobs: None,
            },
        ])),
    };

    let _ = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rate_limit_error_stops_loop() {
    let mut registry = ToolRegistry::new(vec![Arc::new(RateLimitMiddleware)]);
    registry.register(Box::new(EchoTool));
    let loop_ = ToolLoop::new(Arc::new(registry), 5);
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![ProviderResponse {
            text: String::new(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "echo_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: Some(StopReason::ToolUse),
            logprobs: None,
        }])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.stop_reason, LoopStopReason::RateLimited);
    assert_eq!(result.iterations, 1);
}

#[tokio::test]
async fn no_tools_registered_returns_single_turn_response() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![ProviderResponse {
            text: "plain response".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![],
            stop_reason: Some(StopReason::EndTurn),
            logprobs: None,
        }])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(result.iterations, 1);
    assert_eq!(result.final_text, "plain response");
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn streaming_with_none_sink_preserves_behavior() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockStreamingProvider {
        events: vec![
            StreamEvent::ResponseStart { model: None },
            StreamEvent::TextDelta {
                text: "hello".to_string(),
            },
            StreamEvent::TextDelta {
                text: " world".to_string(),
            },
            StreamEvent::Done {
                stop_reason: Some(StopReason::EndTurn),
                input_tokens: Some(3),
                output_tokens: Some(2),
            },
        ],
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(result.final_text, "hello world");
    assert_eq!(result.tokens_used, Some(5));
    assert!(!result.streaming_delivered);
    assert!(result.attachments.is_empty());
}

#[tokio::test]
async fn streaming_with_attached_sink_marks_final_text_delivered() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockStreamingProvider {
        events: vec![
            StreamEvent::TextDelta {
                text: "hello".to_string(),
            },
            StreamEvent::Done {
                stop_reason: Some(StopReason::EndTurn),
                input_tokens: None,
                output_tokens: None,
            },
        ],
    };
    let sink = Arc::new(RecordingSink::default());

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: Some(sink),
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.final_text, "hello");
    assert!(result.streaming_delivered);
}

#[tokio::test]
async fn loop_result_aggregates_attachments_across_tool_calls() {
    let mut registry = ToolRegistry::new(vec![]);
    registry.register(Box::new(AttachmentTool));
    let loop_ = ToolLoop::new(Arc::new(registry), 10);
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "attachment_tool".to_string(),
                    input: json!({"index": 1}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![ContentBlock::ToolUse {
                    id: "toolu_2".to_string(),
                    name: "attachment_tool".to_string(),
                    input: json!({"index": 2}),
                }],
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            },
            ProviderResponse {
                text: "done".to_string(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![],
                stop_reason: Some(StopReason::EndTurn),
                logprobs: None,
            },
        ])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert_eq!(result.attachments.len(), 2);
    assert!(matches!(
        &result.attachments[0].source,
        AttachmentSource::File { path } if path == "/tmp/generated-1.png"
    ));
    assert!(matches!(
        &result.attachments[1].source,
        AttachmentSource::File { path } if path == "/tmp/generated-2.png"
    ));
}

#[tokio::test]
async fn loop_result_attachments_empty_without_tool_uses() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![ProviderResponse {
            text: "plain response".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![],
            stop_reason: Some(StopReason::EndTurn),
            logprobs: None,
        }])),
    };

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    assert!(result.attachments.is_empty());
}

#[tokio::test]
async fn streaming_with_sink_receives_all_events_in_order() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockStreamingProvider {
        events: vec![
            StreamEvent::ResponseStart { model: None },
            StreamEvent::TextDelta {
                text: "a".to_string(),
            },
            StreamEvent::TextDelta {
                text: "b".to_string(),
            },
            StreamEvent::Done {
                stop_reason: Some(StopReason::EndTurn),
                input_tokens: None,
                output_tokens: None,
            },
        ],
    };
    let sink = Arc::new(RecordingSink::default());

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: Some(Arc::clone(&sink) as Arc<dyn StreamSink>),
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    let labels = sink
        .labels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let deltas = sink
        .deltas
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();

    assert_eq!(result.final_text, "ab");
    assert_eq!(
        labels,
        vec!["response_start", "text_delta", "text_delta", "done"]
    );
    assert_eq!(deltas, vec!["a", "b"]);
}

#[tokio::test]
async fn streaming_sink_receives_non_text_events_without_breaking_collection() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockStreamingProvider {
        events: vec![
            StreamEvent::ResponseStart { model: None },
            StreamEvent::ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                input_json_delta: "{\"command\":\"ls\"}".to_string(),
            },
            StreamEvent::Done {
                stop_reason: Some(StopReason::ToolUse),
                input_tokens: None,
                output_tokens: None,
            },
        ],
    };
    let sink = Arc::new(RecordingSink::default());

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: Some(Arc::clone(&sink) as Arc<dyn StreamSink>),
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    let labels = sink
        .labels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();

    assert_eq!(labels, vec!["response_start", "tool_call_delta", "done"]);
    assert!(result.final_text.is_empty());
    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn non_streaming_provider_emits_done_event_to_sink() {
    let loop_ = ToolLoop::new(Arc::new(ToolRegistry::new(vec![])), 5);
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![ProviderResponse {
            text: "plain response".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![],
            stop_reason: Some(StopReason::EndTurn),
            logprobs: None,
        }])),
    };
    let sink = Arc::new(RecordingSink::default());

    let result = loop_
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hello",
            image_content: &[],
            model: "test-model",
            temperature: 0.2,
            inference_options: None,
            ctx: &test_ctx(),
            stream_sink: Some(Arc::clone(&sink) as Arc<dyn StreamSink>),
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .unwrap();

    let labels = sink
        .labels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(labels, vec!["done"]);
    assert_eq!(result.final_text, "plain response");
}

#[test]
fn trust_boundary_present_when_tools_available() {
    let prompt = augment_prompt_with_trust_boundary("base prompt", true);
    assert!(prompt.contains("## Tool Result Trust Policy"));
    assert!(prompt.contains("[[external-content:tool_result:*]]"));
}

#[test]
fn trust_boundary_absent_when_no_tools_available() {
    let prompt = augment_prompt_with_trust_boundary("base prompt", false);
    assert_eq!(prompt, "base prompt");
}
