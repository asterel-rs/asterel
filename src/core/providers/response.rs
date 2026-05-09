//! Canonical response-side content model shared by provider adapters.

use serde::{Deserialize, Serialize};

/// Image payload source in a provider message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    /// Inline base64-encoded image bytes.
    Base64 { media_type: String, data: String },
    /// Remote image URL.
    Url { url: String },
}

/// Structured content block used in provider messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text content.
    Text { text: String },
    /// Model-initiated tool invocation request.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Tool output mapped back into the conversation.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// Image content block.
    Image { source: ImageSource },
}

/// Role associated with a provider message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// Human/user authored message.
    User,
    /// Assistant/model authored message.
    Assistant,
    /// System instruction message.
    System,
}

/// Normalized message passed to provider adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMessage {
    /// Role of the message author.
    pub role: MessageRole,
    /// Ordered content blocks for this message.
    pub content: Vec<ContentBlock>,
}

/// Per-token log-probability with the top alternative.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenLogprob {
    /// The chosen token text.
    pub token: String,
    /// Log-probability of the chosen token.
    pub logprob: f64,
    /// Log-probability of the best alternative token, if available.
    pub top_alt_logprob: Option<f64>,
}

/// Unified stop reason returned by provider adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Completed normally.
    EndTurn,
    /// Stopped to request tool execution.
    ToolUse,
    /// Stopped due to token limit.
    MaxTokens,
    /// Stopped due to provider/runtime error.
    Error,
}

/// Canonical provider response used by the tool loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    /// Best-effort plain text content extracted from the response.
    pub text: String,
    /// Prompt/input token usage, if reported.
    pub input_tokens: Option<u64>,
    /// Completion/output token usage, if reported.
    pub output_tokens: Option<u64>,
    /// Provider-reported model identifier.
    pub model: Option<String>,
    /// Structured content blocks, including tool-use directives.
    pub content_blocks: Vec<ContentBlock>,
    /// Reason the provider ended generation, if available.
    pub stop_reason: Option<StopReason>,
    /// Optional per-token log-probabilities for uncertainty estimation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Vec<TokenLogprob>>,
}

impl ProviderResponse {
    /// Construct a text-only response without usage metadata.
    #[must_use]
    pub fn text_only(text: String) -> Self {
        Self {
            text,
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![],
            stop_reason: None,
            logprobs: None,
        }
    }

    /// Construct a text response with explicit token usage.
    #[must_use]
    pub fn with_usage(text: String, input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            text,
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            model: None,
            content_blocks: vec![],
            stop_reason: None,
            logprobs: None,
        }
    }

    /// Attach the provider-reported model identifier.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Return `input_tokens + output_tokens` when both are present.
    #[must_use]
    pub fn total_tokens(&self) -> Option<u64> {
        match (self.input_tokens, self.output_tokens) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        }
    }

    /// Collect only `ToolUse` blocks from the response.
    #[must_use]
    pub fn tool_use_blocks(&self) -> Vec<&ContentBlock> {
        self.iter_tool_use_blocks().collect()
    }

    /// Iterate over only `ToolUse` blocks from the response.
    pub fn iter_tool_use_blocks(&self) -> impl Iterator<Item = &ContentBlock> {
        self.content_blocks
            .iter()
            .filter(|block| matches!(block, ContentBlock::ToolUse { .. }))
    }

    /// Return whether at least one `ToolUse` block exists.
    #[must_use]
    pub fn has_tool_use(&self) -> bool {
        self.content_blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
    }

    /// Convert this response into an assistant message for follow-up turns.
    #[must_use]
    pub fn to_assistant_message(&self) -> ProviderMessage {
        if self.content_blocks.is_empty() {
            ProviderMessage {
                role: MessageRole::Assistant,
                content: vec![ContentBlock::Text {
                    text: self.text.clone(),
                }],
            }
        } else {
            ProviderMessage {
                role: MessageRole::Assistant,
                content: self.content_blocks.clone(),
            }
        }
    }

    /// Convert this response into an assistant message, consuming the
    /// response so existing content blocks can be reused without cloning.
    #[must_use]
    pub fn into_assistant_message(self) -> ProviderMessage {
        if self.content_blocks.is_empty() {
            ProviderMessage {
                role: MessageRole::Assistant,
                content: vec![ContentBlock::Text { text: self.text }],
            }
        } else {
            ProviderMessage {
                role: MessageRole::Assistant,
                content: self.content_blocks,
            }
        }
    }
}

impl ProviderMessage {
    /// Construct a user message with a single text block.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Construct an assistant message with a single text block.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Construct a system message with a single text block.
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Construct a user message carrying tool execution output.
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error,
            }],
        }
    }

    /// Construct a user message containing text and one image block.
    pub fn user_with_image(text: impl Into<String>, source: ImageSource) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![
                ContentBlock::Text { text: text.into() },
                ContentBlock::Image { source },
            ],
        }
    }

    /// Construct a user message containing only one image block.
    #[must_use]
    pub fn user_image(source: ImageSource) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![ContentBlock::Image { source }],
        }
    }
}

