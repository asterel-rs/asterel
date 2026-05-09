//! `OpenAI` Chat Completions request building, response parsing, and
//! SSE streaming logic shared with the `OpenRouter` provider.

use anyhow::Context;
use futures_util::StreamExt;
use num_traits::ToPrimitive;
use serde_json::Value;

use super::types::{
    ChatCompletionChunk, ChatRequest, ChatResponse, ContentPart, ImageUrlContent, Message,
    MessageContent, OpenAiTool, OpenAiToolCall, OpenAiToolCallFunction, OpenAiToolDefinition,
    ReasoningEffort, StreamOptions, Usage,
};
use crate::core::providers::sse::{SseBuffer, parse_data_lines_no_done};
use crate::core::providers::streaming::{ProviderStream, StreamEvent};
use crate::core::providers::tool_convert::{ToolFields, map_tools_optional};
use crate::core::providers::{
    ContentBlock, ImageSource, InferenceOpts, MessageRole, ProviderMessage, ProviderResponse,
    StopReason, scrub_secrets,
};
use crate::core::tools::traits::ToolSpec;

pub(in crate::core::providers) fn build_request(
    system_prompt: Option<&str>,
    message: &str,
    model: &str,
    temperature: f64,
    inference_options: Option<&InferenceOpts>,
) -> ChatRequest {
    let capacity = if system_prompt.is_some() { 2 } else { 1 };
    let mut messages = Vec::with_capacity(capacity);

    if let Some(sys) = system_prompt {
        messages.push(Message {
            role: "system",
            content: Some(MessageContent::Text(scrub_secrets(sys).into_owned())),
            tool_call_id: None,
            tool_calls: None,
        });
    }

    messages.push(Message {
        role: "user",
        content: Some(MessageContent::Text(scrub_secrets(message).into_owned())),
        tool_call_id: None,
        tool_calls: None,
    });

    ChatRequest {
        model: model.to_string(),
        messages,
        temperature,
        top_p: inference_options.and_then(|opts| opts.top_p),
        max_tokens: map_max_tokens(inference_options),
        reasoning_effort: map_reasoning_effort(inference_options),
        tools: None,
        stream: None,
        stream_options: None,
    }
}

pub(in crate::core::providers) fn build_text_message(
    role: &'static str,
    content: String,
) -> Message {
    Message {
        role,
        content: Some(MessageContent::Text(content)),
        tool_call_id: None,
        tool_calls: None,
    }
}

pub(in crate::core::providers) fn map_provider_message(
    provider_message: &ProviderMessage,
) -> Vec<Message> {
    let mut text_buf = String::new();
    let mut image_parts = Vec::new();
    let mut assistant_tool_calls = Vec::new();
    let mut tool_messages = Vec::new();

    for block in &provider_message.content {
        match block {
            ContentBlock::Text { text } => {
                if !text_buf.is_empty() {
                    text_buf.push('\n');
                }
                text_buf.push_str(&scrub_secrets(text));
            }
            ContentBlock::ToolUse { id, name, input } => {
                assistant_tool_calls.push(OpenAiToolCall {
                    id: id.clone(),
                    r#type: "function".to_string(),
                    function: OpenAiToolCallFunction {
                        name: name.clone(),
                        arguments: input.to_string(),
                    },
                });
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: _,
            } => {
                tool_messages.push(Message {
                    role: "tool",
                    content: Some(MessageContent::Text(scrub_secrets(content).into_owned())),
                    tool_call_id: Some(tool_use_id.clone()),
                    tool_calls: None,
                });
            }
            ContentBlock::Image { source } => {
                let url = match source {
                    ImageSource::Base64 { media_type, data } => {
                        format!("data:{media_type};base64,{data}")
                    }
                    ImageSource::Url { url } => url.clone(),
                };
                image_parts.push(ContentPart::ImageUrl {
                    image_url: ImageUrlContent { url },
                });
            }
        }
    }

    let mut messages = Vec::new();
    let text_content = if text_buf.is_empty() {
        None
    } else {
        Some(text_buf)
    };

    match provider_message.role {
        MessageRole::Assistant => {
            if text_content.is_some() || !assistant_tool_calls.is_empty() {
                messages.push(Message {
                    role: "assistant",
                    content: text_content.map(MessageContent::Text),
                    tool_call_id: None,
                    tool_calls: if assistant_tool_calls.is_empty() {
                        None
                    } else {
                        Some(assistant_tool_calls)
                    },
                });
            }
        }
        MessageRole::User => {
            if image_parts.is_empty() {
                if let Some(content) = text_content {
                    messages.push(build_text_message("user", content));
                }
            } else {
                let mut parts = Vec::new();
                if let Some(text) = text_content {
                    parts.push(ContentPart::Text { text });
                }
                parts.extend(image_parts);
                messages.push(Message {
                    role: "user",
                    content: Some(MessageContent::Parts(parts)),
                    tool_call_id: None,
                    tool_calls: None,
                });
            }
        }
        MessageRole::System => {
            if let Some(content) = text_content {
                messages.push(build_text_message("system", content));
            }
        }
    }

    messages.extend(tool_messages);
    messages
}

