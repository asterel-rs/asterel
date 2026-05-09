//! Responses API call path for the OpenAI-compatible provider.
//!
//! Implements the newer `OpenAI` Responses endpoint with streaming
//! SSE and tool-call extraction.

use anyhow::Context;
use futures_util::StreamExt;

use super::OpenAiCompatProvider;
use super::types::{
    ResponsesInputItem, ResponsesRequest, ResponsesResponse, build_responses_input_from_messages,
    build_responses_tools, extract_responses_sse_text, extract_responses_text,
    extract_responses_tool_calls,
};
use crate::core::providers::sse::{SseBuffer, parse_event_pairs};
use crate::core::providers::streaming::{ProviderStream, StreamEvent};
use crate::core::providers::{
    ContentBlock, ProviderMessage, ProviderResponse, StopReason, sanitize_api_error, scrub_secrets,
};
use crate::core::tools::traits::ToolSpec;

fn parse_responses_sse_value(data: &str) -> anyhow::Result<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(data).map_err(|error| {
        anyhow::anyhow!("Responses stream returned malformed SSE JSON event: {error}")
    })
}

pub(super) fn events_from_responses_sse_block(
    event_block: &str,
    sent_start: &mut bool,
    tool_call_index: &mut u32,
) -> anyhow::Result<Vec<StreamEvent>> {
    let mut events = Vec::new();

    for (event_type, data) in parse_event_pairs(event_block) {
        let value = parse_responses_sse_value(data)?;

        if !*sent_start {
            events.push(StreamEvent::ResponseStart { model: None });
            *sent_start = true;
        }

        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(serde_json::Value::as_str) {
                    events.push(StreamEvent::TextDelta {
                        text: scrub_secrets(delta).into_owned(),
                    });
                }
            }
            "response.output_item.added" => {
                if value
                    .get("item")
                    .and_then(|i| i.get("type"))
                    .and_then(serde_json::Value::as_str)
                    == Some("function_call")
                {
                    let item = &value["item"];
                    let call_id = item
                        .get("call_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    events.push(StreamEvent::ToolCallDelta {
                        index: *tool_call_index,
                        id: Some(call_id),
                        name: Some(name),
                        input_json_delta: String::new(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = value.get("delta").and_then(serde_json::Value::as_str) {
                    events.push(StreamEvent::ToolCallDelta {
                        index: *tool_call_index,
                        id: None,
                        name: None,
                        input_json_delta: delta.to_string(),
                    });
                }
            }
            "response.output_item.done" => {
                if value
                    .get("item")
                    .and_then(|i| i.get("type"))
                    .and_then(serde_json::Value::as_str)
                    == Some("function_call")
                {
                    *tool_call_index += 1;
                }
            }
            "response.completed" => {
                let incomplete = value
                    .pointer("/response/status")
                    .and_then(serde_json::Value::as_str)
                    == Some("incomplete");
                events.push(StreamEvent::Done {
                    stop_reason: Some(if incomplete {
                        StopReason::ToolUse
                    } else {
                        StopReason::EndTurn
                    }),
                    input_tokens: value
                        .pointer("/response/usage/input_tokens")
                        .and_then(serde_json::Value::as_u64),
                    output_tokens: value
                        .pointer("/response/usage/output_tokens")
                        .and_then(serde_json::Value::as_u64),
                });
            }
            _ => {}
        }
    }

    Ok(events)
}

impl OpenAiCompatProvider {
    /// Send a text-only request via the Responses API endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure or response parsing failure.
    pub(super) async fn chat_via_responses(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
    ) -> anyhow::Result<ProviderResponse> {
        let request = ResponsesRequest {
            model: model.to_string(),
            input: vec![ResponsesInputItem::Message {
                role: "user",
                content: scrub_secrets(message).into_owned(),
            }],
            instructions: system_prompt.map(|text| scrub_secrets(text).into_owned()),
            tools: None,
            store: self.prefer_responses_api.then_some(false),
            stream: Some(self.prefer_responses_api),
        };

        let url = self.responses_url();

        let response = self
            .apply_auth_header(self.client.post(url).json(&request))
            .send()
            .await
            .with_context(|| format!("{} Responses API request failed", self.name))?;

        if !response.status().is_success() {
            let error = response.text().await?;
            let sanitized = sanitize_api_error(&error);
            anyhow::bail!("{} Responses API error: {sanitized}", self.name);
        }

        let raw_body = response
            .text()
            .await
            .with_context(|| format!("{} Responses API body read failed", self.name))?;

        let text = if let Ok(responses) = serde_json::from_str::<ResponsesResponse>(&raw_body) {
            extract_responses_text(&responses)
        } else {
            extract_responses_sse_text(&raw_body)
        }
        .ok_or_else(|| super::super::ProviderError::ResponseParse {
            provider: self.name.clone(),
            message: "Responses API JSON decode failed".into(),
        })?;
        let text = scrub_secrets(&text).into_owned();

        Ok(ProviderResponse::text_only(text))
    }

    /// Send a Responses API request with native tool definitions, parse
    /// the response for both text and `function_call` output items.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure, body read failure, or JSON
    /// deserialization failure.
    pub(super) async fn chat_via_responses_with_tools(
        &self,
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
    ) -> anyhow::Result<ProviderResponse> {
        let input = build_responses_input_from_messages(messages);

        let request = ResponsesRequest {
            model: model.to_string(),
            input,
            instructions: system_prompt.map(|text| scrub_secrets(text).into_owned()),
            tools: build_responses_tools(tools),
            store: Some(false),
            stream: Some(false),
        };

        let url = self.responses_url();

        let response = self
            .apply_auth_header(self.client.post(url).json(&request))
            .send()
            .await
            .with_context(|| format!("{} Responses API request failed", self.name))?;

        if !response.status().is_success() {
            let error = response.text().await?;
            let sanitized = sanitize_api_error(&error);
            anyhow::bail!("{} Responses API error: {sanitized}", self.name);
        }

        let raw_body = response
            .text()
            .await
            .with_context(|| format!("{} Responses API body read failed", self.name))?;

        if tracing::enabled!(tracing::Level::DEBUG) {
            let raw_body_preview = sanitize_api_error(&raw_body);
            tracing::debug!(
                raw_body_len = raw_body.len(),
                raw_body_preview = %raw_body_preview,
                "Responses API raw body"
            );
        }

        let responses: ResponsesResponse = serde_json::from_str(&raw_body)
            .with_context(|| format!("{} Responses API JSON decode failed", self.name))?;

        Self::build_responses_provider_response(&responses, &self.name)
    }

    /// Build a `ProviderResponse` from a parsed Responses API body,
    /// extracting text and tool-call content blocks.
    ///
    /// # Errors
    ///
    /// Returns an error if tool call arguments are not valid JSON.
    pub(super) fn build_responses_provider_response(
        response: &ResponsesResponse,
        provider_name: &str,
    ) -> anyhow::Result<ProviderResponse> {
        let text = extract_responses_text(response)
            .map(|text| scrub_secrets(&text).into_owned())
            .unwrap_or_default();
        let native_tool_calls = extract_responses_tool_calls(response);

        if !native_tool_calls.is_empty() {
            let mut content_blocks = Vec::new();
            if !text.is_empty() {
                content_blocks.push(ContentBlock::Text { text: text.clone() });
            }
            for (call_id, name, arguments) in &native_tool_calls {
                let input: serde_json::Value =
                    serde_json::from_str(arguments).with_context(|| {
                        format!(
                            "{provider_name} tool call arguments were not valid JSON for {name}"
                        )
                    })?;
                content_blocks.push(ContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: name.clone(),
                    input,
                });
            }
            let (input_tokens, output_tokens) =
                response.usage.as_ref().map_or((None, None), |usage| {
                    (usage.input_tokens, usage.output_tokens)
                });
            return Ok(ProviderResponse {
                text,
                input_tokens,
                output_tokens,
                model: None,
                content_blocks,
                stop_reason: Some(StopReason::ToolUse),
                logprobs: None,
            });
        }

        let (input_tokens, output_tokens) = response.usage.as_ref().map_or((None, None), |usage| {
            (usage.input_tokens, usage.output_tokens)
        });
        let content_blocks = if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentBlock::Text { text: text.clone() }]
        };
        Ok(ProviderResponse {
            text,
            input_tokens,
            output_tokens,
            model: None,
            content_blocks,
            stop_reason: Some(StopReason::EndTurn),
            logprobs: None,
        })
    }

    /// Stream a Responses API request, yielding `StreamEvent`s for text
    /// deltas and `function_call` items.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure or if the request cannot be sent.
    pub(super) async fn chat_via_responses_stream(
        &self,
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
    ) -> anyhow::Result<ProviderStream> {
        let input = build_responses_input_from_messages(messages);

        let request = ResponsesRequest {
            model: model.to_string(),
            input,
            instructions: system_prompt.map(str::to_string),
            tools: build_responses_tools(tools),
            store: Some(false),
            stream: Some(true),
        };

        let url = self.responses_url();

        let response = self
            .apply_auth_header(self.client.post(url).json(&request))
            .send()
            .await
            .with_context(|| format!("{} Responses API request failed", self.name))?;

        if !response.status().is_success() {
            let error = response.text().await?;
            let sanitized = sanitize_api_error(&error);
            anyhow::bail!("{} Responses API error: {sanitized}", self.name);
        }

        Ok(Self::responses_sse_to_provider_stream(response))
    }

    /// Convert a raw SSE HTTP response from the Responses API into a `ProviderStream`.
    ///
    /// Handles the following `OpenAI` Responses SSE event types:
    /// - `response.output_text.delta` → `TextDelta`
    /// - `response.output_item.added` (type `function_call`) → `ToolCallDelta` (open)
    /// - `response.function_call_arguments.delta` → `ToolCallDelta` (arguments fragment)
    /// - `response.output_item.done` (type `function_call`) → advances `tool_call_index`
    /// - `response.completed` → `Done` with usage and stop reason
    pub(super) fn responses_sse_to_provider_stream(response: reqwest::Response) -> ProviderStream {
        let mut byte_stream = response.bytes_stream();

        let stream = async_stream::try_stream! {
            let mut sse_buffer = SseBuffer::new();
            let mut sent_start = false;
            let mut tool_call_index: u32 = 0;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = chunk_result?;
                sse_buffer.push_chunk(&chunk);

                while let Some(event_block) = sse_buffer.next_event_block() {
                    for event in events_from_responses_sse_block(
                        &event_block,
                        &mut sent_start,
                        &mut tool_call_index,
                    )? {
                        yield event;
                    }
                }
            }

            if let Some(event_block) = sse_buffer.finish_event_block() {
                for event in events_from_responses_sse_block(
                    &event_block,
                    &mut sent_start,
                    &mut tool_call_index,
                )? {
                    yield event;
                }
            }
        };

        Box::pin(stream)
    }
}
