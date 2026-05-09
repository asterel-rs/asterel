//! Tests for the OpenAI-compatible provider.

use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::types::{
    ChatRequest, ChatResponse, Message, ResponsesInputItem, ResponsesRequest, ResponsesResponse,
    extract_responses_text, extract_responses_tool_calls,
};
use super::*;
use crate::core::providers::streaming::StreamEvent;
use crate::core::providers::{ContentBlock, MessageRole, Provider, ProviderMessage};

fn make_provider(name: &str, url: &str, key: Option<&str>) -> OpenAiCompatProvider {
    OpenAiCompatProvider::new(name, url, key, AuthStyle::Bearer, true)
}

#[test]
fn creates_with_key() {
    let p = make_provider("venice", "https://api.venice.ai", Some("vn-key"));
    assert_eq!(p.name, "venice");
    assert_eq!(p.base_url, "https://api.venice.ai");
    assert_eq!(p.api_key.as_deref(), Some("vn-key"));
}

#[test]
fn creates_without_key() {
    let p = make_provider("test", "https://example.com", None);
    assert!(p.api_key.is_none());
}

#[test]
fn responses_endpoint_does_not_advertise_vision_when_images_are_not_mapped() {
    let p = make_provider("codex", "https://example.com/responses", Some("key"));

    assert!(!p.capabilities("gpt-5.3-codex").vision);
    assert!(p.capabilities("gpt-5.3-codex").native_tool_calling);
}

#[test]
fn strips_trailing_slash() {
    let p = make_provider("test", "https://example.com/", None);
    assert_eq!(p.base_url, "https://example.com");
}

#[tokio::test]
async fn chat_fails_without_key() {
    let p = make_provider("Venice", "https://api.venice.ai", None);
    let result = p
        .chat_with_system(None, "hello", "llama-3.3-70b", 0.7)
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Venice API key not set")
    );
}

#[test]
fn request_serializes_correctly() {
    let req = ChatRequest {
        model: "llama-3.3-70b".to_string(),
        messages: vec![
            Message {
                role: "system",
                content: "You are Asterel".to_string(),
            },
            Message {
                role: "user",
                content: "hello".to_string(),
            },
        ],
        temperature: 0.7,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("llama-3.3-70b"));
    assert!(json.contains("system"));
    assert!(json.contains("user"));
}

#[test]
fn response_deserializes() {
    let json = r#"{"choices":[{"message":{"content":"Hello from Venice!"}}]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.choices[0].message.content, "Hello from Venice!");
}

#[test]
fn response_empty_choices() {
    let json = r#"{"choices":[]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert!(resp.choices.is_empty());
}

#[test]
fn x_api_key_auth_style() {
    let p = OpenAiCompatProvider::new(
        "moonshot",
        "https://api.moonshot.cn",
        Some("ms-key"),
        AuthStyle::XApiKey,
        true,
    );
    assert!(matches!(p.auth_header, AuthStyle::XApiKey));
}

#[test]
fn custom_auth_style() {
    let p = OpenAiCompatProvider::new(
        "custom",
        "https://api.example.com",
        Some("key"),
        AuthStyle::Custom("X-Custom-Key".into()),
        true,
    );
    assert!(matches!(p.auth_header, AuthStyle::Custom(_)));
}

#[tokio::test]
async fn all_compatible_providers_fail_without_key() {
    let providers = vec![
        make_provider("Venice", "https://api.venice.ai", None),
        make_provider("Moonshot", "https://api.moonshot.cn", None),
        make_provider("GLM", "https://open.bigmodel.cn", None),
        make_provider("MiniMax", "https://api.minimax.chat", None),
        make_provider("Groq", "https://api.groq.com/openai", None),
        make_provider("Mistral", "https://api.mistral.ai", None),
        make_provider("xAI", "https://api.x.ai", None),
    ];

    for p in providers {
        let result = p.chat_with_system(None, "test", "model", 0.7).await;
        assert!(result.is_err(), "{} should fail without key", p.name);
        assert!(
            result.unwrap_err().to_string().contains("API key not set"),
            "{} error should mention key",
            p.name
        );
    }
}