pub(in crate::core::providers) fn build_openai_tools(
    tools: &[ToolSpec],
) -> Option<Vec<OpenAiTool>> {
    map_tools_optional(tools, |tool| {
        let fields = ToolFields::from_tool(tool);
        OpenAiTool {
            r#type: "function",
            function: OpenAiToolDefinition {
                name: fields.name,
                description: fields.description,
                parameters: fields.parameters,
            },
        }
    })
}

fn build_messages(system_prompt: Option<&str>, messages: &[ProviderMessage]) -> Vec<Message> {
    let mut openai_messages = Vec::new();

    if let Some(sys) = system_prompt {
        openai_messages.push(build_text_message(
            "system",
            scrub_secrets(sys).into_owned(),
        ));
    }

    for provider_message in messages {
        openai_messages.extend(map_provider_message(provider_message));
    }

    openai_messages
}

pub(in crate::core::providers) fn build_tools_request(
    system_prompt: Option<&str>,
    messages: &[ProviderMessage],
    tools: &[ToolSpec],
    model: &str,
    temperature: f64,
    inference_options: Option<&InferenceOpts>,
) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: build_messages(system_prompt, messages),
        temperature,
        top_p: inference_options.and_then(|opts| opts.top_p),
        max_tokens: map_max_tokens(inference_options),
        reasoning_effort: map_reasoning_effort(inference_options),
        tools: build_openai_tools(tools),
        stream: None,
        stream_options: None,
    }
}

pub(in crate::core::providers) fn build_stream_request(
    system_prompt: Option<&str>,
    messages: &[ProviderMessage],
    tools: &[ToolSpec],
    model: &str,
    temperature: f64,
    inference_options: Option<&InferenceOpts>,
) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: build_messages(system_prompt, messages),
        temperature,
        top_p: inference_options.and_then(|opts| opts.top_p),
        max_tokens: map_max_tokens(inference_options),
        reasoning_effort: map_reasoning_effort(inference_options),
        tools: build_openai_tools(tools),
        stream: Some(true),
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
    }
}

fn map_reasoning_effort(inference_options: Option<&InferenceOpts>) -> Option<ReasoningEffort> {
    let effort = inference_options.and_then(|options| {
        crate::core::providers::inference::openai_reasoning_effort(options.thinking_level)
    })?;
    Some(match effort {
        "low" => ReasoningEffort::Low,
        "high" => ReasoningEffort::High,
        _ => ReasoningEffort::Medium,
    })
}

fn map_max_tokens(inference_options: Option<&InferenceOpts>) -> Option<u32> {
    const BASE_MAX_TOKENS: u32 = 4096;
    let factor = inference_options.and_then(|opts| opts.max_tokens_factor)?;
    Some(scaled_max_tokens(BASE_MAX_TOKENS, factor))
}

