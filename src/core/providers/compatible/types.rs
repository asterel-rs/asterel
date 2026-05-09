//! Request/response types for the OpenAI-compatible provider.
//!
//! Covers both the Chat Completions and Responses API wire formats.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::providers::response::{ContentBlock, MessageRole, ProviderMessage};
use crate::core::tools::traits::ToolSpec;
use crate::security::scrub::scrub_secrets;

/// Chat Completions API request body.
#[derive(Debug, Serialize)]
pub(super) struct ChatRequest {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) temperature: f64,
}

/// Single message in a Chat Completions request.
#[derive(Debug, Serialize)]
pub(super) struct Message {
    pub(super) role: &'static str,
    pub(super) content: String,
}

/// Chat Completions API response body.
#[derive(Debug, Deserialize)]
pub(super) struct ChatResponse {
    pub(super) choices: Vec<Choice>,
    pub(super) usage: Option<ChatUsage>,
    pub(super) model: Option<String>,
}

/// Token usage counters from a Chat Completions response.
#[derive(Debug, Deserialize)]
pub(super) struct ChatUsage {
    pub(super) prompt_tokens: u64,
    pub(super) completion_tokens: u64,
}

/// Single completion choice in a Chat Completions response.
#[derive(Debug, Deserialize)]
pub(super) struct Choice {
    pub(super) message: ResponseMessage,
}

/// Assistant message content within a Chat Completions choice.
#[derive(Debug, Deserialize)]
pub(super) struct ResponseMessage {
    pub(super) content: String,
}

// ── Responses API tool definition (flat format) ─────────────────────

/// Tool definition in the Responses API flat format.
#[derive(Debug, Serialize)]
pub(super) struct ResponsesTool {
    pub(super) r#type: &'static str,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) parameters: Value,
}

// ── Responses API request ───────────────────────────────────────────

/// Responses API request body.
#[derive(Debug, Serialize)]
pub(super) struct ResponsesRequest {
    pub(super) model: String,
    pub(super) input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream: Option<bool>,
}

/// Input item for the Responses API (union of message, `function_call`,
/// and `function_call_output`).
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum ResponsesInputItem {
    Message {
        role: &'static str,
        content: String,
    },
    FunctionCall {
        r#type: &'static str,
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        r#type: &'static str,
        call_id: String,
        output: String,
    },
}

// ── Responses API response ──────────────────────────────────────────

/// Responses API response body.
#[derive(Debug, Deserialize)]
pub(super) struct ResponsesResponse {
    #[serde(default)]
    pub(super) output: Vec<ResponsesOutput>,
    #[serde(default)]
    pub(super) output_text: Option<String>,
    #[serde(default)]
    pub(super) usage: Option<ResponsesUsage>,
    #[serde(default)]
    pub(super) status: Option<String>,
}

/// Token usage counters from a Responses API response.
#[derive(Debug, Deserialize)]
pub(super) struct ResponsesUsage {
    pub(super) input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
}

/// Single output item from a Responses API response.
#[derive(Debug, Deserialize)]
pub(super) struct ResponsesOutput {
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) content: Vec<ResponsesContent>,
    // function_call fields
    pub(super) id: Option<String>,
    pub(super) call_id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
}

/// Content block within a Responses API output item.
#[derive(Debug, Deserialize)]
pub(super) struct ResponsesContent {
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    pub(super) text: Option<String>,
}

fn first_nonempty(text: Option<&str>) -> Option<String> {
    text.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Extract the first non-empty text from a Responses API response.
pub(super) fn extract_responses_text(response: &ResponsesResponse) -> Option<String> {
    if response.status.as_deref() == Some("failed") {
        tracing::debug!("responses API returned failed status without output text");
    }

    if let Some(text) = first_nonempty(response.output_text.as_deref()) {
        return Some(text);
    }

    for item in &response.output {
        for content in &item.content {
            if content.kind.as_deref() == Some("output_text")
                && let Some(text) = first_nonempty(content.text.as_deref())
            {
                return Some(text);
            }
        }
    }

    for item in &response.output {
        for content in &item.content {
            if let Some(text) = first_nonempty(content.text.as_deref()) {
                return Some(text);
            }
        }
    }

    None
}

/// Extract text from a Responses API SSE stream body.
pub(super) fn extract_responses_sse_text(body: &str) -> Option<String> {
    let mut output_text = String::new();
    let mut snapshot: Option<String> = None;

    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = data.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            continue;
        };

        if let Some(text) = value
            .pointer("/response/output_text")
            .and_then(Value::as_str)
            .and_then(|v| first_nonempty(Some(v)))
        {
            snapshot = Some(text);
        }

        if let Some(text) = value
            .pointer("/output_text")
            .and_then(Value::as_str)
            .and_then(|v| first_nonempty(Some(v)))
        {
            snapshot = Some(text);
        }

        if let Some(delta) = value.pointer("/delta").and_then(Value::as_str) {
            output_text.push_str(delta);
        }
    }

    if !output_text.trim().is_empty() {
        return Some(output_text.trim().to_string());
    }

    snapshot
}

/// Extract the assistant text from the first Chat Completions choice.
///
/// # Errors
///
/// Returns `ProviderError::EmptyResponse` if no choices exist.
pub(super) fn extract_chat_text(
    response: &ChatResponse,
    provider_name: &str,
) -> anyhow::Result<String> {
    response
        .choices
        .first()
        .map(|choice| choice.message.content.clone())
        .ok_or_else(|| {
            anyhow::Error::from(crate::core::providers::ProviderError::EmptyResponse {
                provider: provider_name.to_string(),
            })
        })
}

// ── Responses API tool calling helpers ──────────────────────────────

/// Convert `ToolSpec` slice to Responses API flat tool definitions.
pub(super) fn build_responses_tools(tools: &[ToolSpec]) -> Option<Vec<ResponsesTool>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|t| ResponsesTool {
                r#type: "function",
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect(),
    )
}

/// Extract `function_call` items from a Responses API response.
/// Returns `(call_id, name, arguments_json)` tuples.
pub(super) fn extract_responses_tool_calls(
    response: &ResponsesResponse,
) -> Vec<(String, String, String)> {
    response
        .output
        .iter()
        .filter(|item| item.kind.as_deref() == Some("function_call"))
        .filter_map(|item| {
            let call_id = item.call_id.as_ref().or(item.id.as_ref())?;
            let name = item.name.as_ref()?;
            let arguments = item.arguments.as_ref()?;
            Some((call_id.clone(), name.clone(), arguments.clone()))
        })
        .collect()
}

/// Convert `ProviderMessage` history to Responses API input items.
pub(super) fn build_responses_input_from_messages(
    messages: &[ProviderMessage],
) -> Vec<ResponsesInputItem> {
    let mut items = Vec::new();

    for msg in messages {
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => {
                    let role = match msg.role {
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                        MessageRole::System => "system",
                    };
                    items.push(ResponsesInputItem::Message {
                        role,
                        content: scrub_secrets(text).into_owned(),
                    });
                }
                ContentBlock::ToolUse { id, name, input } => {
                    items.push(ResponsesInputItem::FunctionCall {
                        r#type: "function_call",
                        id: id.clone(),
                        call_id: id.clone(),
                        name: name.clone(),
                        arguments: input.to_string(),
                    });
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    items.push(ResponsesInputItem::FunctionCallOutput {
                        r#type: "function_call_output",
                        call_id: tool_use_id.clone(),
                        output: scrub_secrets(content).into_owned(),
                    });
                }
                ContentBlock::Image { .. } => {}
            }
        }
    }

    items
}