#[tokio::test]
async fn chat_error_messages_redact_sensitive_fields() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            "{\"error\":\"invalid credentials api_key=raw-secret-123 access_token=eyJhbGciOiJIUzI1Ni\"}",
        ))
        .mount(&server)
        .await;

    let provider = make_provider("MockProvider", &server.uri(), Some("key"));
    let err = provider
        .chat_with_system(None, "hello", "test-model", 0.1)
        .await
        .unwrap_err()
        .to_string();

    assert!(!err.contains("raw-secret-123"));
    assert!(!err.contains("eyJhbGciOiJIUzI1Ni"));
    assert!(err.contains("[REDACTED]"));
}

#[test]
fn responses_extracts_top_level_output_text() {
    let json = r#"{"output_text":"Hello from top-level","output":[]}"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        extract_responses_text(&response).as_deref(),
        Some("Hello from top-level")
    );
}

#[test]
fn responses_extracts_nested_output_text() {
    let json = r#"{"output":[{"content":[{"type":"output_text","text":"Hello from nested"}]}]}"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        extract_responses_text(&response).as_deref(),
        Some("Hello from nested")
    );
}

#[test]
fn responses_extracts_any_text_as_fallback() {
    let json = r#"{"output":[{"content":[{"type":"message","text":"Fallback text"}]}]}"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        extract_responses_text(&response).as_deref(),
        Some("Fallback text")
    );
}

// ══════════════════════════════════════════════════════════
// Custom endpoint path tests (Issue #114)
// ══════════════════════════════════════════════════════════

#[test]
fn chat_completions_url_standard_openai() {
    // Standard OpenAI-compatible providers get /chat/completions appended
    let p = make_provider("openai", "https://api.openai.com/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn chat_completions_url_trailing_slash() {
    // Trailing slash is stripped, then /chat/completions appended
    let p = make_provider("test", "https://api.example.com/v1/", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.example.com/v1/chat/completions"
    );
}

#[test]
fn chat_completions_url_volcengine_ark() {
    // VolcEngine ARK uses custom path - should use as-is
    let p = make_provider(
        "volcengine",
        "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions",
        None,
    );
    assert_eq!(
        p.chat_completions_url(),
        "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
    );
}

#[test]
fn chat_completions_url_custom_full_endpoint() {
    // Custom provider with full endpoint path
    let p = make_provider(
        "custom",
        "https://my-api.example.com/v2/llm/chat/completions",
        None,
    );
    assert_eq!(
        p.chat_completions_url(),
        "https://my-api.example.com/v2/llm/chat/completions"
    );
}

#[test]
fn responses_url_standard() {
    // Standard providers get /v1/responses appended
    let p = make_provider("test", "https://api.example.com", None);
    assert_eq!(p.responses_url(), "https://api.example.com/v1/responses");
}

#[test]
fn responses_url_base_with_v1_does_not_duplicate_version_path() {
    let p = make_provider("test", "https://api.example.com/v1", None);
    assert_eq!(p.responses_url(), "https://api.example.com/v1/responses");
}

#[test]
fn responses_url_custom_full_endpoint() {
    // Custom provider with full responses endpoint
    let p = make_provider(
        "custom",
        "https://my-api.example.com/api/v2/responses",
        None,
    );
    assert_eq!(
        p.responses_url(),
        "https://my-api.example.com/api/v2/responses"
    );
}

#[test]
fn chat_completions_url_without_v1() {
    // Provider configured without /v1 in base URL
    let p = make_provider("test", "https://api.example.com", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.example.com/chat/completions"
    );
}

#[test]
fn chat_completions_url_base_with_v1() {
    // Provider configured with /v1 in base URL
    let p = make_provider("test", "https://api.example.com/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.example.com/v1/chat/completions"
    );
}

// ══════════════════════════════════════════════════════════
// Provider-specific endpoint tests (Issue #167)
// ══════════════════════════════════════════════════════════

#[test]
fn chat_completions_url_zai() {
    // Z.AI uses /api/paas/v4 base path
    let p = make_provider("zai", "https://api.z.ai/api/paas/v4", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
}

#[test]
fn chat_completions_url_glm() {
    // GLM (BigModel) uses /api/paas/v4 base path
    let p = make_provider("glm", "https://open.bigmodel.cn/api/paas/v4", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://open.bigmodel.cn/api/paas/v4/chat/completions"
    );
}

#[test]
fn chat_completions_url_opencode() {
    // OpenCode Zen uses /zen/v1 base path
    let p = make_provider("opencode", "https://opencode.ai/zen/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://opencode.ai/zen/v1/chat/completions"
    );
}

#[test]
fn fallback_input_includes_tool_schema_in_augmented_prompt() {
    let messages = vec![ProviderMessage {
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "read src/lib.rs".to_string(),
        }],
    }];
    let tools = vec![ToolSpec {
        name: "file_read".to_string(),
        description: "Read a file".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            }
        }),
        required_capabilities: Vec::new(),
        effect: crate::contracts::tools::ToolEffect::LocalMutation,
    }];

    let (prompt, text) =
        OpenAiCompatProvider::prepare_fallback_input(Some("System prompt"), &messages, &tools);

    assert!(prompt.contains("## Available Tools"));
    assert!(prompt.contains("file_read: Read a file"));
    assert!(text.contains("User: read src/lib.rs"));
}