fn scaled_max_tokens(base: u32, factor: f64) -> u32 {
    (f64::from(base) * factor.clamp(0.7, 1.0))
        .round()
        .to_u32()
        .unwrap_or(base)
}

pub(in crate::core::providers) fn extract_text(
    chat_response: &ChatResponse,
    provider_name: &str,
) -> anyhow::Result<String> {
    chat_response
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| {
            anyhow::Error::from(crate::core::providers::ProviderError::EmptyResponse {
                provider: provider_name.to_string(),
            })
        })
}

pub(in crate::core::providers) fn map_finish_reason(finish_reason: Option<&str>) -> StopReason {
    match finish_reason {
        Some("stop") => StopReason::EndTurn,
        Some("tool_calls") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some(_) | None => StopReason::Error,
    }
}

pub(in crate::core::providers) fn parse_tool_calls(
    tool_calls: Option<Vec<OpenAiToolCall>>,
    provider_name: &str,
) -> anyhow::Result<Vec<ContentBlock>> {
    tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|tool_call| {
            let input: Value =
                serde_json::from_str(&tool_call.function.arguments).with_context(|| {
                    format!(
                        "{provider_name} tool call arguments were not valid JSON for {}",
                        tool_call.function.name
                    )
                })?;
            Ok(ContentBlock::ToolUse {
                id: tool_call.id,
                name: tool_call.function.name,
                input,
            })
        })
        .collect()
}

pub(in crate::core::providers) struct ChatCompletionsEndpoint<'a> {
    pub(in crate::core::providers) provider_name: &'a str,
    pub(in crate::core::providers) url: &'a str,
    pub(in crate::core::providers) missing_api_key_message: &'a str,
    pub(in crate::core::providers) extra_headers: &'a [(&'a str, &'a str)],
}

pub(in crate::core::providers) async fn send_chat_completions_raw(
    client: &reqwest::Client,
    cached_auth_header: Option<&String>,
    request: &ChatRequest,
    endpoint: ChatCompletionsEndpoint<'_>,
) -> anyhow::Result<reqwest::Response> {
    let auth_header = cached_auth_header.ok_or_else(|| {
        crate::core::providers::ProviderError::MissingCredentials {
            provider: endpoint.provider_name.to_string(),
            message: endpoint.missing_api_key_message.to_string(),
        }
    })?;

    let mut request_builder = client
        .post(endpoint.url)
        .header("Authorization", auth_header)
        .json(request);

    for (name, value) in endpoint.extra_headers {
        request_builder = request_builder.header(*name, *value);
    }

    let response = request_builder.send().await.map_err(|error| {
        crate::core::providers::ProviderError::Network {
            provider: endpoint.provider_name.to_string(),
            source: error,
        }
    })?;

    if !response.status().is_success() {
        return Err(crate::core::providers::api_error(endpoint.provider_name, response).await);
    }

    Ok(response)
}

pub(in crate::core::providers) async fn send_chat_completions_json(
    client: &reqwest::Client,
    cached_auth_header: Option<&String>,
    request: &ChatRequest,
    endpoint: ChatCompletionsEndpoint<'_>,
) -> anyhow::Result<ChatResponse> {
    let provider_name = endpoint.provider_name;
    let response = send_chat_completions_raw(client, cached_auth_header, request, endpoint).await?;

    response.json().await.map_err(|error| {
        anyhow::Error::from(crate::core::providers::ProviderError::ResponseParse {
            provider: provider_name.to_string(),
            message: format!("JSON decode failed: {error}"),
        })
    })
}

fn provider_response_with_usage(text: String, usage: Option<&Usage>) -> ProviderResponse {
    if let Some(usage) = usage {
        ProviderResponse::with_usage(text, usage.prompt_tokens, usage.completion_tokens)
    } else {
        ProviderResponse::text_only(text)
    }
}

