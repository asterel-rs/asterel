use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;

use tempfile::{NamedTempFile, TempDir};

use super::turn_executor_metrics::{emit_turn_evidence_trace, emit_turn_output_trace};
use super::*;
use crate::contracts::provider::ProviderCapabilities;
use crate::core::providers::response::{ProviderResponse, StopReason};
use crate::core::providers::streaming::NullStreamSink;
use crate::core::providers::streaming::StreamEvent;
use crate::core::sessions::types::SessionConfig;
use crate::core::tools::middleware::default_middleware_chain;
use crate::security::SecurityPolicy;

#[derive(Default)]
struct RecordingNotifier {
    states: Mutex<Vec<String>>,
}

impl AgentStateNotifier for RecordingNotifier {
    fn notify_state(&self, state: &str, _detail: Option<&str>) {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(state.to_string());
    }
}

#[derive(Default)]
struct RecordingSink {
    labels: Mutex<Vec<String>>,
}

#[derive(Default)]
struct RecordingObserver {
    events: Mutex<Vec<ObserverEvent>>,
}

impl Observer for RecordingObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.clone());
    }

    fn record_metric(&self, _metric: &crate::contracts::observability::ObserverMetric) {}

    fn name(&self) -> &'static str {
        "recording"
    }
}

impl StreamSink for RecordingSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
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
        })
    }
}

struct MockProvider {
    responses: Mutex<VecDeque<ProviderResponse>>,
    seen_messages: Mutex<Vec<Vec<ProviderMessage>>>,
    streaming: bool,
}

impl MockProvider {
    fn text_response(text: &str) -> ProviderResponse {
        ProviderResponse {
            text: text.to_string(),
            input_tokens: Some(2),
            output_tokens: Some(3),
            model: None,
            content_blocks: vec![],
            stop_reason: Some(StopReason::EndTurn),
            logprobs: None,
        }
    }

    fn seen_messages(&self) -> Vec<Vec<ProviderMessage>> {
        self.seen_messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn message_contains_text(message: &ProviderMessage, expected: &str) -> bool {
    message.content.iter().any(|block| match block {
        ContentBlock::Text { text } => text.contains(expected),
        ContentBlock::ToolUse { .. }
        | ContentBlock::ToolResult { .. }
        | ContentBlock::Image { .. } => false,
    })
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
        messages: &'a [ProviderMessage],
        _tools: &'a [crate::core::tools::traits::ToolSpec],
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.seen_messages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(messages.to_vec());
            let mut guard = self
                .responses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(guard
                .pop_front()
                .unwrap_or_else(|| MockProvider::text_response("done")))
        })
    }

    fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            streaming: self.streaming,
            vision: false,
        }
    }
}

async fn session_manager() -> (
    TempDir,
    NamedTempFile,
    SessionOrchestrator,
    crate::utils::test_env::TestDbGuard,
) {
    let db_guard = crate::utils::test_env::acquire_test_db().await;
    let database_url = crate::utils::test_env::postgres_url()
        .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let workspace_dir = temp_dir.path().join("workspace");
    crate::utils::test_env::write_workspace_postgres_config(&workspace_dir, &database_url)
        .expect("test config should be written");
    let db_file = NamedTempFile::new_in(&workspace_dir).expect("session db file should exist");
    // Use the async connect path to avoid constructing a nested runtime
    // inside the test's own async context.
    let manager = SessionOrchestrator::connect(db_file.path(), SessionConfig::default())
        .await
        .expect("session manager should be created");
    (temp_dir, db_file, manager, db_guard)
}

fn executor() -> TurnExecutor {
    TurnExecutor::new(
        Arc::new(ToolRegistry::new(default_middleware_chain())),
        4,
        LoopDetectionConfig::default(),
    )
}

fn test_ctx() -> ExecutionContext {
    ExecutionContext::test_default(Arc::new(SecurityPolicy::default()))
}