impl ImageSource {
    /// Construct an inline base64 image source.
    pub fn base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Base64 {
            media_type: media_type.into(),
            data: data.into(),
        }
    }

    /// Construct an image source referencing a URL.
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url { url: url.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContentBlock, ImageSource, MessageRole, ProviderMessage, ProviderResponse, StopReason,
        TokenLogprob,
    };

    #[test]
    fn content_block_serde_round_trip() {
        let value = serde_json::json!({
            "type": "tool_use",
            "id": "toolu_123",
            "name": "search",
            "input": {"query": "rust"}
        });

        let block: ContentBlock = serde_json::from_value(value.clone()).unwrap();
        let serialized = serde_json::to_value(&block).unwrap();

        assert_eq!(serialized, value);
    }

    #[test]
    fn provider_message_user_constructor() {
        let message = ProviderMessage::user("hello");

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected text content block"),
        }
    }

    #[test]
    fn provider_message_assistant_constructor() {
        let message = ProviderMessage::assistant("hello");

        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected text content block"),
        }
    }

    #[test]
    fn provider_message_tool_result_constructor() {
        let message = ProviderMessage::tool_result("toolu_123", "ok", false);

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_123");
                assert_eq!(content, "ok");
                assert!(!is_error);
            }
            _ => panic!("expected tool_result content block"),
        }
    }

    #[test]
    fn image_source_base64_constructor() {
        let source = ImageSource::base64("image/png", "iVBOR...");
        match &source {
            ImageSource::Base64 { media_type, data } => {
                assert_eq!(media_type, "image/png");
                assert_eq!(data, "iVBOR...");
            }
            ImageSource::Url { .. } => panic!("expected Base64 variant"),
        }
    }

    #[test]
    fn image_source_url_constructor() {
        let source = ImageSource::url("https://example.com/img.png");
        match &source {
            ImageSource::Url { url } => assert_eq!(url, "https://example.com/img.png"),
            ImageSource::Base64 { .. } => panic!("expected Url variant"),
        }
    }

    #[test]
    fn image_source_serde_roundtrip_base64() {
        let source = ImageSource::base64("image/jpeg", "abc123");
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["type"], "base64");
        assert_eq!(json["media_type"], "image/jpeg");
        assert_eq!(json["data"], "abc123");
        let decoded: ImageSource = serde_json::from_value(json).unwrap();
        match decoded {
            ImageSource::Base64 { media_type, data } => {
                assert_eq!(media_type, "image/jpeg");
                assert_eq!(data, "abc123");
            }
            ImageSource::Url { .. } => panic!("expected Base64"),
        }
    }

    #[test]
    fn image_source_serde_roundtrip_url() {
        let source = ImageSource::url("https://example.com/img.png");
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["type"], "url");
        assert_eq!(json["url"], "https://example.com/img.png");
        let decoded: ImageSource = serde_json::from_value(json).unwrap();
        match decoded {
            ImageSource::Url { url } => assert_eq!(url, "https://example.com/img.png"),
            ImageSource::Base64 { .. } => panic!("expected Url"),
        }
    }

    #[test]
    fn content_block_image_serde_roundtrip() {
        let block = ContentBlock::Image {
            source: ImageSource::base64("image/png", "data123"),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        let decoded: ContentBlock = serde_json::from_value(json).unwrap();
        assert!(matches!(decoded, ContentBlock::Image { .. }));
    }

    #[test]
    fn provider_message_user_with_image_constructor() {
        let msg = ProviderMessage::user_with_image(
            "What's in this image?",
            ImageSource::base64("image/png", "data"),
        );
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content.len(), 2);
        assert!(
            matches!(&msg.content[0], ContentBlock::Text { text } if text == "What's in this image?")
        );
        assert!(matches!(&msg.content[1], ContentBlock::Image { .. }));
    }

    #[test]
    fn provider_message_user_image_constructor() {
        let msg = ProviderMessage::user_image(ImageSource::url("https://example.com/img.png"));
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content.len(), 1);
        assert!(matches!(&msg.content[0], ContentBlock::Image { .. }));
    }

    #[test]
    fn provider_response_tool_use_blocks_filters_correctly() {
        let response = ProviderResponse {
            text: "done".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![
                ContentBlock::Text {
                    text: "hi".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "search".to_string(),
                    input: serde_json::json!({"q": "rust"}),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "toolu_1".to_string(),
                    content: "result".to_string(),
                    is_error: false,
                },
                ContentBlock::ToolUse {
                    id: "toolu_2".to_string(),
                    name: "read_file".to_string(),
                    input: serde_json::json!({"path": "src/lib.rs"}),
                },
            ],
            stop_reason: Some(StopReason::ToolUse),
            logprobs: None,
        };

        let blocks = response.iter_tool_use_blocks().collect::<Vec<_>>();

        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], ContentBlock::ToolUse { .. }));
        assert!(matches!(blocks[1], ContentBlock::ToolUse { .. }));
    }

    #[test]
    fn provider_response_has_tool_use_works() {
        let with_tool_use = ProviderResponse {
            text: "done".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "search".to_string(),
                input: serde_json::json!({"q": "rust"}),
            }],
            stop_reason: Some(StopReason::ToolUse),
            logprobs: None,
        };
        let without_tool_use = ProviderResponse {
            text: "done".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            logprobs: None,
        };

        assert!(with_tool_use.has_tool_use());
        assert!(!without_tool_use.has_tool_use());
    }

    #[test]
    fn provider_response_to_assistant_message_empty_content_blocks() {
        let response = ProviderResponse::text_only("plain text".to_string());

        let message = response.to_assistant_message();

        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "plain text"),
            _ => panic!("expected text content block"),
        }
    }

    #[test]
    fn provider_response_to_assistant_message_non_empty_content_blocks() {
        let response = ProviderResponse {
            text: "fallback".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "search".to_string(),
                input: serde_json::json!({"q": "rust"}),
            }],
            stop_reason: Some(StopReason::ToolUse),
            logprobs: None,
        };

        let message = response.to_assistant_message();

        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.content.len(), 1);
        assert!(matches!(message.content[0], ContentBlock::ToolUse { .. }));
    }

    #[test]
    fn provider_response_into_assistant_message_moves_text_only_content() {
        let response = ProviderResponse::text_only("plain text".to_string());

        let message = response.into_assistant_message();

        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.content.len(), 1);
        match &message.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "plain text"),
            _ => panic!("expected text content block"),
        }
    }

    #[test]
    fn provider_response_into_assistant_message_moves_existing_blocks() {
        let response = ProviderResponse {
            text: "fallback".to_string(),
            input_tokens: None,
            output_tokens: None,
            model: None,
            content_blocks: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "search".to_string(),
                input: serde_json::json!({"q": "rust"}),
            }],
            stop_reason: Some(StopReason::ToolUse),
            logprobs: None,
        };

        let message = response.into_assistant_message();

        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.content.len(), 1);
        assert!(matches!(message.content[0], ContentBlock::ToolUse { .. }));
    }

    #[test]
    fn stop_reason_serde_round_trip() {
        let reason = StopReason::MaxTokens;

        let value = serde_json::to_value(reason).unwrap();
        assert_eq!(value, serde_json::json!("max_tokens"));

        let decoded: StopReason = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, StopReason::MaxTokens);
    }

    #[test]
    fn text_only_and_with_usage_still_work() {
        let text_only = ProviderResponse::text_only("hello".to_string());
        assert_eq!(text_only.text, "hello");
        assert_eq!(text_only.input_tokens, None);
        assert_eq!(text_only.output_tokens, None);
        assert_eq!(text_only.model, None);
        assert!(text_only.content_blocks.is_empty());
        assert_eq!(text_only.stop_reason, None);
        assert_eq!(text_only.logprobs, None);
        assert_eq!(text_only.total_tokens(), None);

        let with_usage = ProviderResponse::with_usage("hello".to_string(), 10, 20);
        assert_eq!(with_usage.text, "hello");
        assert_eq!(with_usage.input_tokens, Some(10));
        assert_eq!(with_usage.output_tokens, Some(20));
        assert_eq!(with_usage.model, None);
        assert!(with_usage.content_blocks.is_empty());
        assert_eq!(with_usage.stop_reason, None);
        assert_eq!(with_usage.logprobs, None);
        assert_eq!(with_usage.total_tokens(), Some(30));
    }

    #[test]
    fn token_logprob_serde_roundtrip() {
        let token = TokenLogprob {
            token: "hello".to_string(),
            logprob: -0.5,
            top_alt_logprob: Some(-2.3),
        };
        let json = serde_json::to_value(&token).unwrap();
        assert_eq!(json["token"], "hello");
        let decoded: TokenLogprob = serde_json::from_value(json).unwrap();
        assert!((decoded.logprob - (-0.5)).abs() < f64::EPSILON);
        assert!((decoded.top_alt_logprob.unwrap() - (-2.3)).abs() < f64::EPSILON);
    }

    #[test]
    fn logprobs_field_skipped_when_none_in_serde() {
        let response = ProviderResponse::text_only("hi".to_string());
        let json = serde_json::to_value(&response).unwrap();
        assert!(!json.as_object().unwrap().contains_key("logprobs"));
    }
}
