//! XML-based tool calling fallback for providers without native tool support.
//!
//! When a provider does not support structured tool calling, this module
//! augments the system prompt with inline tool schema descriptions and parses
//! `<tool_call>` XML blocks from the model's free-text response.
//!
//! Used by `Ollama`, `Codex` CLI, and `OpenRouter` models that fail the
//! native-tool capability check.

use crate::core::providers::response::{ContentBlock, ProviderResponse, StopReason};
use crate::core::tools::traits::ToolSpec;

pub use crate::core::agent::tool_protocol::ParsedToolCall as ExtractedToolCall;

/// Inject tool schemas into the system prompt so the model can emit
/// `<tool_call>` XML blocks in its response.
#[must_use]
pub fn augment_prompt_with_tools(system_prompt: &str, tools: &[ToolSpec]) -> String {
    let instructions = crate::core::agent::tool_protocol::render_tool_instructions(tools);
    format!("{system_prompt}\n{instructions}")
}

/// Extract `<tool_call>` XML blocks from a model response.
///
/// Returns `(remaining_text, tool_calls)` where `remaining_text` is the
/// response with all `<tool_call>` blocks stripped out.
#[must_use]
pub fn extract_tool_calls(
    response_text: &str,
    valid_tools: &[ToolSpec],
) -> (String, Vec<ExtractedToolCall>) {
    let parsed = crate::core::agent::tool_protocol::parse_fallback_response(response_text);
    (
        parsed.display_text,
        filter_valid_tool_calls(parsed.tool_calls, valid_tools),
    )
}

fn filter_valid_tool_calls(
    tool_calls: Vec<ExtractedToolCall>,
    valid_tools: &[ToolSpec],
) -> Vec<ExtractedToolCall> {
    tool_calls
        .into_iter()
        .filter(|call| valid_tools.iter().any(|tool| tool.name == call.name))
        .collect()
}

