//! Tests for the `OpenAI` provider.

use super::*;
use crate::core::providers::streaming::StreamEvent;
use crate::core::providers::{InferenceOpts, Provider, ThinkingLevel};

#[test]
fn creates_with_key() {
    let p = OpenAiProvider::new(Some("sk-proj-abc123"));
    assert_eq!(
        p.cached_auth_header.as_deref(),
        Some("Bearer sk-proj-abc123")
    );
}

#[test]
fn creates_without_key() {
    let p = OpenAiProvider::new(None);
    assert!(p.cached_auth_header.is_none());
}

#[test]
fn creates_with_empty_key() {
    let p = OpenAiProvider::new(Some(""));
    assert!(p.cached_auth_header.is_none());
}

#[test]
fn creates_with_blank_key_as_missing() {
    let p = OpenAiProvider::new(Some("  \t"));
    assert!(p.cached_auth_header.is_none());
}

#[tokio::test]
async fn chat_fails_without_key() {
    let p = OpenAiProvider::new(None);
    let result = p.chat_with_system(None, "hello", "gpt-4o", 0.7).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("API key not set"));
}

#[tokio::test]
async fn chat_with_system_fails_without_key() {
    let p = OpenAiProvider::new(None);
    let result = p
        .chat_with_system(Some("You are Asterel"), "test", "gpt-4o", 0.5)
        .await;
    assert!(result.is_err());
}

#[test]
fn request_serializes_with_system_message() {
    let req = OpenAiProvider::build_request(Some("You are Asterel"), "hello", "gpt-4o", 0.7, None);
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"role\":\"system\""));
    assert!(json.contains("\"role\":\"user\""));
    assert!(json.contains("gpt-4o"));
}

#[test]
fn request_serializes_without_system() {
    let req = OpenAiProvider::build_request(None, "hello", "gpt-4o", 0.0, None);
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("system"));
    assert!(json.contains("\"temperature\":0.0"));
    assert!(!json.contains("\"tools\":"));
}

#[test]
fn simple_request_scrubs_system_and_user_text() {
    let req = OpenAiProvider::build_request(
        Some("system has sk-system-secret-token"),
        "user has sk-user-secret-token",
        "gpt-4o",
        0.0,
        None,
    );
    let json = serde_json::to_string(&req).unwrap();

    assert!(!json.contains("sk-system-secret-token"));
    assert!(!json.contains("sk-user-secret-token"));
    assert_eq!(json.matches("[REDACTED]").count(), 2);
}

#[test]
fn request_serializes_reasoning_effort_when_thinking_enabled() {
    let options = InferenceOpts::from_thinking_level(ThinkingLevel::High);
    let req = OpenAiProvider::build_request(None, "hello", "gpt-4o", 0.0, Some(&options));
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"reasoning_effort\":\"high\""));
}

#[test]
fn response_deserializes_single_choice() {
    let json = r#"{"choices":[{"message":{"content":"Hi!"}}]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.choices.len(), 1);
    assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hi!"));
}

#[test]
fn response_deserializes_empty_choices() {
    let json = r#"{"choices":[]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert!(resp.choices.is_empty());
}

#[test]
fn response_deserializes_multiple_choices() {
    let json = r#"{"choices":[{"message":{"content":"A"}},{"message":{"content":"B"}}]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.choices.len(), 2);
    assert_eq!(resp.choices[0].message.content.as_deref(), Some("A"));
}

#[test]
fn response_with_unicode() {
    let json = r#"{"choices":[{"message":{"content":"こんにちは 🦀"}}]}"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        resp.choices[0].message.content.as_deref(),
        Some("こんにちは 🦀")
    );
}

#[test]
fn response_with_long_content() {
    let long = "x".repeat(100_000);
    let json = format!(r#"{{"choices":[{{"message":{{"content":"{long}"}}}}]}}"#);
    let resp: ChatResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(
        resp.choices[0].message.content.as_deref().map(str::len),
        Some(100_000)
    );
}

#[test]
fn tools_request_serializes_in_openai_function_format() {
    let messages = vec![ProviderMessage {
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "list files".to_string(),
        }],
    }];
    let tools = vec![ToolSpec {
        name: "shell".to_string(),
        description: "Execute a shell command".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"}
            },
            "required": ["command"]
        }),
        required_capabilities: Vec::new(),
        effect: crate::contracts::tools::ToolEffect::LocalMutation,
    }];

    let req = OpenAiProvider::build_tools_request(None, &messages, &tools, "gpt-4o", 0.2, None);
    let json = serde_json::to_value(&req).unwrap();

    assert_eq!(json["tools"][0]["type"], "function");
    assert_eq!(json["tools"][0]["function"]["name"], "shell");
    assert_eq!(
        json["tools"][0]["function"]["description"],
        "Execute a shell command"
    );
    assert_eq!(json["tools"][0]["function"]["parameters"]["type"], "object");
}