#[test]
fn supports_tool_calling_returns_true_for_standard() {
    let provider = make_provider("test", "https://api.example.com", Some("key"));
    assert!(provider.supports_tools());
    assert!(provider.supports_streaming());
}

#[test]
fn supports_tool_calling_returns_true_for_responses_api() {
    let provider = OpenAiCompatProvider::new(
        "codex",
        "https://chatgpt.com/backend-api/codex/responses",
        Some("key"),
        AuthStyle::Bearer,
        false,
    );
    assert!(!provider.supports_tools());
    assert!(provider.supports_streaming());
    assert!(provider.prefer_responses_api);
}

// ══════════════════════════════════════════════════════════
// Responses API tool calling tests
// ══════════════════════════════════════════════════════════

fn sample_tools() -> Vec<crate::core::tools::traits::ToolSpec> {
    vec![crate::core::tools::traits::ToolSpec {
        name: "shell".to_string(),
        description: "Execute a shell command.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        }),
        required_capabilities: Vec::new(),
        effect: crate::contracts::tools::ToolEffect::LocalMutation,
    }]
}

#[test]
fn responses_request_serializes_with_tools() {
    use types::ResponsesTool;
    let request = ResponsesRequest {
        model: "gpt-5.3-codex".to_string(),
        input: vec![ResponsesInputItem::Message {
            role: "user",
            content: "run pwd".to_string(),
        }],
        instructions: Some("You are a helper.".to_string()),
        tools: Some(vec![ResponsesTool {
            r#type: "function",
            name: "shell".to_string(),
            description: "Execute a shell command.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"command": {"type": "string"}}
            }),
        }]),
        store: Some(false),
        stream: Some(false),
    };
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("\"type\":\"function\""));
    assert!(json.contains("\"name\":\"shell\""));
    assert!(json.contains("\"tools\":["));
}

#[test]
fn responses_input_item_message_serializes_correctly() {
    let item = ResponsesInputItem::Message {
        role: "user",
        content: "hello".to_string(),
    };
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["role"], "user");
    assert_eq!(json["content"], "hello");
    assert!(json.get("type").is_none());
}

#[test]
fn responses_input_item_function_call_output_serializes_correctly() {
    let item = ResponsesInputItem::FunctionCallOutput {
        r#type: "function_call_output",
        call_id: "call_123".to_string(),
        output: "result text".to_string(),
    };
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["type"], "function_call_output");
    assert_eq!(json["call_id"], "call_123");
    assert_eq!(json["output"], "result text");
}

#[test]
fn responses_input_item_function_call_serializes_correctly() {
    let item = ResponsesInputItem::FunctionCall {
        r#type: "function_call",
        id: "fc_1".to_string(),
        call_id: "call_1".to_string(),
        name: "shell".to_string(),
        arguments: r#"{"command":"pwd"}"#.to_string(),
    };
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["type"], "function_call");
    assert_eq!(json["call_id"], "call_1");
    assert_eq!(json["name"], "shell");
}

#[test]
fn responses_response_with_function_call_deserializes() {
    let json = r#"{
        "output": [
            {
                "type": "function_call",
                "id": "fc_001",
                "call_id": "call_001",
                "name": "shell",
                "arguments": "{\"command\":\"pwd\"}"
            }
        ],
        "status": "incomplete"
    }"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.output.len(), 1);
    assert_eq!(response.output[0].kind.as_deref(), Some("function_call"));
    assert_eq!(response.output[0].call_id.as_deref(), Some("call_001"));
    assert_eq!(response.output[0].name.as_deref(), Some("shell"));
}

