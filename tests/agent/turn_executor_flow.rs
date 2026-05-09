use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use asterel::config::LoopDetectionConfig;
use asterel::core::agent::{
    AgentStateNotifier, TurnExecutionRequest, TurnExecutor, TurnHistoryAdapter, TurnRunAdapter,
    TurnTranscriptAdapter,
};
use asterel::core::providers::response::{ProviderMessage, ProviderResponse, StopReason};
use asterel::core::providers::streaming::{StreamEvent, StreamSink};
use asterel::core::providers::{Provider, ProviderResult, ThinkingLevel};
use asterel::core::sessions::SessionOrchestrator;
use asterel::core::sessions::types::SessionConfig;
use asterel::core::tools::ToolRegistry;
use asterel::core::tools::middleware::{ExecutionContext, default_middleware_chain};
use asterel::security::SecurityPolicy;
use tempfile::{NamedTempFile, TempDir};

struct RecordingNotifier {
    states: Mutex<Vec<String>>,
}

impl RecordingNotifier {
    fn has_events(&self) -> bool {
        !self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
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

impl RecordingSink {
    fn saw_done(&self) -> bool {
        self.labels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|label| label == "done")
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
}

impl MockProvider {
    fn with_text_responses(values: &[&str]) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(
                values
                    .iter()
                    .map(|value| ProviderResponse {
                        text: (*value).to_string(),
                        input_tokens: Some(1),
                        output_tokens: Some(1),
                        model: None,
                        content_blocks: vec![],
                        stop_reason: Some(StopReason::EndTurn),
                        logprobs: None,
                    })
                    .collect::<Vec<_>>(),
            )),
            seen_messages: Mutex::new(Vec::new()),
        }
    }

    fn seen_messages(&self) -> Vec<Vec<ProviderMessage>> {
        self.seen_messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
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
        _tools: &'a [asterel::core::tools::ToolSpec],
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
                .unwrap_or_else(|| ProviderResponse::text_only("done".to_string())))
        })
    }

    fn supports_tools(&self) -> bool {
        true
    }
}

async fn session_manager() -> (TempDir, NamedTempFile, SessionOrchestrator) {
    let database_url = crate::test_env::postgres_url()
        .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let workspace_dir = temp_dir.path().join("workspace");
    crate::test_env::write_workspace_postgres_config(&workspace_dir, &database_url)
        .expect("test config should be written");
    let db_file = NamedTempFile::new_in(&workspace_dir).expect("session db file should be created");
    let manager = SessionOrchestrator::connect(db_file.path(), SessionConfig::default())
        .await
        .expect("session manager should be created");
    (temp_dir, db_file, manager)
}

fn executor() -> TurnExecutor {
    TurnExecutor::new(
        Arc::new(ToolRegistry::new(default_middleware_chain())),
        4,
        LoopDetectionConfig::default(),
    )
}

fn ctx() -> ExecutionContext {
    ExecutionContext::from_security(Arc::new(SecurityPolicy::default()))
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn shared_turn_executor_supports_gateway_and_channel_adapters() {
    let (_temp_dir, _db_file, manager) = session_manager().await;

    let executor = executor();
    let ctx = ctx();
    let gateway_provider = MockProvider::with_text_responses(&["gateway first", "gateway second"]);
    let channel_provider = MockProvider::with_text_responses(&["channel first", "channel second"]);
    let notifier = Arc::new(RecordingNotifier {
        states: Mutex::new(Vec::new()),
    });
    let sink = Arc::new(RecordingSink::default());

    let gateway_first = executor
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
                provider: &gateway_provider,
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
        .expect("gateway first turn");

    let gateway_second = executor
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
                provider: &gateway_provider,
                system_prompt: "system",
                user_message: "follow up gateway",
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
                user_message: "follow up gateway",
                log_target: "tests::gateway",
            },
        })
        .await
        .expect("gateway second turn");

    let channel_first = executor
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
                provider: &channel_provider,
                system_prompt: "system",
                user_message: "hello channel",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: Some(
                    asterel::core::providers::InferenceOpts::from_thinking_level(
                        ThinkingLevel::Low,
                    ),
                ),
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
        .expect("channel first turn");

    let channel_second = executor
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
                provider: &channel_provider,
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
        .expect("channel second turn");

    assert_eq!(gateway_first.session_id, gateway_second.session_id);
    assert_eq!(channel_first.session_id, channel_second.session_id);
    assert_eq!(gateway_second.result.final_text, "gateway second");
    assert_eq!(channel_second.result.final_text, "channel second");
    assert!(gateway_provider.seen_messages()[1].len() >= 2);
    assert!(channel_provider.seen_messages()[1].len() >= 2);
    assert!(notifier.has_events());
    assert!(sink.saw_done());
}