#[test]
fn request_without_tools_omits_tools_field() {
    let req = OpenAiProvider::build_tools_request(None, &[], &[], "gpt-4o", 0.1, None);
    let json = serde_json::to_value(&req).unwrap();

    assert!(json.get("tools").is_none());
}

#[test]
fn response_tool_calls_deserialize_and_parse_to_content_blocks() {
    let json = r#"{
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "shell",
                        "arguments": "{\"command\":\"ls\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }"#;
    let resp: ChatResponse = serde_json::from_str(json).unwrap();
    let blocks =
        OpenAiProvider::parse_tool_calls(resp.choices[0].message.tool_calls.clone()).unwrap();

    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "call_abc123");
            assert_eq!(name, "shell");
            assert_eq!(input, &serde_json::json!({"command": "ls"}));
        }
        _ => panic!("expected tool use block"),
    }
}

#[test]
fn finish_reason_mapping_tool_calls_and_stop() {
    assert_eq!(
        OpenAiProvider::map_finish_reason(Some("tool_calls")),
        StopReason::ToolUse
    );
    assert_eq!(
        OpenAiProvider::map_finish_reason(Some("stop")),
        StopReason::EndTurn
    );
}

#[test]
fn map_provider_message_handles_image_block() {
    let msg = ProviderMessage {
        role: MessageRole::User,
        content: vec![
            ContentBlock::Text {
                text: "What's this?".to_string(),
            },
            ContentBlock::Image {
                source: ImageSource::base64("image/jpeg", "abc123"),
            },
        ],
    };
    let messages = OpenAiProvider::map_provider_message(&msg);
    assert_eq!(messages.len(), 1);
    let json = serde_json::to_value(&messages[0]).unwrap();
    let content = json["content"].as_array().expect("content should be array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert!(
        content[1]["image_url"]["url"]
            .as_str()
            .expect("image url should be string")
            .starts_with("data:image/jpeg;base64,")
    );
}

#[test]
fn supports_tool_calling_returns_true() {
    let provider = OpenAiProvider::new(Some("sk-test"));
    assert!(provider.supports_tools());
}

#[test]
fn supports_vision_returns_true() {
    let provider = OpenAiProvider::new(Some("test-key"));
    assert!(provider.supports_vision());
}

#[test]
fn capabilities_are_model_specific_for_legacy_text_models() {
    let provider = OpenAiProvider::new(Some("test-key"));

    let legacy = provider.capabilities("text-davinci-003");
    assert!(!legacy.native_tool_calling);
    assert!(!legacy.vision);

    let current = provider.capabilities("gpt-5-mini");
    assert!(current.native_tool_calling);
    assert!(current.vision);
}

#[test]
fn parse_sse_data_lines_basic() {
    let chunk = "data: {\"choices\":[]}\n\n";
    let lines = parse_data_lines_no_done(chunk);
    assert_eq!(lines, vec!["{\"choices\":[]}"]);
}

#[test]
fn parse_sse_data_lines_done_filtered() {
    let chunk = "data: [DONE]\n\ndata: {\"choices\":[]}\n\n";
    let lines = parse_data_lines_no_done(chunk);
    assert_eq!(lines, vec!["{\"choices\":[]}"]);
}

#[test]
fn openai_compatible_malformed_sse_json_errors() {
    let mut sent_start = false;
    let error = compat::events_from_openai_sse_block("data: {not-json}\n\n", &mut sent_start)
        .expect_err("malformed JSON chunks must fail visibly");

    assert!(
        error
            .to_string()
            .contains("OpenAI-compatible stream returned malformed SSE JSON chunk")
    );
    assert!(!sent_start);
}

#[test]
fn openai_compatible_valid_sse_json_yields_events() {
    let mut sent_start = false;
    let events = compat::events_from_openai_sse_block(
        "data: {\"model\":\"gpt-test\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
        &mut sent_start,
    )
    .expect("valid chunks should still parse");

    assert!(sent_start);
    assert!(matches!(
        events.as_slice(),
        [StreamEvent::ResponseStart { .. }, StreamEvent::TextDelta { text }] if text == "hi"
    ));
}

#[test]
fn supports_streaming_returns_true() {
    let provider = OpenAiProvider::new(Some("sk-test"));
    assert!(provider.supports_streaming());
}