#[test]
fn extract_responses_tool_calls_extracts_function_calls() {
    let json = r#"{
        "output": [
            {
                "type": "message",
                "content": [{"type": "output_text", "text": "I will run pwd."}]
            },
            {
                "type": "function_call",
                "id": "fc_001",
                "call_id": "call_001",
                "name": "shell",
                "arguments": "{\"command\":\"pwd\"}"
            }
        ],
        "status": "incomplete"
    }"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    let calls = extract_responses_tool_calls(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "call_001");
    assert_eq!(calls[0].1, "shell");
    assert_eq!(calls[0].2, r#"{"command":"pwd"}"#);
}

#[test]
fn build_responses_provider_response_text_and_tool() {
    let json = r#"{
        "output": [
            {
                "type": "message",
                "content": [{"type": "output_text", "text": "Running pwd now."}]
            },
            {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "shell",
                "arguments": "{\"command\":\"pwd\"}"
            }
        ],
        "status": "incomplete"
    }"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    let provider_response =
        OpenAiCompatProvider::build_responses_provider_response(&response, "test").unwrap();

    assert_eq!(provider_response.text, "Running pwd now.");
    assert_eq!(
        provider_response.stop_reason,
        Some(crate::core::providers::StopReason::ToolUse)
    );
    assert_eq!(provider_response.content_blocks.len(), 2);
    assert!(matches!(
        provider_response.content_blocks[0],
        ContentBlock::Text { .. }
    ));
    assert!(matches!(
        provider_response.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
}

#[test]
fn build_responses_provider_response_text_only() {
    let json = r#"{
        "output": [
            {
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }
        ],
        "status": "completed"
    }"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    let provider_response =
        OpenAiCompatProvider::build_responses_provider_response(&response, "test").unwrap();

    assert_eq!(provider_response.text, "Hello!");
    assert_eq!(
        provider_response.stop_reason,
        Some(crate::core::providers::StopReason::EndTurn)
    );
    assert_eq!(provider_response.content_blocks.len(), 1);
}

#[test]
fn build_responses_provider_response_preserves_usage() {
    let json = r#"{
        "output": [
            {
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }
        ],
        "usage": {
            "input_tokens": 123,
            "output_tokens": 45
        },
        "status": "completed"
    }"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    let provider_response =
        OpenAiCompatProvider::build_responses_provider_response(&response, "test").unwrap();

    assert_eq!(provider_response.input_tokens, Some(123));
    assert_eq!(provider_response.output_tokens, Some(45));
}

#[test]
fn build_responses_provider_response_scrubs_output_text() {
    let json = r#"{
        "output": [
            {
                "type": "message",
                "content": [{"type": "output_text", "text": "provider echoed sk-leaked-response-token"}]
            }
        ],
        "status": "completed"
    }"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    let provider_response =
        OpenAiCompatProvider::build_responses_provider_response(&response, "test").unwrap();

    assert!(!provider_response.text.contains("sk-leaked-response-token"));
    assert!(provider_response.text.contains("[REDACTED]"));
}

#[test]
fn build_responses_input_from_messages_converts_tool_history() {
    use types::build_responses_input_from_messages;

    let messages = vec![
        ProviderMessage {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: "run pwd".to_string(),
            }],
        },
        ProviderMessage {
            role: MessageRole::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"command": "pwd"}),
            }],
        },
        ProviderMessage {
            role: MessageRole::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: "/home/user".to_string(),
                is_error: false,
            }],
        },
    ];

    let items = build_responses_input_from_messages(&messages);
    assert_eq!(items.len(), 3);

    let json_items: Vec<_> = items
        .iter()
        .map(|i| serde_json::to_value(i).unwrap())
        .collect();
    assert_eq!(json_items[0]["role"], "user");
    assert_eq!(json_items[0]["content"], "run pwd");
    assert_eq!(json_items[1]["type"], "function_call");
    assert_eq!(json_items[1]["name"], "shell");
    assert_eq!(json_items[2]["type"], "function_call_output");
    assert_eq!(json_items[2]["call_id"], "call_1");
    assert_eq!(json_items[2]["output"], "/home/user");
}

