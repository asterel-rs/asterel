//! Tests for the Gemini provider.

use super::types::CandidateContent;
use super::*;
use crate::core::providers::streaming::StreamEvent;
use crate::core::providers::{InferenceOpts, Provider, ProviderError, ThinkingLevel};
use crate::core::tools::traits::ToolSpec;

#[test]
fn provider_creates_without_key() {
    let _provider = GeminiProvider::new(None);
}

#[test]
fn provider_creates_with_key() {
    let provider = GeminiProvider::new(Some("test-api-key"));
    assert_eq!(
        provider.auth,
        Some(GeminiResolvedAuth::ApiKey("test-api-key".to_string()))
    );
}

#[test]
fn gemini_cli_dir_returns_path() {
    let dir = GeminiProvider::gemini_cli_dir();
    // Should return Some on systems with home dir
    if UserDirs::new().is_some() {
        assert!(dir.is_some());
        assert!(dir.unwrap().ends_with(".gemini"));
    }
}

#[test]
fn auth_source_reports_correctly() {
    let provider = GeminiProvider::new(Some("explicit-key"));
    // With explicit key, should report "config" (unless CLI credentials exist)
    let source = provider.auth_source();
    // Should be either "config" or "Gemini CLI OAuth" if CLI is configured
    assert!(source == "config" || source == "Gemini CLI OAuth");
}

#[test]
fn model_name_formatting() {
    // Test that model names are formatted correctly
    let model = "gemini-2.0-flash";
    let formatted = if model.starts_with("models/") {
        model.to_string()
    } else {
        format!("models/{model}")
    };
    assert_eq!(formatted, "models/gemini-2.0-flash");

    // Already prefixed
    let model2 = "models/gemini-1.5-pro";
    let formatted2 = if model2.starts_with("models/") {
        model2.to_string()
    } else {
        format!("models/{model2}")
    };
    assert_eq!(formatted2, "models/gemini-1.5-pro");
}

#[test]
fn request_serialization() {
    let request = GenerateContentRequest {
        contents: vec![Content {
            role: Some("user".to_string()),
            parts: vec![Part::text("Hello".to_string())],
        }],
        system_instruction: Some(Content {
            role: None,
            parts: vec![Part::text("You are helpful".to_string())],
        }),
        tools: None,
        generation_config: GenerationConfig {
            temperature: 0.7,
            max_output_tokens: 8192,
            top_p: None,
            thinking_config: None,
        },
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("\"role\":\"user\""));
    assert!(json.contains("\"text\":\"Hello\""));
    assert!(json.contains("\"temperature\":0.7"));
    assert!(json.contains("\"maxOutputTokens\":8192"));
}

#[test]
fn build_request_sets_thinking_config_when_enabled() {
    let options = InferenceOpts::from_thinking_level(ThinkingLevel::Low);
    let request = GeminiProvider::build_request(None, "hello", 0.2, Some(&options));
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(
        value["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
    assert_eq!(
        value["generationConfig"]["thinkingConfig"]["includeThoughts"],
        false
    );
}

#[test]
fn response_deserialization() {
    let json = r#"{
        "candidates": [{
            "content": {
                "parts": [{"text": "Hello there!"}]
            }
        }]
    }"#;

    let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
    assert!(response.candidates.is_some());
    let text = response
        .candidates
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .content
        .parts
        .into_iter()
        .next()
        .unwrap()
        .text;
    assert_eq!(text, Some("Hello there!".to_string()));
}

#[test]
fn gemini_tools_serialize_as_function_declarations() {
    let tools = vec![ToolSpec {
        name: "shell".to_string(),
        description: "Execute shell command".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        }),
        required_capabilities: Vec::new(),
        effect: crate::contracts::tools::ToolEffect::LocalMutation,
    }];

    let request = GeminiProvider::build_tools_request(
        None,
        &[ProviderMessage::user("list files")],
        &tools,
        0.1,
        None,
    );
    let value = serde_json::to_value(&request).unwrap();

    assert_eq!(
        value["tools"][0]["function_declarations"][0]["name"],
        "shell"
    );
    assert_eq!(
        value["tools"][0]["function_declarations"][0]["parameters"]["type"],
        "object"
    );
}

#[test]
fn gemini_function_call_response_parses_to_tool_use_block() {
    let json = r#"{
        "candidates": [{
            "content": {
                "parts": [{"functionCall": {"name": "shell", "args": {"command": "ls"}}}]
            },
            "finishReason": "FUNCTION_CALL"
        }]
    }"#;

    let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
    let candidate = response.candidates.unwrap().into_iter().next().unwrap();
    let blocks = GeminiProvider::parse_content_blocks(&candidate.content.parts);

    assert!(matches!(
        &blocks[0],
        ContentBlock::ToolUse { name, input, .. }
        if name == "shell" && input == &serde_json::json!({"command": "ls"})
    ));
}

#[test]
fn gemini_finish_reason_mapping_handles_tool_calls() {
    let with_tool_call = Candidate {
        content: CandidateContent {
            parts: vec![ResponsePart {
                text: None,
                function_call: Some(GeminiFunctionCall {
                    name: "shell".to_string(),
                    args: serde_json::json!({"command": "ls"}),
                    id: None,
                }),
            }],
        },
        finish_reason: Some("STOP".to_string()),
    };
    let max_tokens = Candidate {
        content: CandidateContent {
            parts: vec![ResponsePart {
                text: Some("x".to_string()),
                function_call: None,
            }],
        },
        finish_reason: Some("MAX_TOKENS".to_string()),
    };

    assert_eq!(
        GeminiProvider::map_stop_reason(&with_tool_call),
        StopReason::ToolUse
    );
    assert_eq!(
        GeminiProvider::map_stop_reason(&max_tokens),
        StopReason::MaxTokens
    );
}