/// Parse a free-text provider response for `<tool_call>` blocks and build
/// a structured `ProviderResponse` with `ContentBlock::ToolUse` entries.
///
/// If no tool calls are found, the response is returned as-is with
/// `StopReason::EndTurn`. If tool calls are found, `StopReason::ToolUse`
/// is set and the content blocks are populated.
#[must_use]
pub fn build_fallback_response(
    mut response: ProviderResponse,
    tools: &[ToolSpec],
) -> ProviderResponse {
    let parsed = crate::core::agent::tool_protocol::parse_fallback_response(&response.text);
    let tool_calls = filter_valid_tool_calls(parsed.tool_calls, tools);

    if tool_calls.is_empty() {
        response.text = parsed.display_text;
        response.stop_reason = Some(StopReason::EndTurn);
        return response;
    }

    let mut blocks = Vec::new();
    if !parsed.display_text.trim().is_empty() {
        blocks.push(ContentBlock::Text {
            text: parsed.display_text.clone(),
        });
    }

    for call in tool_calls {
        blocks.push(ContentBlock::ToolUse {
            id: call.id,
            name: call.name,
            input: call.input,
        });
    }

    response.text = parsed.display_text;
    response.content_blocks = blocks;
    response.stop_reason = Some(StopReason::ToolUse);
    response
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ExtractedToolCall, augment_prompt_with_tools, build_fallback_response, extract_tool_calls,
    };
    use crate::core::providers::response::{ContentBlock, ProviderResponse, StopReason};
    use crate::core::tools::traits::ToolSpec;

    fn sample_tools() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "shell".to_string(),
                description: "Execute a shell command.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"]
                }),
                required_capabilities: Vec::new(),
                effect: crate::contracts::tools::ToolEffect::LocalMutation,
            },
            ToolSpec {
                name: "file_read".to_string(),
                description: "Read file contents.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
                required_capabilities: Vec::new(),
                effect: crate::contracts::tools::ToolEffect::LocalMutation,
            },
        ]
    }

    #[test]
    fn extract_tool_calls_parses_valid_tool_call() {
        let tools = sample_tools();
        let text =
            "<tool_call>{\"name\": \"shell\", \"arguments\": {\"command\": \"ls\"}}</tool_call>";

        let (remaining, calls) = extract_tool_calls(text, &tools);

        assert_eq!(remaining, "");
        assert_eq!(calls.len(), 1);
        assert_tool_call(
            &calls[0],
            "fallback_call_1",
            "shell",
            &json!({"command": "ls"}),
        );
    }

    #[test]
    fn extract_tool_calls_discards_unknown_tool_names() {
        let tools = sample_tools();
        let text = "<tool_call>{\"name\": \"fake_tool\", \"arguments\": {}}</tool_call>";

        let (remaining, calls) = extract_tool_calls(text, &tools);

        assert!(calls.is_empty());
        assert_eq!(remaining, "");
    }

    #[test]
    fn build_fallback_response_drops_unknown_tool_names() {
        let tools = sample_tools();
        let response = ProviderResponse::text_only(
            "<tool_call>{\"name\": \"fake_tool\", \"arguments\": {}}</tool_call>".to_string(),
        );

        let built = build_fallback_response(response, &tools);

        assert_eq!(built.stop_reason, Some(StopReason::EndTurn));
        assert!(built.content_blocks.is_empty());
    }

    #[test]
    fn extract_tool_calls_preserves_text_without_tool_blocks() {
        let tools = sample_tools();
        let text = "Just a normal response.";

        let (remaining, calls) = extract_tool_calls(text, &tools);

        assert!(calls.is_empty());
        assert_eq!(remaining, text);
    }

    #[test]
    fn extract_tool_calls_discards_malformed_json_and_keeps_original_text() {
        let tools = sample_tools();
        let text = "<tool_call>{\"name\": \"shell\", \"arguments\": {\"command\": }}</tool_call>";

        let (remaining, calls) = extract_tool_calls(text, &tools);

        assert!(calls.is_empty());
        assert_eq!(
            remaining,
            "{\"name\": \"shell\", \"arguments\": {\"command\": }}"
        );
    }

    #[test]
    fn extract_tool_calls_parses_multiple_tool_calls() {
        let tools = sample_tools();
        let text = concat!(
            "<tool_call>{\"name\": \"shell\", \"arguments\": {\"command\": \"pwd\"}}</tool_call>",
            "\n",
            "<tool_call>{\"name\": \"file_read\", \"arguments\": {\"path\": \"src/lib.rs\"}}</tool_call>"
        );

        let (remaining, calls) = extract_tool_calls(text, &tools);

        assert_eq!(remaining, "");
        assert_eq!(calls.len(), 2);
        assert_tool_call(
            &calls[0],
            "fallback_call_1",
            "shell",
            &json!({"command": "pwd"}),
        );
        assert_tool_call(
            &calls[1],
            "fallback_call_2",
            "file_read",
            &json!({"path": "src/lib.rs"}),
        );
    }

    #[test]
    fn augment_system_prompt_with_tools_includes_names_and_schema() {
        let tools = sample_tools();
        let prompt = augment_prompt_with_tools("System prompt", &tools);

        assert!(prompt.contains("## Available Tools"));
        assert!(prompt.contains("<tool_call>"));
        assert!(prompt.contains("shell: Execute a shell command."));
        assert!(prompt.contains("file_read: Read file contents."));
        assert!(prompt.contains("\"required\":[\"command\"]"));
    }

    #[test]
    fn build_fallback_response_sets_stop_reason_when_tool_calls_exist() {
        let tools = sample_tools();
        let response = ProviderResponse::text_only(
            "<tool_call>{\"name\": \"shell\", \"arguments\": {\"command\": \"ls\"}}</tool_call>"
                .to_string(),
        );

        let built = build_fallback_response(response, &tools);

        assert_eq!(built.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(built.text, "");
        assert_eq!(built.content_blocks.len(), 1);
        assert!(matches!(
            built.content_blocks[0],
            ContentBlock::ToolUse { .. }
        ));
    }

    #[test]
    fn build_fallback_response_keeps_end_turn_when_no_tool_calls() {
        let tools = sample_tools();
        let response = ProviderResponse::text_only("No tools requested".to_string());

        let built = build_fallback_response(response, &tools);

        assert_eq!(built.stop_reason, Some(StopReason::EndTurn));
        assert_eq!(built.text, "No tools requested");
        assert!(built.content_blocks.is_empty());
    }

    #[test]
    fn build_fallback_response_preserves_text_and_extracts_tool_calls() {
        let tools = sample_tools();
        let response = ProviderResponse::text_only(
            concat!(
                "I will inspect files.\n",
                "<tool_call>{\"name\": \"shell\", \"arguments\": {\"command\": \"ls\"}}</tool_call>",
                "\nThen I will continue."
            )
            .to_string(),
        );

        let built = build_fallback_response(response, &tools);

        assert_eq!(built.stop_reason, Some(StopReason::ToolUse));
        assert_eq!(built.text, "I will inspect files.\n\nThen I will continue.");
        assert_eq!(built.content_blocks.len(), 2);
        assert!(matches!(built.content_blocks[0], ContentBlock::Text { .. }));
        assert!(matches!(
            built.content_blocks[1],
            ContentBlock::ToolUse { .. }
        ));
    }

    fn assert_tool_call(call: &ExtractedToolCall, id: &str, name: &str, input: &serde_json::Value) {
        assert_eq!(call.id, id);
        assert_eq!(call.name, name);
        assert_eq!(&call.input, input);
    }
}
