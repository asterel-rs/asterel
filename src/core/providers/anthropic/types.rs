//! Anthropic messages API wire types.

use serde::{Deserialize, Serialize};

/// Anthropic `messages` request payload.
#[derive(Debug, Serialize)]
pub(super) struct ChatRequest {
    pub(super) model: String,
    pub(super) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) system: Option<Vec<SystemBlock>>,
    pub(super) messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<AnthropicToolDef>>,
    pub(super) temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream: Option<bool>,
}

/// Cache control hint for Anthropic prompt caching.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CacheControl {
    pub(super) r#type: &'static str,
}

impl CacheControl {
    pub(super) const fn ephemeral() -> Self {
        Self {
            r#type: "ephemeral",
        }
    }
}

/// System prompt block with optional cache control.
#[derive(Debug, Serialize)]
pub(super) struct SystemBlock {
    pub(super) r#type: &'static str,
    pub(super) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_control: Option<CacheControl>,
}

/// Extended thinking configuration for Anthropic models.
#[derive(Debug, Serialize)]
pub(super) struct ThinkingConfig {
    pub(super) r#type: &'static str,
    pub(super) budget_tokens: u32,
}

/// Input message payload.
#[derive(Debug, Serialize)]
pub(super) struct Message {
    pub(super) role: &'static str,
    pub(super) content: MessageContent,
}

/// Message content representation (text or block list).
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum MessageContent {
    Text(String),
    Blocks(Vec<InputContentBlock>),
}

/// Supported input content blocks.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum InputContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Image {
        source: AnthropicImageSource,
    },
}

/// Image source for Anthropic image blocks.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AnthropicImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// Tool declaration sent to Anthropic.
#[derive(Debug, Serialize)]
pub(super) struct AnthropicToolDef {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_control: Option<CacheControl>,
}

/// Anthropic response payload.
#[derive(Debug, Deserialize)]
pub(super) struct ChatResponse {
    pub(super) content: Vec<ResponseContentBlock>,
    pub(super) stop_reason: Option<String>,
    pub(super) usage: Option<Usage>,
    pub(super) model: Option<String>,
}

/// Token usage metadata.
#[derive(Debug, Deserialize)]
pub(super) struct Usage {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
}

/// Response content block variants.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ResponseContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(other)]
    Unsupported,
}

/// Stream event payload for message start.
#[derive(Debug, Deserialize)]
pub(super) struct StreamMessageStart {
    pub(super) message: StreamMessageInfo,
}

/// Stream-level message metadata.
#[derive(Debug, Deserialize)]
pub(super) struct StreamMessageInfo {
    pub(super) model: Option<String>,
    pub(super) usage: Option<Usage>,
}

/// Stream event payload for content block start.
#[derive(Debug, Deserialize)]
pub(super) struct StreamContentBlockStart {
    pub(super) index: u32,
    pub(super) content_block: StreamContentBlockType,
}

/// Streamed content block type.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum StreamContentBlockType {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
    },
    #[serde(other)]
    Unknown,
}

/// Stream event payload for content block delta.
#[derive(Debug, Deserialize)]
pub(super) struct StreamContentBlockDelta {
    pub(super) index: u32,
    pub(super) delta: StreamDelta,
}

/// Delta variants in streaming responses.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum StreamDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Unknown,
}

/// Stream event payload for message delta.
#[derive(Debug, Deserialize)]
pub(super) struct StreamMessageDelta {
    pub(super) delta: StreamMessageDeltaBody,
    pub(super) usage: Option<StreamDeltaUsage>,
}

/// Message delta body.
#[derive(Debug, Deserialize)]
pub(super) struct StreamMessageDeltaBody {
    pub(super) stop_reason: Option<String>,
}

/// Streaming usage delta.
#[derive(Debug, Deserialize)]
pub(super) struct StreamDeltaUsage {
    pub(super) output_tokens: u64,
}