#[test]
fn turn_evidence_trace_emission_accepts_compiled_contract() {
    let temp = TempDir::new().expect("tempdir should be created");
    let mem = crate::core::memory::MarkdownMemory::new(temp.path());
    let policy_context = crate::security::policy::TenantPolicyContext::disabled();
    let input = PreTurnInput {
        mem: &mem,
        workspace_dir: temp.path(),
        base_prompt: "base",
        user_message: "hello",
        entity_id: "person:test",
        person_id: "test",
        base_temperature: 0.4,
        policy_context: &policy_context,
        recall_min_confidence: None,
        persona_config: None,
        session_manager: None,
        session_surface: None,
        is_direct_address: true,
        session_owner_scope: None,
        session_id: Some("session-test"),
        policy_section: "",
        exposure_plan: None,
        working_memory: None,
    };
    let contract =
        crate::core::agent::turn_enrichment::compile_turn_contract("base", "", None, "", 0.4);
    let observer = RecordingObserver::default();

    emit_turn_evidence_trace(&input, &contract, &observer);

    let events = observer
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(events.len(), 10);
    assert!(events.iter().any(|event| matches!(
        event,
        ObserverEvent::CompanionPolicyRail { phase, .. } if phase == "tool_action"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ObserverEvent::CompanionTurnEvidence { phase, decision, .. }
            if phase == "context" && decision == "defer"
    )));
}

#[test]
fn output_trace_records_runtime_output_decision() {
    let temp = TempDir::new().expect("tempdir should be created");
    let mem = crate::core::memory::MarkdownMemory::new(temp.path());
    let policy_context = crate::security::policy::TenantPolicyContext::disabled();
    let input = PreTurnInput {
        mem: &mem,
        workspace_dir: temp.path(),
        base_prompt: "base",
        user_message: "hello",
        entity_id: "person:test",
        person_id: "test",
        base_temperature: 0.4,
        policy_context: &policy_context,
        recall_min_confidence: None,
        persona_config: None,
        session_manager: None,
        session_surface: None,
        is_direct_address: true,
        session_owner_scope: None,
        session_id: Some("session-test"),
        policy_section: "",
        exposure_plan: None,
        working_memory: None,
    };
    let observer = RecordingObserver::default();
    let result = ToolLoopResult {
        final_text: "hello back".to_string(),
        tool_calls: Vec::new(),
        attachments: Vec::new(),
        loop_detection_events: Vec::new(),
        iterations: 1,
        tokens_used: None,
        stop_reason: LoopStopReason::Completed,
        logprobs: None,
        streaming_delivered: false,
    };

    emit_turn_output_trace(&input, &result, &observer);

    let events = observer
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(events.iter().any(|event| matches!(
        event,
        ObserverEvent::CompanionTurnEvidence {
            phase,
            decision,
            reason_code,
            provenance,
            ..
        } if phase == "output"
            && decision == "allow"
            && reason_code == "turn_output_available"
            && provenance == "turn_executor"
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn gateway_adapter_reuses_history_and_persists_turns() {
    let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
    let executor = executor();
    let notifier = Arc::new(RecordingNotifier::default());
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            MockProvider::text_response("gateway first"),
            MockProvider::text_response("gateway second"),
        ])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: false,
    };
    let ctx = test_ctx();

    let first = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("gateway-session"),
                tenant_id: Some("tenant-a"),
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "hello gateway",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: Some(Arc::clone(&notifier) as Arc<dyn AgentStateNotifier>),
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "hello gateway",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("gateway first turn should succeed");

    let second = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("gateway-session"),
                tenant_id: Some("tenant-a"),
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "follow up gateway",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: Some(Arc::new(NullStreamSink) as Arc<dyn StreamSink>),
                state_notifier: Some(Arc::clone(&notifier) as Arc<dyn AgentStateNotifier>),
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "follow up gateway",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("gateway second turn should succeed");

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.result.final_text, "gateway first");
    assert_eq!(second.result.final_text, "gateway second");
    assert!(provider.seen_messages()[1].len() >= 2);
    assert!(
        !notifier
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}

#[tokio::test]
async fn turn_executor_uses_fallback_history_without_session_manager() {
    let executor = executor();
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![MockProvider::text_response("ok")])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: false,
    };
    let ctx = test_ctx();
    let fallback_history = vec![ProviderMessage::user("prior context")];

    let outcome = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: None,
                channel_name: "main_session",
                session_key: None,
                tenant_id: None,
                max_tokens: 0,
                fallback_history: &fallback_history,
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "hello",
                response_finalization_enabled: false,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: None,
                user_message: "hello",
                log_target: "tests::fallback_history",
            },
        })
        .await
        .expect("turn should succeed");

    assert_eq!(outcome.result.final_text, "ok");
    let seen = provider.seen_messages();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0]
            .iter()
            .any(|message| message_contains_text(message, "prior context"))
    );
}

