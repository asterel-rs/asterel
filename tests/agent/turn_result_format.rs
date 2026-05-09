use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use asterel::config::LoopDetectionConfig;
use asterel::contracts::ids::EntityId;
use asterel::contracts::provider::ProviderCapabilities;
use asterel::core::agent::{
    LoopStopReason, TurnExecutionRequest, TurnExecutor, TurnHistoryAdapter, TurnRunAdapter,
    TurnTranscriptAdapter,
};
use asterel::core::providers::response::{
    ContentBlock, MessageRole, ProviderMessage, ProviderResponse, StopReason,
};
use asterel::core::providers::{Provider, ProviderResult};
use asterel::core::sessions::SessionOrchestrator;
use asterel::core::sessions::types::SessionConfig;
use asterel::core::tools::ToolRegistry;
use asterel::core::tools::middleware::{ExecutionContext, default_middleware_chain};
use asterel::core::tools::traits::{Tool, ToolResult};
use asterel::security::{AutonomyLevel, EntityRateLimiter, SecurityPolicy};
use serde_json::json;
use tempfile::{NamedTempFile, TempDir};

#[derive(Debug)]
struct MockProvider {
    responses: Mutex<VecDeque<ProviderResponse>>,
    seen_messages: Mutex<Vec<Vec<ProviderMessage>>>,
}

impl MockProvider {
    fn new(responses: Vec<ProviderResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            seen_messages: Mutex::new(Vec::new()),
        }
    }

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

            let mut responses = self
                .responses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(responses
                .pop_front()
                .unwrap_or_else(|| MockProvider::text_response("done")))
        })
    }

    fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            streaming: false,
            vision: false,
        }
    }
}

#[derive(Debug)]
struct RecordingTool {
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl RecordingTool {
    fn new(calls: Arc<Mutex<Vec<serde_json::Value>>>) -> Self {
        Self { calls }
    }
}

impl Tool for RecordingTool {
    fn name(&self) -> &str {
        "record_note"
    }

