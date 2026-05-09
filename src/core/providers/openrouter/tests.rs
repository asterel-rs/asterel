//! Tests for the `OpenRouter` provider.

use super::*;
use crate::core::providers::{ContentBlock, ImageSource, MessageRole, Provider};

#[test]
fn tools_request_serializes_in_openai_function_format() {
    let messages = vec![ProviderMessage::user("list files")];
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

    let request =
        OpenRouterProvider::build_tools_request(None, &messages, &tools, "gpt-4o-mini", 0.3, None);
    let json = serde_json::to_value(&request).unwrap();

    assert_eq!(json["tools"][0]["type"], "function");
    assert_eq!(json["tools"][0]["function"]["name"], "shell");
    assert_eq!(json["tools"][0]["function"]["parameters"]["type"], "object");
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
    let messages = OpenRouterProvider::map_provider_message(&msg);
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
    let provider = OpenRouterProvider::new(Some("or-key"));
    assert!(provider.supports_tools());
}

#[test]
fn empty_or_blank_key_is_treated_as_missing() {
    let empty = OpenRouterProvider::new(Some(""));
    let blank = OpenRouterProvider::new(Some("  \t"));

    assert!(empty.cached_auth_header.is_none());
    assert!(blank.cached_auth_header.is_none());
}

#[test]
fn simple_request_scrubs_user_text_through_shared_openai_compat() {
    let req = OpenRouterProvider::build_request(
        None,
        "user has sk-openrouter-secret-token",
        "openai/gpt-4o-mini",
        0.0,
        None,
    );
    let json = serde_json::to_string(&req).unwrap();

    assert!(!json.contains("sk-openrouter-secret-token"));
    assert!(json.contains("[REDACTED]"));
}

#[test]
fn supports_tools_model_accepts_known_tool_families() {
    let provider = OpenRouterProvider::new(Some("or-key"));
    assert!(provider.supports_tools_model("openai/gpt-4o-mini"));
    assert!(provider.supports_tools_model("anthropic/claude-3.7-sonnet"));
    assert!(provider.supports_tools_model("qwen/qwen2.5-72b-instruct"));
}

#[test]
fn supports_tools_model_rejects_unknown_or_empty_models() {
    let provider = OpenRouterProvider::new(Some("or-key"));
    assert!(!provider.supports_tools_model(""));
    assert!(!provider.supports_tools_model("google/imagen-4"));
    assert!(!provider.supports_tools_model("runway/gen-4-turbo"));
}

#[test]
fn capability_profile_uses_model_specific_tool_support() {
    let provider = OpenRouterProvider::new(Some("or-key"));

    assert!(
        provider
            .capability_profile("openai/gpt-4o-mini")
            .native
            .native_tool_calling
    );
    assert!(
        !provider
            .capability_profile("google/imagen-4")
            .native
            .native_tool_calling
    );
}

#[test]
fn supports_vision_returns_true() {
    let provider = OpenRouterProvider::new(Some("test-key"));
    assert!(provider.supports_vision());
}

#[test]
fn supports_vision_model_accepts_known_multimodal_families() {
    let provider = OpenRouterProvider::new(Some("test-key"));
    assert!(provider.supports_vision_model("openai/gpt-4o-mini"));
    assert!(provider.supports_vision_model("anthropic/claude-3.7-sonnet"));
    assert!(provider.supports_vision_model("qwen/qwen2-vl-72b-instruct"));
}

#[test]
fn supports_vision_model_rejects_unknown_text_only_families() {
    let provider = OpenRouterProvider::new(Some("test-key"));
    assert!(!provider.supports_vision_model("deepseek/deepseek-chat"));
    assert!(!provider.supports_vision_model("meta-llama/llama-3.1-70b-instruct"));
    assert!(!provider.supports_vision_model(""));
}

#[test]
fn capability_profile_uses_model_specific_vision_support() {
    let provider = OpenRouterProvider::new(Some("test-key"));

    assert!(
        provider
            .capability_profile("openai/gpt-4o-mini")
            .native
            .vision
    );
    assert!(
        !provider
            .capability_profile("deepseek/deepseek-chat")
            .native
            .vision
    );
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
fn prepare_fallback_input_augments_prompt_and_flattens_messages() {
    let messages = vec![
        ProviderMessage::user("list files"),
        ProviderMessage::assistant("Working on it."),
    ];
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

    let (prompt, text) =
        OpenRouterProvider::prepare_fallback_input(Some("System policy"), &messages, &tools);

    assert!(prompt.contains("System policy"));
    assert!(prompt.contains("## Available Tools"));
    assert!(prompt.contains("<tool_call>"));
    assert!(prompt.contains("shell"));
    assert!(text.contains("User: list files"));
    assert!(text.contains("Assistant: Working on it."));
}

#[test]
fn supports_streaming_returns_true() {
    let provider = OpenRouterProvider::new(Some("or-key"));
    assert!(provider.supports_streaming());
}
