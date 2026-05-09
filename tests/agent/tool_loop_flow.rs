use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use asterel::contracts::ids::EntityId;
use asterel::core::agent::{LoopStopReason, ToolLoop, ToolLoopRunParams};
use asterel::core::providers::ProviderResult;
use asterel::core::providers::response::{
    ContentBlock, ProviderMessage, ProviderResponse, StopReason,
};
use asterel::core::providers::traits::Provider;
use asterel::core::tools::middleware::{ExecutionContext, default_middleware_chain};
use asterel::core::tools::{FileReadTool, ShellTool, ToolRegistry, ToolSpec};
use asterel::security::{AutonomyLevel, EntityRateLimiter, SecurityPolicy};
use serde_json::json;
use tempfile::TempDir;

#[derive(Debug)]
struct MockProvider {
    responses: Mutex<VecDeque<ProviderResponse>>,
    seen_system_prompts: Mutex<Vec<Option<String>>>,
    seen_messages: Mutex<Vec<Vec<ProviderMessage>>>,
}

impl MockProvider {
    fn new(responses: Vec<ProviderResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            seen_system_prompts: Mutex::new(Vec::new()),
            seen_messages: Mutex::new(Vec::new()),
        }
    }

    fn seen_messages(&self) -> Vec<Vec<ProviderMessage>> {
        self.seen_messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn seen_system_prompts(&self) -> Vec<Option<String>> {
        self.seen_system_prompts
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
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        _tools: &'a [ToolSpec],
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.seen_system_prompts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(system_prompt.map(str::to_string));
            self.seen_messages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(messages.to_vec());

            let mut responses = self
                .responses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(responses.pop_front().unwrap_or_else(|| ProviderResponse {
                text: String::new(),
                input_tokens: None,
                output_tokens: None,
                model: None,
                content_blocks: vec![],
                stop_reason: Some(StopReason::EndTurn),
                logprobs: None,
            }))
        })
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn capabilities(&self, _model: &str) -> asterel::contracts::provider::ProviderCapabilities {
        asterel::contracts::provider::ProviderCapabilities {
            native_tool_calling: true,
            streaming: false,
            vision: false,
        }
    }
}

fn tool_use_response(id: &str, name: &str, input: serde_json::Value) -> ProviderResponse {
    ProviderResponse {
        text: String::new(),
        input_tokens: None,
        output_tokens: None,
        model: None,
        content_blocks: vec![ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input,
        }],
        stop_reason: Some(StopReason::ToolUse),
        logprobs: None,
    }
}

fn end_turn_text(text: &str) -> ProviderResponse {
    ProviderResponse {
        text: text.to_string(),
        input_tokens: None,
        output_tokens: None,
        model: None,
        content_blocks: vec![],
        stop_reason: Some(StopReason::EndTurn),
        logprobs: None,
    }
}

fn test_registry_and_ctx() -> (TempDir, Arc<ToolRegistry>, ExecutionContext) {
    let tmp = TempDir::new().expect("tempdir");
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let mut registry = ToolRegistry::new(default_middleware_chain());
    registry.register(Box::new(FileReadTool::new()));
    registry.register(Box::new(ShellTool::new()));

    let mut ctx = ExecutionContext::from_security(security);
    ctx.autonomy_level = AutonomyLevel::Full;
    ctx.entity_id = EntityId::from("agent:test");
    ctx.rate_limiter = Arc::new(EntityRateLimiter::new(1000, 1000));
    (tmp, Arc::new(registry), ctx)
}

fn assert_has_tool_result(messages: &[ProviderMessage], expected_text: &str) {
    let has_match = messages.iter().any(|message| {
        message.content.iter().any(|block| {
            if let ContentBlock::ToolResult { content, .. } = block {
                content.contains(expected_text)
            } else {
                false
            }
        })
    });
    assert!(
        has_match,
        "expected tool result containing '{expected_text}'"
    );
}

#[tokio::test]
async fn tool_loop_single_call() {
    let (tmp, registry, ctx) = test_registry_and_ctx();
    std::fs::write(tmp.path().join("input.txt"), "alpha").expect("write test file");
    let provider = MockProvider::new(vec![
        tool_use_response("toolu_1", "file_read", json!({"path": "input.txt"})),
        end_turn_text("done"),
    ]);

    let result = ToolLoop::new(registry, 8)
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "read it",
            image_content: &[],
            model: "test-model",
            temperature: 0.0,
            inference_options: None,
            ctx: &ctx,
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should run");

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].tool_name, "file_read");
    assert!(result.tool_calls[0].result.success);

    let seen_messages = provider.seen_messages();
    assert_eq!(seen_messages.len(), 2);
    assert_has_tool_result(&seen_messages[1], "alpha");

    let seen_prompts = provider.seen_system_prompts();
    assert!(
        seen_prompts[0]
            .as_deref()
            .is_some_and(|prompt| prompt.contains("Tool Result Trust Policy"))
    );
}

