//! Tests for the Anthropic provider.

use super::*;
use crate::core::providers::{InferenceOpts, ThinkingLevel};

#[test]
fn creates_with_key() {
    let p = AnthropicProvider::new(Some("sk-ant-test123"));
    assert!(p.cached_auth.is_some());
    let (name, value) = p.cached_auth.as_ref().unwrap();
    assert_eq!(*name, "x-api-key");
    assert_eq!(value, "sk-ant-test123");
    assert_eq!(
        p.cached_messages_url,
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn creates_without_key() {
    let p = AnthropicProvider::new(None);
    assert!(p.cached_auth.is_none());
    assert_eq!(
        p.cached_messages_url,
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn creates_with_empty_key() {
    let p = AnthropicProvider::new(Some(""));
    assert!(p.cached_auth.is_none());
}

#[test]
fn creates_with_whitespace_key() {
    let p = AnthropicProvider::new(Some("  sk-ant-test123  "));
    assert!(p.cached_auth.is_some());
    let (name, value) = p.cached_auth.as_ref().unwrap();
    assert_eq!(*name, "x-api-key");
    assert_eq!(value, "sk-ant-test123");
}

#[test]
fn creates_with_custom_base_url() {
    let p = AnthropicProvider::with_base_url(Some("sk-ant-test"), Some("https://api.example.com"));
    assert_eq!(p.cached_messages_url, "https://api.example.com/v1/messages");
    let (name, value) = p.cached_auth.as_ref().unwrap();
    assert_eq!(*name, "x-api-key");
    assert_eq!(value, "sk-ant-test");
}

#[test]
fn custom_base_url_trims_trailing_slash() {
    let p = AnthropicProvider::with_base_url(None, Some("https://api.example.com/"));
    assert_eq!(p.cached_messages_url, "https://api.example.com/v1/messages");
}

#[test]
fn default_base_url_when_none_provided() {
    let p = AnthropicProvider::with_base_url(None, None);
    assert_eq!(
        p.cached_messages_url,
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn setup_token_uses_bearer_auth() {
    let p = AnthropicProvider::new(Some("sk-ant-oat01-abc123"));
    let (name, value) = p.cached_auth.as_ref().unwrap();
    assert_eq!(*name, "Authorization");
    assert_eq!(value, "Bearer sk-ant-oat01-abc123");
}

#[tokio::test]
async fn chat_fails_without_key() {
    let p = AnthropicProvider::new(None);
    let result = p
        .chat_with_system(None, "hello", "claude-3-opus", 0.7)
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("credentials not configured"),
        "Expected key error, got: {err}"
    );
}

#[test]
fn setup_token_detection_works() {
    assert!(AnthropicProvider::is_setup_token("sk-ant-oat01-abcdef"));
    assert!(!AnthropicProvider::is_setup_token("sk-ant-api-key"));
}

#[tokio::test]
async fn chat_with_system_fails_without_key() {
    let p = AnthropicProvider::new(None);
    let result = p
        .chat_with_system(Some("You are Asterel"), "hello", "claude-3-opus", 0.7)
        .await;
    assert!(result.is_err());
}

#[test]
fn chat_request_serializes_without_system() {
    let req = ChatRequest {
        model: "claude-3-opus".to_string(),
        max_tokens: 4096,
        system: None,
        messages: vec![Message {
            role: "user",
            content: MessageContent::Text("hello".to_string()),
        }],
        tools: None,
        temperature: 0.7,
        top_p: None,
        thinking: None,
        stream: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(
        !json.contains("system"),
        "system field should be skipped when None"
    );
    assert!(!json.contains("\"tools\""));
    assert!(json.contains("claude-3-opus"));
    assert!(json.contains("hello"));
}

#[test]
fn chat_request_serializes_with_system() {
    let req = ChatRequest {
        model: "claude-3-opus".to_string(),
        max_tokens: 4096,
        system: Some(vec![SystemBlock {
            r#type: "text",
            text: "You are Asterel".to_string(),
            cache_control: Some(CacheControl::ephemeral()),
        }]),
        messages: vec![Message {
            role: "user",
            content: MessageContent::Text("hello".to_string()),
        }],
        tools: None,
        temperature: 0.7,
        top_p: None,
        thinking: None,
        stream: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["system"][0]["type"], "text");
    assert_eq!(json["system"][0]["text"], "You are Asterel");
    assert_eq!(json["system"][0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn chat_request_serializes_with_tools() {
    let req = ChatRequest {
        model: "claude-3-opus".to_string(),
        max_tokens: 4096,
        system: None,
        messages: vec![Message {
            role: "user",
            content: MessageContent::Text("hello".to_string()),
        }],
        tools: Some(vec![AnthropicToolDef {
            name: "shell".to_string(),
            description: "Run a shell command".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }),
            cache_control: None,
        }]),
        temperature: 0.7,
        top_p: None,
        thinking: None,
        stream: None,
    };

    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["tools"][0]["name"], "shell");
    assert_eq!(json["tools"][0]["description"], "Run a shell command");
    assert_eq!(json["tools"][0]["input_schema"]["type"], "object");
}

#[test]
fn build_request_sets_thinking_budget_when_enabled() {
    let options = InferenceOpts::from_thinking_level(ThinkingLevel::Medium);
    let req = AnthropicProvider::build_request(None, "hello", "claude-3-opus", 0.7, Some(&options));
    let value = serde_json::to_value(&req).unwrap();
    assert_eq!(value["thinking"]["type"], "enabled");
    assert_eq!(value["thinking"]["budget_tokens"], 2048);
}

#[test]
fn chat_response_deserializes() {
    let json = r#"{"content":[{"type":"text","text":"Hello there!"}]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.content.len(), 1);
    assert!(matches!(
        &resp.content[0],
        ResponseContentBlock::Text { text } if text == "Hello there!"
    ));
}

#[test]
fn chat_response_empty_content() {
    let json = r#"{"content":[]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert!(resp.content.is_empty());
}

#[test]
fn chat_response_multiple_blocks() {
    let json = r#"{"content":[{"type":"text","text":"First"},{"type":"text","text":"Second"}]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.content.len(), 2);
    assert!(matches!(
        &resp.content[0],
        ResponseContentBlock::Text { text } if text == "First"
    ));
    assert!(matches!(
        &resp.content[1],
        ResponseContentBlock::Text { text } if text == "Second"
    ));
}

#[test]
fn chat_response_tool_use_block_maps_to_provider_content_block() {
    let json = r#"{
        "content":[
            {"type":"tool_use","id":"toolu_1","name":"shell","input":{"command":"ls"}}
        ],
        "stop_reason":"tool_use"
    }"#;

    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    let blocks = AnthropicProvider::parse_content_blocks(&resp.content);

    assert_eq!(blocks.len(), 1);
    assert!(matches!(
        &blocks[0],
        ContentBlock::ToolUse { id, name, input }
        if id == "toolu_1" && name == "shell" && input == &serde_json::json!({"command": "ls"})
    ));
}

#[test]
fn map_stop_reason_handles_tool_use() {
    let reason = AnthropicProvider::map_stop_reason(Some("tool_use"));
    assert_eq!(reason, Some(StopReason::ToolUse));
}

#[test]
fn map_stop_reason_handles_end_turn() {
    let reason = AnthropicProvider::map_stop_reason(Some("end_turn"));
    assert_eq!(reason, Some(StopReason::EndTurn));
}

#[test]
fn provider_message_to_message_supports_tool_result_and_mixed_assistant_blocks() {
    let assistant = ProviderMessage {
        role: MessageRole::Assistant,
        content: vec![
            ContentBlock::Text {
                text: "Running shell".to_string(),
            },
            ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"command": "ls"}),
            },
        ],
    };
    let tool_result = ProviderMessage::tool_result("toolu_1", "src", false);

    let assistant_message = AnthropicProvider::provider_message_to_message(&assistant);
    let tool_result_message = AnthropicProvider::provider_message_to_message(&tool_result);

    let assistant_json = serde_json::to_value(&assistant_message).unwrap();
    let tool_result_json = serde_json::to_value(&tool_result_message).unwrap();

    assert_eq!(assistant_json["role"], "assistant");
    assert_eq!(assistant_json["content"][0]["type"], "text");
    assert_eq!(assistant_json["content"][1]["type"], "tool_use");
    assert_eq!(tool_result_json["role"], "user");
    assert_eq!(tool_result_json["content"][0]["type"], "tool_result");
    assert_eq!(tool_result_json["content"][0]["tool_use_id"], "toolu_1");
    assert_eq!(tool_result_json["content"][0]["content"], "src");
}

#[test]
fn provider_message_to_message_maps_image_block() {
    let msg = ProviderMessage {
        role: MessageRole::User,
        content: vec![
            ContentBlock::Text {
                text: "Describe this".to_string(),
            },
            ContentBlock::Image {
                source: ImageSource::base64("image/png", "iVBOR"),
            },
        ],
    };
    let mapped = AnthropicProvider::provider_message_to_message(&msg);
    assert_eq!(mapped.role, "user");

    let json = serde_json::to_value(&mapped.content).unwrap();
    let blocks = json.as_array().expect("content should be array");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[1]["source"]["type"], "base64");
    assert_eq!(blocks[1]["source"]["media_type"], "image/png");
}

#[test]
fn supports_tool_calling_returns_true() {
    let provider = AnthropicProvider::new(Some("sk-ant-test123"));
    assert!(provider.supports_tools());
}

#[test]
fn supports_streaming_returns_true() {
    let provider = AnthropicProvider::new(Some("sk-ant-test123"));
    assert!(provider.supports_streaming());
}

#[test]
fn supports_vision_returns_true() {
    let provider = AnthropicProvider::new(Some("test-key"));
    assert!(provider.supports_vision());
}

#[test]
fn provider_message_to_message_scrubs_text_and_tool_results() {
    let msg = ProviderMessage {
        role: MessageRole::User,
        content: vec![
            ContentBlock::Text {
                text: "message has sk-leaked-message-token".to_string(),
            },
            ContentBlock::ToolResult {
                tool_use_id: "toolu_1".to_string(),
                content: "result has sk-leaked-result-token".to_string(),
                is_error: false,
            },
        ],
    };

    let mapped = AnthropicProvider::provider_message_to_message(&msg);
    let json = serde_json::to_value(&mapped).unwrap();

    assert!(!json.to_string().contains("sk-leaked-message-token"));
    assert!(!json.to_string().contains("sk-leaked-result-token"));
    assert!(json.to_string().contains("[REDACTED]"));
}

#[test]
fn capabilities_are_model_specific_for_legacy_text_models() {
    let provider = AnthropicProvider::new(Some("test-key"));

    let legacy = provider.capabilities("claude-2.1");
    assert!(!legacy.native_tool_calling);
    assert!(!legacy.vision);

    let current = provider.capabilities("claude-sonnet-4-20250514");
    assert!(current.native_tool_calling);
    assert!(current.vision);
}

#[test]
fn parse_sse_events_basic() {
    let chunk = "event: message_start\ndata: {\"message\":{}}\n\n";
    let events = parse_event_pairs(chunk);
    assert_eq!(events, vec![("message_start", "{\"message\":{}}")]);
}

#[test]
fn parse_sse_events_multiple() {
    let chunk = concat!(
        "event: message_start\n",
        "data: {\"message\":{}}\n\n",
        "event: content_block_delta\n",
        "data: {\"delta\":{}}\n\n"
    );
    let events = parse_event_pairs(chunk);
    assert_eq!(
        events,
        vec![
            ("message_start", "{\"message\":{}}"),
            ("content_block_delta", "{\"delta\":{}}")
        ]
    );
}

#[test]
fn parse_sse_events_empty() {
    let events = parse_event_pairs("");
    assert!(events.is_empty());
}

#[test]
fn temperature_range_serializes() {
    for temp in [0.0, 0.5, 1.0, 2.0] {
        let req = ChatRequest {
            model: "claude-3-opus".to_string(),
            max_tokens: 4096,
            system: None,
            messages: vec![],
            tools: None,
            temperature: temp,
            top_p: None,
            thinking: None,
            stream: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(&format!("{temp}")));
    }
}