pub(in crate::core::providers) fn build_text_provider_response(
    chat_response: ChatResponse,
    provider_name: &str,
) -> anyhow::Result<ProviderResponse> {
    let text = extract_text(&chat_response, provider_name)?;
    let mut provider_response = provider_response_with_usage(text, chat_response.usage.as_ref());

    if let Some(api_model) = chat_response.model {
        provider_response = provider_response.with_model(api_model);
    }

    Ok(provider_response)
}

pub(in crate::core::providers) fn build_tool_provider_response(
    chat_response: ChatResponse,
    provider_name: &str,
) -> anyhow::Result<ProviderResponse> {
    let choice = chat_response.choices.first().ok_or_else(|| {
        crate::core::providers::ProviderError::EmptyResponse {
            provider: provider_name.to_string(),
        }
    })?;

    let text = choice.message.content.clone().unwrap_or_default();
    let scrubbed_text = scrub_secrets(&text).into_owned();
    let mut content_blocks = parse_tool_calls(choice.message.tool_calls.clone(), provider_name)?;

    if !scrubbed_text.is_empty() {
        content_blocks.insert(
            0,
            ContentBlock::Text {
                text: scrubbed_text.clone(),
            },
        );
    }

    let mut provider_response =
        provider_response_with_usage(scrubbed_text, chat_response.usage.as_ref());
    provider_response.content_blocks = content_blocks;
    provider_response.stop_reason = Some(map_finish_reason(choice.finish_reason.as_deref()));

    if let Some(api_model) = chat_response.model {
        provider_response = provider_response.with_model(api_model);
    }

    Ok(provider_response)
}

pub(in crate::core::providers) fn sse_response_to_provider_stream(
    response: reqwest::Response,
) -> ProviderStream {
    let mut byte_stream = response.bytes_stream();

    let stream = async_stream::try_stream! {
        let mut sse_buffer = SseBuffer::new();
        let mut sent_start = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let chunk = chunk_result?;
            sse_buffer.push_chunk(&chunk);

            while let Some(event_block) = sse_buffer.next_event_block() {
                for event in events_from_openai_sse_block(&event_block, &mut sent_start)? {
                    yield event;
                }
            }
        }

        if let Some(event_block) = sse_buffer.finish_event_block() {
            for event in events_from_openai_sse_block(&event_block, &mut sent_start)? {
                yield event;
            }
        }
    };

    Box::pin(stream)
}

pub(super) fn events_from_openai_sse_block(
    event_block: &str,
    sent_start: &mut bool,
) -> anyhow::Result<Vec<StreamEvent>> {
    let mut events = Vec::new();
    for data in parse_data_lines_no_done(event_block) {
        let chunk = serde_json::from_str::<ChatCompletionChunk>(data).map_err(|error| {
            anyhow::anyhow!("OpenAI-compatible stream returned malformed SSE JSON chunk: {error}")
        })?;

        if !*sent_start {
            events.push(StreamEvent::ResponseStart {
                model: chunk.model.clone(),
            });
            *sent_start = true;
        }

        for choice in &chunk.choices {
            if let Some(content) = &choice.delta.content
                && !content.is_empty()
            {
                events.push(StreamEvent::TextDelta {
                    text: scrub_secrets(content).into_owned(),
                });
            }

            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tool_call in tool_calls {
                    events.push(StreamEvent::ToolCallDelta {
                        index: tool_call.index,
                        id: tool_call.id.clone(),
                        name: tool_call.function.as_ref().and_then(|f| f.name.clone()),
                        input_json_delta: tool_call
                            .function
                            .as_ref()
                            .and_then(|f| f.arguments.clone())
                            .unwrap_or_default(),
                    });
                }
            }

            if let Some(finish) = choice.finish_reason.as_deref() {
                let (input_t, output_t) = chunk.usage.as_ref().map_or((None, None), |u| {
                    (Some(u.prompt_tokens), Some(u.completion_tokens))
                });
                events.push(StreamEvent::Done {
                    stop_reason: Some(map_finish_reason(Some(finish))),
                    input_tokens: input_t,
                    output_tokens: output_t,
                });
            }
        }
    }
    Ok(events)
}