#[tokio::test]
async fn tool_loop_chain() {
    let (tmp, registry, ctx) = test_registry_and_ctx();
    std::fs::write(tmp.path().join("seq.txt"), "first").expect("write test file");
    let provider = MockProvider::new(vec![
        tool_use_response("toolu_1", "file_read", json!({"path": "seq.txt"})),
        tool_use_response("toolu_2", "shell", json!({"command": "pwd"})),
        end_turn_text("all good"),
    ]);

    let result = ToolLoop::new(registry, 8)
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "chain tools",
            image_content: &[],
            model: "test-model",
            temperature: 0.0,
            inference_options: None,
            ctx: &ctx,
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should run");

    let names: Vec<&str> = result
        .tool_calls
        .iter()
        .map(|call| call.tool_name.as_str())
        .collect();
    assert_eq!(names, vec!["file_read", "shell"]);
    assert_eq!(result.iterations, 3);
    assert_eq!(result.stop_reason, LoopStopReason::Completed);
}

#[tokio::test]
async fn tool_loop_max_iterations() {
    let (tmp, registry, ctx) = test_registry_and_ctx();
    std::fs::write(tmp.path().join("loop.txt"), "x").expect("write test file");
    let provider = MockProvider::new(vec![
        tool_use_response("toolu_1", "file_read", json!({"path": "loop.txt"})),
        tool_use_response("toolu_2", "file_read", json!({"path": "loop.txt"})),
        tool_use_response("toolu_3", "file_read", json!({"path": "loop.txt"})),
    ]);

    let result = ToolLoop::new(registry, 2)
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "keep calling",
            image_content: &[],
            model: "test-model",
            temperature: 0.0,
            inference_options: None,
            ctx: &ctx,
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should run");

    assert_eq!(result.stop_reason, LoopStopReason::MaxIterations);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls.len(), 2);
}

#[tokio::test]
async fn tool_loop_hard_cap() {
    let (tmp, registry, ctx) = test_registry_and_ctx();
    std::fs::write(tmp.path().join("hardcap.txt"), "hc").expect("write test file");

    let mut responses = Vec::new();
    for i in 0..30 {
        responses.push(tool_use_response(
            &format!("toolu_{i}"),
            "file_read",
            json!({"path": "hardcap.txt"}),
        ));
    }
    let provider = MockProvider::new(responses);

    let result = ToolLoop::new(registry, 100)
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "hard cap",
            image_content: &[],
            model: "test-model",
            temperature: 0.0,
            inference_options: None,
            ctx: &ctx,
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should run");

    assert_eq!(result.stop_reason, LoopStopReason::MaxIterations);
    assert_eq!(result.iterations, 25);
    assert_eq!(result.tool_calls.len(), 25);
}

#[tokio::test]
async fn tool_loop_error_recovery() {
    let (tmp, registry, ctx) = test_registry_and_ctx();
    std::fs::write(tmp.path().join("ok.txt"), "recovered").expect("write test file");
    let provider = MockProvider::new(vec![
        tool_use_response("toolu_1", "file_read", json!({"path": "missing.txt"})),
        tool_use_response("toolu_2", "file_read", json!({"path": "ok.txt"})),
        end_turn_text("final"),
    ]);

    let result = ToolLoop::new(registry, 8)
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "recover",
            image_content: &[],
            model: "test-model",
            temperature: 0.0,
            inference_options: None,
            ctx: &ctx,
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should run");

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(result.tool_calls.len(), 2);
    assert!(!result.tool_calls[0].result.success);
    assert!(
        result.tool_calls[0]
            .result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Failed to resolve file path"))
    );
    assert!(result.tool_calls[1].result.success);

    let seen_messages = provider.seen_messages();
    assert_has_tool_result(&seen_messages[1], "Failed to resolve file path");
}

#[tokio::test]
async fn tool_loop_no_tools() {
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        ..SecurityPolicy::default()
    });
    let mut ctx = ExecutionContext::from_security(security);
    ctx.autonomy_level = AutonomyLevel::Full;
    let provider = MockProvider::new(vec![end_turn_text("plain")]);

    let result = ToolLoop::new(Arc::new(ToolRegistry::new(default_middleware_chain())), 8)
        .run(ToolLoopRunParams {
            provider: &provider,
            system_prompt: "system",
            user_message: "just text",
            image_content: &[],
            model: "test-model",
            temperature: 0.0,
            inference_options: None,
            ctx: &ctx,
            stream_sink: None,
            conversation_history: &[],
            state_notifier: None,
            checkpoint_dir: None,
        })
        .await
        .expect("tool loop should run");

    assert_eq!(result.stop_reason, LoopStopReason::Completed);
    assert_eq!(result.iterations, 1);
    assert_eq!(result.final_text, "plain");
    assert!(result.tool_calls.is_empty());

    let seen_prompts = provider.seen_system_prompts();
    assert_eq!(seen_prompts[0].as_deref(), Some("system"));
}