    fn description(&self) -> &str {
        "Records the provided payload for test assertions"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "note": {"type": "string"}
            },
            "required": ["note"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(args.clone());
            Ok(ToolResult {
                success: true,
                output: format!("recorded: {args}"),
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: asterel::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
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

fn executor(registry: Arc<ToolRegistry>) -> TurnExecutor {
    TurnExecutor::new(registry, 4, LoopDetectionConfig::default())
}

fn empty_registry() -> Arc<ToolRegistry> {
    Arc::new(ToolRegistry::new(default_middleware_chain()))
}

fn registry_with_recording_tool(calls: Arc<Mutex<Vec<serde_json::Value>>>) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new(default_middleware_chain());
    registry.register(Box::new(RecordingTool::new(calls)));
    Arc::new(registry)
}

fn ctx(temp_dir: &TempDir) -> ExecutionContext {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: temp_dir.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let mut ctx = ExecutionContext::from_security(security);
    ctx.autonomy_level = AutonomyLevel::Full;
    ctx.entity_id = EntityId::from("agent:test");
    ctx.rate_limiter = Arc::new(EntityRateLimiter::new(1_000, 1_000));
    ctx
}

fn message_contains_text(message: &ProviderMessage, expected: &str) -> bool {
    message.content.iter().any(|block| match block {
        ContentBlock::Text { text } => text.contains(expected),
        ContentBlock::ToolUse { .. }
        | ContentBlock::ToolResult { .. }
        | ContentBlock::Image { .. } => false,
    })
}

fn assistant_message_has_tool_use(message: &ProviderMessage, tool_name: &str) -> bool {
    message.role == MessageRole::Assistant
        && message.content.iter().any(|block| match block {
            ContentBlock::ToolUse { name, .. } => name == tool_name,
            ContentBlock::Text { .. }
            | ContentBlock::ToolResult { .. }
            | ContentBlock::Image { .. } => false,
        })
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn simple_text_response_has_nonempty_final_text() {
    let (_temp_dir, _db_file, manager) = session_manager().await;
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let execution_context = ctx(&temp_dir);
    let provider = MockProvider::new(vec![MockProvider::text_response("mock says hello")]);

    let outcome = executor(empty_registry())
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("simple-text"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "hello",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "hello",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("turn should succeed");

    assert!(!outcome.result.final_text.is_empty());
    assert_eq!(outcome.result.final_text, "mock says hello");
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn turn_creates_session_when_none_exists() {
    let (_temp_dir, _db_file, manager) = session_manager().await;
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let execution_context = ctx(&temp_dir);
    let provider = MockProvider::new(vec![MockProvider::text_response("created")]);

    let outcome = executor(empty_registry())
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("new-session"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "start",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "start",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("turn should succeed");

    assert!(outcome.session_id.is_some());
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn stop_reason_is_completed_for_normal_response() {
    let (_temp_dir, _db_file, manager) = session_manager().await;
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let execution_context = ctx(&temp_dir);
    let provider = MockProvider::new(vec![MockProvider::text_response("normal")]);

    let outcome = executor(empty_registry())
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("stop-reason"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "hello",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "hello",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("turn should succeed");

    assert_eq!(outcome.result.stop_reason, LoopStopReason::Completed);
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn tool_call_response_executes_tool_and_continues() {
    let (_temp_dir, _db_file, manager) = session_manager().await;
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let execution_context = ctx(&temp_dir);
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider::new(vec![
        ProviderResponse {
            text: String::new(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "record_note".to_string(),
                input: json!({"note": "remember this"}),
            }],
            stop_reason: Some(StopReason::ToolUse),
            logprobs: None,
        },
        MockProvider::text_response("tool completed"),
    ]);

    let outcome = executor(registry_with_recording_tool(Arc::clone(&tool_calls)))
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("tool-turn"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "use the tool",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "use the tool",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("turn should succeed");

    assert_eq!(outcome.result.final_text, "tool completed");
    assert_eq!(outcome.result.stop_reason, LoopStopReason::Completed);
    assert_eq!(outcome.result.tool_calls.len(), 1);
    assert_eq!(outcome.result.tool_calls[0].tool_name, "record_note");

    let recorded = tool_calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(recorded, vec![json!({"note": "remember this"})]);

    let seen_messages = provider.seen_messages();
    assert_eq!(seen_messages.len(), 2);
    assert!(
        seen_messages[1]
            .iter()
            .any(|message| assistant_message_has_tool_use(message, "record_note"))
    );
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn empty_response_from_provider_still_returns_outcome() {
    let (_temp_dir, _db_file, manager) = session_manager().await;
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let execution_context = ctx(&temp_dir);
    let provider = MockProvider::new(vec![MockProvider::text_response("")]);

    let outcome = executor(empty_registry())
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("empty-response"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "respond with nothing",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "respond with nothing",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("turn should succeed");

    assert_eq!(outcome.result.final_text, "");
    assert_eq!(outcome.result.stop_reason, LoopStopReason::Completed);
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn multi_turn_conversation_preserves_context() {
    let (_temp_dir, _db_file, manager) = session_manager().await;
    let temp_dir = TempDir::new().expect("tempdir should be created");
    let execution_context = ctx(&temp_dir);
    let provider = MockProvider::new(vec![
        MockProvider::text_response("first answer"),
        MockProvider::text_response("second answer"),
    ]);
    let turn_executor = executor(empty_registry());

    let first = turn_executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("shared-session"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "first question",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "first question",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("first turn should succeed");

    let second = turn_executor
        .execute(TurnExecutionRequest {
            history: TurnHistoryAdapter {
                session_manager: Some(&manager),
                channel_name: "gateway_ws",
                session_key: Some("shared-session"),
                tenant_id: None,
                max_tokens: 8_192,
                fallback_history: &[],
            },
            run: TurnRunAdapter {
                provider: &provider,
                system_prompt: "system",
                user_message: "second question",
                response_finalization_enabled: true,
                response_contract: None,
                image_content: &[],
                model: "test-model",
                temperature: 0.0,
                inference_options: None,
                ctx: &execution_context,
                stream_sink: None,
                state_notifier: None,
            },
            transcript: TurnTranscriptAdapter {
                session_manager: Some(&manager),
                user_message: "second question",
                log_target: "tests::agent",
            },
        })
        .await
        .expect("second turn should succeed");

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(second.result.final_text, "second answer");

    let seen_messages = provider.seen_messages();
    assert_eq!(seen_messages.len(), 2);
    assert!(
        seen_messages[1]
            .iter()
            .any(|message| message_contains_text(message, "first question"))
    );
    assert!(
        seen_messages[1]
            .iter()
            .any(|message| message_contains_text(message, "first answer"))
    );
}
