//! Anthropic Claude provider implementation.
//!
//! Handles the Messages API with streaming SSE, extended thinking,
//! and tool-use support.

use std::future::Future;
use std::pin::Pin;

use num_traits::ToPrimitive;
use reqwest::Client;

use crate::core::providers::sse::{SseBuffer, parse_event_pairs};
use crate::core::providers::streaming::ProviderStream;
use crate::core::providers::tool_convert::{ToolFields, map_tools_optional};
use crate::core::providers::traits::Provider;
use crate::core::providers::{
    ContentBlock, ImageSource, InferenceOpts, MessageRole, ProviderMessage, ProviderResponse,
    ProviderResult, StopReason, build_provider_http_client, scrub_secrets,
};
use crate::core::tools::traits::ToolSpec;

mod types;
use types::{
    AnthropicImageSource, AnthropicToolDef, CacheControl, ChatRequest, ChatResponse,
    InputContentBlock, Message, MessageContent, ResponseContentBlock, StreamContentBlockDelta,
    StreamContentBlockStart, StreamContentBlockType, StreamDelta, StreamMessageDelta,
    StreamMessageStart, SystemBlock, ThinkingConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnthropicAuthMode {
    Auto,
    Bearer,
}

/// Anthropic Claude provider using the Messages API.
pub struct AnthropicProvider {
    /// Pre-computed auth: `("Authorization", "Bearer <token>")` or `("x-api-key", "<key>")`.
    cached_auth: Option<(&'static str, String)>,
    cached_messages_url: String,
    client: Client,
}

impl AnthropicProvider {
    /// Create a provider targeting the default Anthropic API endpoint.
    #[must_use]
    pub fn new(api_key: Option<&str>) -> Self {
        Self::with_base_url(api_key, None)
    }

    /// Create a provider with a custom base URL (for proxies or tests).
    #[must_use]
    pub fn with_base_url(api_key: Option<&str>, base_url: Option<&str>) -> Self {
        Self::with_base_url_and_auth_mode(api_key, base_url, AnthropicAuthMode::Auto)
    }

    pub(crate) fn with_base_url_and_auth_mode(
        api_key: Option<&str>,
        base_url: Option<&str>,
        auth_mode: AnthropicAuthMode,
    ) -> Self {
        let base = base_url
            .map_or("https://api.anthropic.com", |u| u.trim_end_matches('/'))
            .to_string();
        let cached_messages_url = format!("{base}/v1/messages");
        let cached_auth = api_key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(|token| Self::auth_header_for_token_with_mode(token, auth_mode));
        Self {
            cached_auth,
            cached_messages_url,
            client: build_provider_http_client(),
        }
    }

    /// Return `true` for `Anthropic` OAuth setup-tokens (`sk-ant-oat01-`).
    /// Setup-tokens use `Authorization: Bearer` instead of `x-api-key`.
    fn is_setup_token(token: &str) -> bool {
        token.starts_with("sk-ant-oat01-")
    }

    pub(crate) fn auth_header_for_token(token: &str) -> (&'static str, String) {
        Self::auth_header_for_token_with_mode(token, AnthropicAuthMode::Auto)
    }

    pub(crate) fn auth_header_for_token_with_mode(
        token: &str,
        auth_mode: AnthropicAuthMode,
    ) -> (&'static str, String) {
        if matches!(auth_mode, AnthropicAuthMode::Bearer) {
            return ("Authorization", format!("Bearer {token}"));
        }
        if Self::is_setup_token(token) {
            ("Authorization", format!("Bearer {token}"))
        } else {
            ("x-api-key", token.to_string())
        }
    }

    /// Compute `max_tokens` for a request. Scales the base (4096) by the
    /// `max_tokens_factor` from `InferenceOpts` when provided.
    fn effective_max_tokens(inference_options: Option<&InferenceOpts>) -> u32 {
        const BASE_MAX_TOKENS: u32 = 4096;
        inference_options
            .and_then(|opts| opts.max_tokens_factor)
            .map_or(BASE_MAX_TOKENS, |factor| {
                scaled_max_tokens(BASE_MAX_TOKENS, factor)
            })
    }

    fn build_request(
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            max_tokens: Self::effective_max_tokens(inference_options),
            system: system_prompt.map(Self::build_system_blocks),
            messages: vec![Message {
                role: "user",
                content: MessageContent::Text(message.to_string()),
            }],
            tools: None,
            temperature,
            top_p: inference_options.and_then(|opts| opts.top_p),
            thinking: Self::map_thinking_config(inference_options),
            stream: None,
        }
    }

    /// Split a system prompt into cacheable (static) and non-cacheable
    /// (dynamic) blocks to maximize prompt cache hits.
    ///
    /// Strategy: the base identity prompt (before the first `\n\n---\n\n`
    /// or `## Tool Result Trust Policy` boundary) is stable across turns
    /// and gets `cache_control: ephemeral`. Everything after is dynamic
    /// (augmentation addendum) and is sent without cache control so it
    /// doesn't invalidate the cached prefix.
    fn build_system_blocks(text: &str) -> Vec<SystemBlock> {
        // Look for a natural split point between static and dynamic content.
        let split_markers = ["\n\n## Tool Result Trust Policy", "\n\n## Self-Contract"];

        for marker in split_markers {
            if let Some(pos) = text.find(marker) {
                let static_part = text[..pos].trim();
                let dynamic_part = text[pos..].trim();
                if !static_part.is_empty() && !dynamic_part.is_empty() {
                    return vec![
                        SystemBlock {
                            r#type: "text",
                            text: static_part.to_string(),
                            cache_control: Some(CacheControl::ephemeral()),
                        },
                        SystemBlock {
                            r#type: "text",
                            text: dynamic_part.to_string(),
                            cache_control: None,
                        },
                    ];
                }
            }
        }

        // No split point found — cache the entire prompt.
        vec![SystemBlock {
            r#type: "text",
            text: text.to_string(),
            cache_control: Some(CacheControl::ephemeral()),
        }]
    }

    /// Map `InferenceOpts::thinking_level` to an `Anthropic` extended thinking
    /// config with a token budget. Returns `None` when thinking is disabled
    /// (budget is zero or thinking level maps to no budget).
    fn map_thinking_config(inference_options: Option<&InferenceOpts>) -> Option<ThinkingConfig> {
        let budget_tokens = inference_options.and_then(|options| {
            super::inference::anthropic_budget_tokens(options.thinking_level)
        })?;
        Some(ThinkingConfig {
            r#type: "enabled",
            budget_tokens,
        })
    }

    /// Mark the last tool definition with `cache_control: ephemeral` so
    /// `Anthropic`'s prompt cache covers the complete tool list prefix.
    fn set_cache_control_on_last_tool(tools: &mut [AnthropicToolDef]) {
        if let Some(last) = tools.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }
    }

    /// Convert a canonical `ProviderMessage` into an `Anthropic` Messages API
    /// `Message`, mapping content blocks and scrubbing secrets from text.
    fn provider_message_to_message(provider_message: &ProviderMessage) -> Message {
        let role = match provider_message.role {
            MessageRole::User | MessageRole::System => "user",
            MessageRole::Assistant => "assistant",
        };

        if let [ContentBlock::Text { text }] = provider_message.content.as_slice() {
            return Message {
                role,
                content: MessageContent::Text(scrub_secrets(text).into_owned()),
            };
        }

        let blocks = provider_message
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => InputContentBlock::Text {
                    text: scrub_secrets(text).into_owned(),
                },
                ContentBlock::ToolUse { id, name, input } => InputContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                },
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => InputContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: scrub_secrets(content).into_owned(),
                    is_error: if *is_error { Some(true) } else { None },
                },
                ContentBlock::Image { source } => {
                    let anthropic_source = match source {
                        ImageSource::Base64 { media_type, data } => AnthropicImageSource::Base64 {
                            media_type: media_type.clone(),
                            data: data.clone(),
                        },
                        ImageSource::Url { url } => AnthropicImageSource::Url { url: url.clone() },
                    };
                    InputContentBlock::Image {
                        source: anthropic_source,
                    }
                }
            })
            .collect();

        Message {
            role,
            content: MessageContent::Blocks(blocks),
        }
    }

    /// Map an `Anthropic` stop-reason string to the canonical `StopReason` enum.
    fn map_stop_reason(stop_reason: Option<&str>) -> Option<StopReason> {
        stop_reason.map(|reason| match reason {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            _ => StopReason::Error,
        })
    }

    /// Convert `Anthropic` response content blocks to canonical `ContentBlock`s,
    /// dropping unsupported block types silently.
    fn parse_content_blocks(blocks: &[ResponseContentBlock]) -> Vec<ContentBlock> {
        blocks
            .iter()
            .filter_map(|block| match block {
                ResponseContentBlock::Text { text } => {
                    Some(ContentBlock::Text { text: text.clone() })
                }
                ResponseContentBlock::ToolUse { id, name, input } => Some(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }),
                ResponseContentBlock::Unsupported => None,
            })
            .collect()
    }

    /// Concatenate text blocks into a single string, separated by newlines.
    /// Returns `None` if no text blocks are present.
    fn text_from_content_blocks(blocks: &[ContentBlock]) -> Option<String> {
        let mut text = String::new();
        for block in blocks {
            if let ContentBlock::Text { text: t } = block {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }

        if text.is_empty() { None } else { Some(text) }
    }

    /// Extract the text content from a non-streaming `Anthropic` response,
    /// returning `EmptyResponse` if no text blocks are present.
    fn extract_text(chat_response: &ChatResponse) -> anyhow::Result<String> {
        Self::text_from_content_blocks(&Self::parse_content_blocks(&chat_response.content))
            .ok_or_else(|| {
                anyhow::Error::from(super::ProviderError::EmptyResponse {
                    provider: "Anthropic".into(),
                })
            })
    }

    /// Build and send a non-streaming `Anthropic` Messages API request.
    async fn call_api(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ChatResponse> {
        let request = Self::build_request(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        );
        self.call_api_req(&request).await
    }

    /// Send a pre-built `ChatRequest` and deserialize the JSON response.
    /// Returns a structured API error for non-2xx status codes.
    async fn call_api_req(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        let (auth_name, auth_value) =
            self.cached_auth
                .as_ref()
                .ok_or_else(|| super::ProviderError::MissingCredentials {
                    provider: "Anthropic".into(),
                    message: "Set ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN (setup-token).".into(),
                })?;

        let response = self
            .client
            .post(&self.cached_messages_url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header(*auth_name, auth_value)
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::api_error("Anthropic", response).await);
        }

        response.json().await.map_err(anyhow::Error::msg)
    }

    /// Send a pre-built `ChatRequest` with `stream: true` and return the raw
    /// HTTP response for SSE processing. Returns a structured API error for
    /// non-2xx status codes.
    async fn call_api_streaming(&self, request: &ChatRequest) -> anyhow::Result<reqwest::Response> {
        let (auth_name, auth_value) =
            self.cached_auth
                .as_ref()
                .ok_or_else(|| super::ProviderError::MissingCredentials {
                    provider: "Anthropic".into(),
                    message: "Set ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN (setup-token).".into(),
                })?;

        let response = self
            .client
            .post(&self.cached_messages_url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header(*auth_name, auth_value)
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::api_error("Anthropic", response).await);
        }

        Ok(response)
    }

    /// Translate a single SSE `(event_type, data)` pair from the `Anthropic`
    /// streaming protocol into zero or more canonical `StreamEvent`s.
    ///
    /// Handles: `message_start` (usage + model), `content_block_start`
    /// (tool call or text open), `content_block_delta` (text/JSON fragments),
    /// `message_delta` (stop reason + output token count).
    fn stream_events_from_sse(
        event_type: &str,
        data: &str,
        input_tokens: &mut Option<u64>,
        output_tokens: &mut Option<u64>,
    ) -> Vec<crate::core::providers::streaming::StreamEvent> {
        use crate::core::providers::streaming::StreamEvent;

        let mut events = Vec::new();
        match event_type {
            "message_start" => {
                if let Ok(msg) = serde_json::from_str::<StreamMessageStart>(data) {
                    if let Some(usage) = msg.message.usage {
                        *input_tokens = Some(usage.input_tokens);
                    }
                    events.push(StreamEvent::ResponseStart {
                        model: msg.message.model,
                    });
                }
            }
            "content_block_start" => {
                if let Ok(block) = serde_json::from_str::<StreamContentBlockStart>(data) {
                    match block.content_block {
                        StreamContentBlockType::ToolUse { id, name } => {
                            events.push(StreamEvent::ToolCallDelta {
                                index: block.index,
                                id: Some(id),
                                name: Some(name),
                                input_json_delta: String::new(),
                            });
                        }
                        StreamContentBlockType::Text { text } => {
                            if !text.is_empty() {
                                events.push(StreamEvent::TextDelta { text });
                            }
                        }
                        StreamContentBlockType::Unknown => {}
                    }
                }
            }
            "content_block_delta" => {
                if let Ok(delta) = serde_json::from_str::<StreamContentBlockDelta>(data) {
                    match delta.delta {
                        StreamDelta::TextDelta { text } => {
                            events.push(StreamEvent::TextDelta { text });
                        }
                        StreamDelta::InputJsonDelta { partial_json } => {
                            events.push(StreamEvent::ToolCallDelta {
                                index: delta.index,
                                id: None,
                                name: None,
                                input_json_delta: partial_json,
                            });
                        }
                        StreamDelta::Unknown => {}
                    }
                }
            }
            "message_delta" => {
                if let Ok(msg_delta) = serde_json::from_str::<StreamMessageDelta>(data) {
                    if let Some(usage) = msg_delta.usage {
                        *output_tokens = Some(usage.output_tokens);
                    }
                    let stop = Self::map_stop_reason(msg_delta.delta.stop_reason.as_deref());
                    events.push(StreamEvent::Done {
                        stop_reason: stop,
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                    });
                }
            }
            _ => {}
        }
        events
    }

    /// Build a streaming tool-calling request and drive the SSE response into
    /// a `ProviderStream`. Sets `cache_control: ephemeral` on the last tool
    /// definition and on the system prompt block to maximize cache hits.
    async fn chat_with_tools_stream_inner(
        &self,
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ProviderStream> {
        use futures_util::StreamExt;

        let anthropic_messages: Vec<Message> = messages
            .iter()
            .map(Self::provider_message_to_message)
            .collect();
        let mut anthropic_tools = map_tools_optional(tools, |tool| {
            let fields = ToolFields::from_tool_with_description(
                tool,
                scrub_secrets(&tool.description).into_owned(),
            );

            AnthropicToolDef {
                name: fields.name,
                description: fields.description,
                input_schema: fields.parameters,
                cache_control: None,
            }
        });

        if let Some(tools) = anthropic_tools.as_mut() {
            Self::set_cache_control_on_last_tool(tools);
        }

        let request = ChatRequest {
            model: model.to_string(),
            max_tokens: Self::effective_max_tokens(inference_options),
            system: system_prompt.map(|text| {
                vec![SystemBlock {
                    r#type: "text",
                    text: scrub_secrets(text).into_owned(),
                    cache_control: Some(CacheControl::ephemeral()),
                }]
            }),
            messages: anthropic_messages,
            tools: anthropic_tools,
            temperature,
            top_p: inference_options.and_then(|opts| opts.top_p),
            thinking: Self::map_thinking_config(inference_options),
            stream: Some(true),
        };

        let response = self.call_api_streaming(&request).await?;
        let mut byte_stream = response.bytes_stream();

        let stream = async_stream::try_stream! {
            let mut sse_buffer = SseBuffer::new();
            let mut input_tokens: Option<u64> = None;
            let mut output_tokens: Option<u64> = None;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = chunk_result?;
                sse_buffer.push_chunk(&chunk);

                while let Some(event_block) = sse_buffer.next_event_block() {
                    for (event_type, data) in parse_event_pairs(&event_block) {
                        for event in Self::stream_events_from_sse(
                            event_type,
                            data,
                            &mut input_tokens,
                            &mut output_tokens,
                        ) {
                            yield event;
                        }
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

/// Scale a base token limit by `factor`, clamped to the range [0.7, 1.0].
fn scaled_max_tokens(base: u32, factor: f64) -> u32 {
    (f64::from(base) * factor.clamp(0.7, 1.0))
        .round()
        .to_u32()
        .unwrap_or(base)
}

fn anthropic_model_supports_native_tools(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized.starts_with("claude-3")
        || normalized.starts_with("claude-4")
        || normalized.starts_with("claude-sonnet")
        || normalized.starts_with("claude-opus")
        || normalized.starts_with("claude-haiku")
}

fn anthropic_model_supports_vision(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized.starts_with("claude-3")
        || normalized.starts_with("claude-4")
        || normalized.starts_with("claude-sonnet")
        || normalized.starts_with("claude-opus")
        || normalized.starts_with("claude-haiku")
}

impl Provider for AnthropicProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: anthropic_model_supports_native_tools(model),
            streaming: true,
            vision: anthropic_model_supports_vision(model),
        }
    }

    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let chat_response = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            Self::extract_text(&chat_response).map_err(Into::into)
        })
    }

    fn chat_with_system_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let chat_response = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            Self::extract_text(&chat_response).map_err(Into::into)
        })
    }

    fn chat_with_system_full<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let chat_response = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            let text = Self::extract_text(&chat_response)?;
            let mut provider_response = if let Some(usage) = chat_response.usage {
                ProviderResponse::with_usage(text, usage.input_tokens, usage.output_tokens)
            } else {
                ProviderResponse::text_only(text)
            };
            if let Some(api_model) = chat_response.model {
                provider_response = provider_response.with_model(api_model);
            }
            Ok(provider_response)
        })
    }

    fn chat_with_system_full_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let chat_response = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            let text = Self::extract_text(&chat_response)?;
            let mut provider_response = if let Some(usage) = chat_response.usage {
                ProviderResponse::with_usage(text, usage.input_tokens, usage.output_tokens)
            } else {
                ProviderResponse::text_only(text)
            };
            if let Some(api_model) = chat_response.model {
                provider_response = provider_response.with_model(api_model);
            }
            Ok(provider_response)
        })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let anthropic_messages = messages
                .iter()
                .map(Self::provider_message_to_message)
                .collect();
            let mut anthropic_tools = map_tools_optional(tools, |tool| {
                let fields = ToolFields::from_tool_with_description(
                    tool,
                    scrub_secrets(&tool.description).into_owned(),
                );

                AnthropicToolDef {
                    name: fields.name,
                    description: fields.description,
                    input_schema: fields.parameters,
                    cache_control: None,
                }
            });

            if let Some(tools) = anthropic_tools.as_mut() {
                Self::set_cache_control_on_last_tool(tools);
            }

            let request = ChatRequest {
                model: model.to_string(),
                max_tokens: 4096,
                system: system_prompt.map(|text| {
                    vec![SystemBlock {
                        r#type: "text",
                        text: scrub_secrets(text).into_owned(),
                        cache_control: Some(CacheControl::ephemeral()),
                    }]
                }),
                messages: anthropic_messages,
                tools: anthropic_tools,
                temperature,
                top_p: None,
                thinking: None,
                stream: None,
            };
            let chat_response = self.call_api_req(&request).await?;

            let content_blocks = Self::parse_content_blocks(&chat_response.content);
            let text = Self::text_from_content_blocks(&content_blocks).unwrap_or_default();

            let mut provider_response = if let Some(usage) = chat_response.usage {
                ProviderResponse::with_usage(text, usage.input_tokens, usage.output_tokens)
            } else {
                ProviderResponse::text_only(text)
            };
            provider_response.content_blocks = content_blocks;
            provider_response.stop_reason =
                Self::map_stop_reason(chat_response.stop_reason.as_deref());

            if let Some(api_model) = chat_response.model {
                provider_response = provider_response.with_model(api_model);
            }

            Ok(provider_response)
        })
    }

    fn chat_with_tools_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let anthropic_messages = messages
                .iter()
                .map(Self::provider_message_to_message)
                .collect();
            let mut anthropic_tools = map_tools_optional(tools, |tool| {
                let fields = ToolFields::from_tool_with_description(
                    tool,
                    scrub_secrets(&tool.description).into_owned(),
                );

                AnthropicToolDef {
                    name: fields.name,
                    description: fields.description,
                    input_schema: fields.parameters,
                    cache_control: None,
                }
            });

            if let Some(tools) = anthropic_tools.as_mut() {
                Self::set_cache_control_on_last_tool(tools);
            }

            let request = ChatRequest {
                model: model.to_string(),
                max_tokens: Self::effective_max_tokens(inference_options),
                system: system_prompt.map(|text| {
                    vec![SystemBlock {
                        r#type: "text",
                        text: scrub_secrets(text).into_owned(),
                        cache_control: Some(CacheControl::ephemeral()),
                    }]
                }),
                messages: anthropic_messages,
                tools: anthropic_tools,
                temperature,
                top_p: inference_options.and_then(|opts| opts.top_p),
                thinking: Self::map_thinking_config(inference_options),
                stream: None,
            };
            let chat_response = self.call_api_req(&request).await?;

            let content_blocks = Self::parse_content_blocks(&chat_response.content);
            let text = Self::text_from_content_blocks(&content_blocks).unwrap_or_default();

            let mut provider_response = if let Some(usage) = chat_response.usage {
                ProviderResponse::with_usage(text, usage.input_tokens, usage.output_tokens)
            } else {
                ProviderResponse::text_only(text)
            };
            provider_response.content_blocks = content_blocks;
            provider_response.stop_reason =
                Self::map_stop_reason(chat_response.stop_reason.as_deref());

            if let Some(api_model) = chat_response.model {
                provider_response = provider_response.with_model(api_model);
            }

            Ok(provider_response)
        })
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_tools_stream_inner(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                None,
            )
            .await
            .map_err(Into::into)
        })
    }

    fn chat_with_tools_stream_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_tools_stream_inner(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            )
            .await
            .map_err(Into::into)
        })
    }
}

#[cfg(test)]
mod tests;