#[test]
fn map_provider_message_handles_image_block() {
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
    let tool_map = std::collections::HashMap::new();
    let content = GeminiProvider::map_provider_message(&msg, &tool_map);
    assert_eq!(content.role.as_deref(), Some("user"));
    assert_eq!(content.parts.len(), 2);
    let json = serde_json::to_value(&content).unwrap();
    assert!(json["parts"][0]["text"].is_string());
    assert_eq!(json["parts"][1]["inlineData"]["mimeType"], "image/png");
}

#[test]
fn supports_tool_calling_returns_true() {
    let provider = GeminiProvider::new(Some("test-api-key"));
    assert!(provider.supports_tools());
}

#[test]
fn supports_vision_returns_true() {
    let provider = GeminiProvider::new(Some("test-key"));
    assert!(provider.supports_vision());
}

#[test]
fn supports_streaming_returns_true() {
    let provider = GeminiProvider::new(Some("test-api-key"));
    assert!(provider.supports_streaming());
}

#[test]
fn build_request_scrubs_system_and_user_text() {
    let request = GeminiProvider::build_request(
        Some("system has sk-leaked-system-token"),
        "user has sk-leaked-user-token",
        0.0,
        None,
    );
    let json = serde_json::to_value(&request).unwrap();

    assert!(!json.to_string().contains("sk-leaked-system-token"));
    assert!(!json.to_string().contains("sk-leaked-user-token"));
    assert!(json.to_string().contains("[REDACTED]"));
}

#[test]
fn build_tools_request_scrubs_message_and_tool_description_text() {
    let messages = vec![ProviderMessage {
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "message has sk-leaked-message-token".to_string(),
        }],
    }];
    let tools = vec![crate::core::tools::traits::ToolSpec {
        name: "lookup".to_string(),
        description: "tool has sk-leaked-tool-token".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        required_capabilities: Vec::new(),
        effect: crate::contracts::tools::ToolEffect::ReadOnly,
    }];

    let request = GeminiProvider::build_tools_request(None, &messages, &tools, 0.0, None);
    let json = serde_json::to_value(&request).unwrap();

    assert!(!json.to_string().contains("sk-leaked-message-token"));
    assert!(!json.to_string().contains("sk-leaked-tool-token"));
    assert!(json.to_string().contains("[REDACTED]"));
}

#[test]
fn capabilities_are_model_specific_for_non_generation_models() {
    let provider = GeminiProvider::new(Some("test-api-key"));

    let embedding = provider.capabilities("text-embedding-004");
    assert!(!embedding.native_tool_calling);
    assert!(!embedding.vision);

    let current = provider.capabilities("gemini-2.5-flash");
    assert!(current.native_tool_calling);
    assert!(current.vision);
}

#[test]
fn parse_sse_data_lines_basic() {
    let chunk = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]}}]}\n\n"
    );
    let lines = parse_data_lines(chunk);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0],
        "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}"
    );
    assert_eq!(
        lines[1],
        "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]}}]}"
    );
}

#[test]
fn parse_sse_data_lines_empty() {
    let lines = parse_data_lines("");
    assert!(lines.is_empty());
}

#[test]
fn gemini_unterminated_malformed_sse_event_errors() {
    let mut sent_start = false;
    let mut tool_call_index = 1;

    let err = GeminiProvider::events_from_gemini_sse_block(
        "data: {not-json",
        &mut sent_start,
        &mut tool_call_index,
    )
    .expect_err("unterminated malformed SSE JSON must fail visibly");

    assert!(
        err.to_string()
            .contains("Gemini stream returned malformed SSE JSON chunk")
    );
}

#[test]
fn gemini_unterminated_valid_sse_event_is_processed() {
    let mut sent_start = false;
    let mut tool_call_index = 1;

    let events = GeminiProvider::events_from_gemini_sse_block(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}",
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

#[test]
fn error_response_deserialization() {
    let json = r#"{
        "error": {
            "message": "Invalid API key"
        }
    }"#;

    let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
    assert!(response.error.is_some());
    assert_eq!(response.error.unwrap().message, "Invalid API key");
}

#[test]
fn embedded_error_classification_marks_quota_as_non_retryable() {
    let err = GeminiProvider::classify_embedded_api_error("insufficient_quota: billing");
    assert!(matches!(err, ProviderError::QuotaExhausted { .. }));
    assert!(err.is_non_retryable());
}

#[test]
fn embedded_error_classification_marks_auth_for_oauth_recovery() {
    let err = GeminiProvider::classify_embedded_api_error("Unauthorized: invalid API key");
    assert!(matches!(err, ProviderError::Auth { .. }));
    assert!(err.is_non_retryable());
    assert!(err.is_auth_error());
}

#[test]
fn embedded_error_classification_redacts_secret_material() {
    let err = GeminiProvider::classify_embedded_api_error(
        "Unauthorized: invalid API key api_key=sk-secret-value",
    );
    let rendered = err.to_string();
    assert!(!rendered.contains("sk-secret-value"));
    assert!(rendered.contains("[REDACTED]"));
}
