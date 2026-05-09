//! `OpenAI` chat-completions wire types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Chat completions request payload.
#[derive(Debug, Serialize)]
pub(in crate::core::providers) struct ChatRequest {
    pub(in crate::core::providers) model: String,
    pub(in crate::core::providers) messages: Vec<Message>,
    pub(in crate::core::providers) temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) reasoning_effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) stream_options: Option<StreamOptions>,
}

/// Streaming options for chat completions.
#[derive(Debug, Serialize)]
pub(in crate::core::providers) struct StreamOptions {
    pub(in crate::core::providers) include_usage: bool,
}

/// Single message in `OpenAI` request/response payloads.
#[derive(Debug, Serialize)]
pub(in crate::core::providers) struct Message {
    pub(in crate::core::providers) role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::core::providers) tool_calls: Option<Vec<OpenAiToolCall>>,
}

/// Message content in either simple text or multipart form.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(in crate::core::providers) enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// Multipart content item used by multimodal requests.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(in crate::core::providers) enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlContent },
}

/// Image URL wrapper used in content parts.
#[derive(Debug, Serialize)]
pub(in crate::core::providers) struct ImageUrlContent {
    pub(in crate::core::providers) url: String,
}

/// Tool declaration for `OpenAI` function-calling.
#[derive(Debug, Clone, Serialize)]
pub(in crate::core::providers) struct OpenAiTool {
    pub(in crate::core::providers) r#type: &'static str,
    pub(in crate::core::providers) function: OpenAiToolDefinition,
}

/// `OpenAI` function definition schema.
#[derive(Debug, Clone, Serialize)]
pub(in crate::core::providers) struct OpenAiToolDefinition {
    pub(in crate::core::providers) name: String,
    pub(in crate::core::providers) description: String,
    pub(in crate::core::providers) parameters: Value,
}

/// Tool call emitted by `OpenAI`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::core::providers) struct OpenAiToolCall {
    pub(in crate::core::providers) id: String,
    pub(in crate::core::providers) r#type: String,
    pub(in crate::core::providers) function: OpenAiToolCallFunction,
}

/// Function section of an `OpenAI` tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::core::providers) struct OpenAiToolCallFunction {
    pub(in crate::core::providers) name: String,
    pub(in crate::core::providers) arguments: String,
}

/// Chat completions response payload.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ChatResponse {
    pub(in crate::core::providers) choices: Vec<Choice>,
    pub(in crate::core::providers) usage: Option<Usage>,
    pub(in crate::core::providers) model: Option<String>,
}

/// Token usage metadata.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct Usage {
    pub(in crate::core::providers) prompt_tokens: u64,
    pub(in crate::core::providers) completion_tokens: u64,
}

/// Top-level completion choice.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct Choice {
    pub(in crate::core::providers) message: ResponseMessage,
    pub(in crate::core::providers) finish_reason: Option<String>,
}

/// Assistant message payload returned by `OpenAI`.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ResponseMessage {
    pub(in crate::core::providers) content: Option<String>,
    pub(in crate::core::providers) tool_calls: Option<Vec<OpenAiToolCall>>,
}

/// Streaming chunk payload.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ChatCompletionChunk {
    pub(in crate::core::providers) model: Option<String>,
    pub(in crate::core::providers) choices: Vec<ChunkChoice>,
    pub(in crate::core::providers) usage: Option<Usage>,
}

/// Single choice in a stream chunk.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ChunkChoice {
    pub(in crate::core::providers) delta: ChunkDelta,
    pub(in crate::core::providers) finish_reason: Option<String>,
}

/// Delta payload in a stream chunk.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ChunkDelta {
    pub(in crate::core::providers) content: Option<String>,
    pub(in crate::core::providers) tool_calls: Option<Vec<ChunkToolCall>>,
}

/// Tool call delta entry in streamed output.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ChunkToolCall {
    pub(in crate::core::providers) index: u32,
    pub(in crate::core::providers) id: Option<String>,
    pub(in crate::core::providers) function: Option<ChunkToolCallFunction>,
}

/// Function delta payload for streamed tool calls.
#[derive(Debug, Deserialize)]
pub(in crate::core::providers) struct ChunkToolCallFunction {
    pub(in crate::core::providers) name: Option<String>,
    pub(in crate::core::providers) arguments: Option<String>,
}

/// `OpenAI` reasoning effort selector.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::core::providers) enum ReasoningEffort {
    Low,
    Medium,
    High,
}