#[test]
fn build_responses_input_from_messages_scrubs_text_fields() {
    use types::build_responses_input_from_messages;

    let messages = vec![ProviderMessage {
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "please use sk-leaked-input-token".to_string(),
        }],
    }];

    let items = build_responses_input_from_messages(&messages);
    let json = serde_json::to_value(&items[0]).unwrap();

    assert!(
        !json["content"]
            .as_str()
            .unwrap()
            .contains("sk-leaked-input-token")
    );
    assert!(json["content"].as_str().unwrap().contains("[REDACTED]"));
}

#[tokio::test]
async fn responses_text_request_scrubs_system_and_user_fields() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(body_string_contains("[REDACTED]"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "ok"}]
                }
            ],
            "status": "completed"
        })))
        .mount(&server)
        .await;

    let provider = OpenAiCompatProvider::new(
        "codex",
        &format!("{}/responses", server.uri()),
        Some("test-key"),
        AuthStyle::Bearer,
        false,
    );

    let response = provider
        .chat_with_system(
            Some("system has sk-leaked-system-token"),
            "user has sk-leaked-user-token",
            "gpt-5.3-codex",
            0.0,
        )
        .await
        .expect("responses request should be scrubbed and accepted");

    assert_eq!(response, "ok");
}

#[tokio::test]
async fn responses_api_tool_calling_end_to_end() {
    let server = MockServer::start().await;

    let response_body = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [{"type": "output_text", "text": "I will execute pwd."}]
            },
            {
                "type": "function_call",
                "id": "fc_001",
                "call_id": "call_001",
                "name": "shell",
                "arguments": "{\"command\":\"pwd\"}"
            }
        ],
        "status": "incomplete"
    });

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&server)
        .await;

    let provider = OpenAiCompatProvider::new(
        "codex",
        &format!("{}/responses", server.uri()),
        Some("test-key"),
        AuthStyle::Bearer,
        false,
    );

    let messages = vec![ProviderMessage {
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "shellツールでpwdを実行してください".to_string(),
        }],
    }];

    let result = provider
        .chat_with_tools(None, &messages, &sample_tools(), "gpt-5.3-codex", 0.0)
        .await
        .unwrap();

    assert_eq!(
        result.stop_reason,
        Some(crate::core::providers::StopReason::ToolUse)
    );
    assert!(result.has_tool_use());
    let tool_blocks = result.tool_use_blocks();
    assert_eq!(tool_blocks.len(), 1);
    match &tool_blocks[0] {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "call_001");
            assert_eq!(name, "shell");
            assert_eq!(input, &serde_json::json!({"command": "pwd"}));
        }
        _ => panic!("expected ToolUse block"),
    }
}

#[test]
fn build_responses_tools_creates_flat_format() {
    use types::build_responses_tools;
    let tools = sample_tools();
    let result = build_responses_tools(&tools).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].r#type, "function");
    assert_eq!(result[0].name, "shell");
}

#[test]
fn build_responses_tools_returns_none_for_empty() {
    use types::build_responses_tools;
    assert!(build_responses_tools(&[]).is_none());
}

#[test]
fn responses_unterminated_malformed_sse_event_errors() {
    let mut sent_start = false;
    let mut tool_call_index = 0;

    let err = responses::events_from_responses_sse_block(
        "event: response.output_text.delta\ndata: {not-json",
        &mut sent_start,
        &mut tool_call_index,
    )
    .expect_err("unterminated malformed SSE JSON must fail visibly");

    assert!(
        err.to_string()
            .contains("Responses stream returned malformed SSE JSON event")
    );
}

#[test]
fn responses_unterminated_valid_sse_event_is_processed() {
    let mut sent_start = false;
    let mut tool_call_index = 0;

    let events = responses::events_from_responses_sse_block(
        "event: response.output_text.delta\ndata: {\"delta\":\"hi\"}",
        &mut sent_start,
        &mut tool_call_index,
    )
    .expect("unterminated final SSE event should be drained at EOF");

    assert!(matches!(
        events.first(),
        Some(StreamEvent::ResponseStart { .. })
    ));
    assert!(matches!(
        events.get(1),
        Some(StreamEvent::TextDelta { text }) if text == "hi"
    ));
}