#[tokio::test]
async fn turn_executor_threads_history_to_naturalness_context() {
    let executor = executor().with_naturalness_gate(true);
    let raw_text = "了解しました\n- **重要**: ここを見る";
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![MockProvider::text_response(raw_text)])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: false,
    };
    let ctx = test_ctx();
    let fallback_history = vec![ProviderMessage::assistant(
        "了解しました。前回の返答では短く受けました。",
    )];

    let outcome = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: None,
                channel_name: "main_session",
                session_key: None,
                tenant_id: None,
                max_tokens: 0,
                fallback_history: &fallback_history,
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "雑談しよう",
                response_finalization_enabled: false,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: None,
                user_message: "雑談しよう",
                log_target: "tests::naturalness_history",
            },
        })
        .await
        .expect("turn should succeed");

    assert_eq!(outcome.result.final_text, raw_text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn channel_adapter_reuses_history_and_stream_sink() {
    let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
    let executor = executor();
    let sink = Arc::new(RecordingSink::default());
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            MockProvider::text_response("channel first"),
            MockProvider::text_response("channel second"),
        ])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: false,
    };
    let ctx = test_ctx();

    let first = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "discord",
                session_key: Some("conversation::discord::channel-7"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "hello channel",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: Some(InferenceOpts::from_thinking_level(
                    crate::core::providers::ThinkingLevel::Low,
                )),
                ctx: &ctx,
                stream_sink: Some(Arc::clone(&sink) as Arc<dyn StreamSink>),
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "hello channel",
                log_target: "tests::channel",
            },
        })
        .await
        .expect("channel first turn should succeed");

    let second = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "discord",
                session_key: Some("conversation::discord::channel-7"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "follow up channel",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: Some(Arc::clone(&sink) as Arc<dyn StreamSink>),
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "follow up channel",
                log_target: "tests::channel",
            },
        })
        .await
        .expect("channel second turn should succeed");

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.result.final_text, "channel first");
    assert_eq!(second.result.final_text, "channel second");
    assert!(provider.seen_messages()[1].len() >= 2);
    assert!(
        sink.labels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|label| label == "done")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn shared_turn_finalizes_text_before_history_reuse() {
    let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
    let executor = executor();
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            MockProvider::text_response("いい質問です。原因は接続順です。"),
            MockProvider::text_response("history loaded"),
        ])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: false,
    };
    let ctx = test_ctx();

    let first = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("finalization-session"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "説明して",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "説明して",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("first turn should succeed");

    let second = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("finalization-session"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "続けて",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "続けて",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("second turn should succeed");

    assert_eq!(first.result.final_text, "原因は接続順です。");
    assert_eq!(second.result.final_text, "history loaded");
    let seen_messages = provider.seen_messages();
    assert!(
        seen_messages[1]
            .iter()
            .any(|message| message_contains_text(message, "原因は接続順です。"))
    );
    assert!(
        seen_messages[1]
            .iter()
            .all(|message| !message_contains_text(message, "いい質問です。"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn shared_turn_streaming_keeps_original_text() {
    let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
    let executor = executor();
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![MockProvider::text_response(
            "いい質問です。原因は接続順です。",
        )])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: true,
    };
    let ctx = test_ctx();

    let outcome = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "discord",
                session_key: Some("streaming-finalization"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "説明して",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: Some(Arc::new(NullStreamSink) as Arc<dyn StreamSink>),
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "説明して",
                log_target: "tests::channel",
            },
        })
        .await
        .expect("streaming turn should succeed");

    assert_eq!(
        outcome.result.final_text,
        "いい質問です。原因は接続順です。"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn shared_turn_can_disable_response_finalization() {
    let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
    let executor = executor();
    let provider = MockProvider {
        responses: Mutex::new(VecDeque::from(vec![
            MockProvider::text_response("いい質問です。原因は接続順です。"),
            MockProvider::text_response("history loaded"),
        ])),
        seen_messages: Mutex::new(Vec::new()),
        streaming: false,
    };
    let ctx = test_ctx();

    let first = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("finalization-disabled"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "説明して",
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: None,
                response_finalization_enabled: false,
                response_contract: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "説明して",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("first turn should succeed");

    let second = executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("finalization-disabled"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "続けて",
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                state_notifier: None,
                response_finalization_enabled: false,
                response_contract: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "続けて",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("second turn should succeed");

    assert_eq!(first.result.final_text, "いい質問です。原因は接続順です。");
    assert_eq!(second.result.final_text, "history loaded");
    let seen_messages = provider.seen_messages();
    assert!(
        seen_messages[1]
            .iter()
            .any(|message| message_contains_text(message, "いい質問です。原因は接続順です。"))
    );
}
