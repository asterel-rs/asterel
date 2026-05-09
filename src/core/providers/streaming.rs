//! Provider streaming infrastructure: stream types, sinks, fan-out,
//! secret scrubbing, and response collection.
mod redactor;
mod sink;

use std::pin::Pin;

use anyhow::Result;
use futures_util::Stream;
use serde::{Deserialize, Serialize};

use crate::core::providers::response::{ContentBlock, ProviderResponse, StopReason};
use crate::security::scrub::scrub_secrets;

pub use self::sink::{
    ChannelStreamSink, CliStreamSink, FanoutStreamSink, NullStreamSink, StreamSink,
};

/// Boxed async stream of provider streaming events.
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send + 'static>>;

/// Incremental event emitted during a streaming provider response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    /// First event indicating the model used.
    ResponseStart {
        /// Model identifier returned by the API, if available.
        model: Option<String>,
    },
    /// Incremental text fragment.
    TextDelta {
        /// The text fragment.
        text: String,
    },
    /// Incremental tool call argument data.
    ToolCallDelta {
        /// Zero-based tool call index within the response.
        index: u32,
        /// Tool call ID (set on the first delta for this index).
        id: Option<String>,
        /// Tool name (set on the first delta for this index).
        name: Option<String>,
        /// Partial JSON fragment to append to the arguments.
        input_json_delta: String,
    },
    /// Fully assembled tool call ready for execution.
    ToolCallComplete {
        /// Unique tool call identifier.
        id: String,
        /// Name of the tool to invoke.
        name: String,
        /// Parsed JSON arguments.
        input: serde_json::Value,
    },
    /// Terminal event with usage statistics.
    Done {
        /// Reason the model stopped generating.
        stop_reason: Option<StopReason>,
        /// Prompt tokens consumed.
        input_tokens: Option<u64>,
        /// Completion tokens generated.
        output_tokens: Option<u64>,
    },
}

/// Maximum number of concurrent in-flight tool call builders. Protects
/// against OOM caused by malicious or buggy streaming delta indices.
const MAX_TOOL_CALL_BUILDERS: usize = 128;
/// Maximum accumulated visible text retained by a stream collector.
const MAX_STREAM_TEXT_BYTES: usize = 1_048_576;
/// Maximum argument JSON retained for a single streamed tool call.
const MAX_TOOL_CALL_INPUT_JSON_BYTES: usize = 262_144;
/// Maximum aggregate argument JSON retained across all streamed tool calls.
const MAX_TOTAL_TOOL_CALL_INPUT_JSON_BYTES: usize = 1_048_576;

/// Accumulates streaming events into a complete `ProviderResponse`.
pub struct StreamCollector {
    text: String,
    content_blocks: Vec<ContentBlock>,
    tool_call_builders: Vec<ToolCallBuilder>,
    total_tool_input_json_bytes: usize,
    stop_reason: Option<StopReason>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    model: Option<String>,
}

#[derive(Default)]
struct ToolCallBuilder {
    id: String,
    name: String,
    input_json: String,
}

impl ToolCallBuilder {
    fn into_content_block(self) -> Option<ContentBlock> {
        if self.id.is_empty() || self.name.is_empty() {
            if !self.input_json.trim().is_empty() {
                tracing::warn!("Skipping incomplete streamed tool call (missing id or name)");
            }
            return None;
        }

        match serde_json::from_str::<serde_json::Value>(&self.input_json) {
            Ok(input) => Some(ContentBlock::ToolUse {
                id: self.id,
                name: self.name,
                input,
            }),
            Err(error) => {
                tracing::warn!(
                    tool_id = self.id,
                    tool_name = self.name,
                    "Skipping malformed streamed tool call JSON: {error}"
                );
                None
            }
        }
    }
}

impl StreamCollector {
    /// Create an empty collector ready to receive events.
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: String::new(),
            content_blocks: Vec::new(),
            tool_call_builders: Vec::new(),
            total_tool_input_json_bytes: 0,
            stop_reason: None,
            input_tokens: None,
            output_tokens: None,
            model: None,
        }
    }

    /// Ingest a single streaming event into the collector.
    pub fn feed(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::ResponseStart { model } => {
                self.model.clone_from(model);
            }
            StreamEvent::TextDelta { text } => {
                append_capped_utf8(&mut self.text, text, MAX_STREAM_TEXT_BYTES, "stream text");
            }
            StreamEvent::ToolCallDelta {
                index,
                id,
                name,
                input_json_delta,
            } => {
                self.feed_tool_call_delta(*index, id.as_deref(), name.as_deref(), input_json_delta);
            }
            StreamEvent::ToolCallComplete { id, name, input } => {
                self.content_blocks.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            StreamEvent::Done {
                stop_reason,
                input_tokens,
                output_tokens,
            } => {
                self.stop_reason = *stop_reason;
                self.input_tokens = *input_tokens;
                self.output_tokens = *output_tokens;
            }
        }
    }

    fn feed_tool_call_delta(
        &mut self,
        index: u32,
        id: Option<&str>,
        name: Option<&str>,
        input_json_delta: &str,
    ) {
        let Some(builder_index) = Self::builder_index(index) else {
            return;
        };
        self.ensure_builder_slot(builder_index);

        if let Some(builder) = self.tool_call_builders.get_mut(builder_index) {
            if let Some(call_id) = id {
                builder.id = call_id.to_string();
            }
            if let Some(call_name) = name {
                builder.name = call_name.to_string();
            }
            let remaining_total = MAX_TOTAL_TOOL_CALL_INPUT_JSON_BYTES
                .saturating_sub(self.total_tool_input_json_bytes);
            let remaining_builder =
                MAX_TOOL_CALL_INPUT_JSON_BYTES.saturating_sub(builder.input_json.len());
            let max_append = remaining_total.min(remaining_builder);
            let max_len = builder.input_json.len().saturating_add(max_append);
            let appended = append_capped_utf8(
                &mut builder.input_json,
                input_json_delta,
                max_len,
                "stream tool call JSON",
            );
            self.total_tool_input_json_bytes =
                self.total_tool_input_json_bytes.saturating_add(appended);
        }
    }

    fn builder_index(index: u32) -> Option<usize> {
        let Ok(builder_index) = usize::try_from(index) else {
            tracing::warn!(
                index,
                "Skipping tool call delta due to non-convertible index"
            );
            return None;
        };

        if builder_index >= MAX_TOOL_CALL_BUILDERS {
            tracing::warn!(
                index = builder_index,
                max = MAX_TOOL_CALL_BUILDERS,
                "tool call delta index exceeds maximum; skipping"
            );
            return None;
        }

        Some(builder_index)
    }

    fn ensure_builder_slot(&mut self, builder_index: usize) {
        while self.tool_call_builders.len() <= builder_index {
            self.tool_call_builders.push(ToolCallBuilder::default());
        }
    }

    /// Consume the collector and build the final `ProviderResponse`.
    #[must_use]
    pub fn finish(mut self) -> ProviderResponse {
        for builder in self.tool_call_builders {
            if let Some(content_block) = builder.into_content_block() {
                self.content_blocks.push(content_block);
            }
        }

        if !self.text.is_empty() {
            self.content_blocks.insert(
                0,
                ContentBlock::Text {
                    text: self.text.clone(),
                },
            );
        }

        ProviderResponse {
            text: self.text,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            model: self.model,
            content_blocks: self.content_blocks,
            stop_reason: self.stop_reason,
            logprobs: None,
        }
    }
}

fn append_capped_utf8(target: &mut String, input: &str, max_bytes: usize, field: &str) -> usize {
    let remaining = max_bytes.saturating_sub(target.len());
    if remaining == 0 {
        if !input.is_empty() {
            tracing::warn!(field, max_bytes, "Skipping stream fragment after byte cap");
        }
        return 0;
    }

    if input.len() <= remaining {
        target.push_str(input);
        return input.len();
    }

    let split = input
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= remaining)
        .last()
        .unwrap_or(0);
    if split > 0 {
        target.push_str(&input[..split]);
    }
    tracing::warn!(field, max_bytes, "Truncated stream fragment at byte cap");
    split
}

impl Default for StreamCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a completed `ProviderResponse` into a sequence of stream events.
#[must_use]
pub fn resp_to_events(resp: ProviderResponse) -> Vec<Result<StreamEvent>> {
    let ProviderResponse {
        text,
        input_tokens,
        output_tokens,
        model,
        content_blocks,
        stop_reason,
        logprobs: _,
    } = resp;

    let mut events = vec![Ok(StreamEvent::ResponseStart { model })];
    if !text.is_empty() {
        events.push(Ok(StreamEvent::TextDelta { text }));
    }
    for block in content_blocks {
        match block {
            ContentBlock::ToolUse { id, name, input } => {
                events.push(Ok(StreamEvent::ToolCallComplete { id, name, input }));
            }
            ContentBlock::Text { .. }
            | ContentBlock::ToolResult { .. }
            | ContentBlock::Image { .. } => {}
        }
    }
    events.push(Ok(StreamEvent::Done {
        stop_reason,
        input_tokens,
        output_tokens,
    }));
    events
}

/// Sliding-window secret scrubber for incremental streaming text deltas.
///
/// Buffers a trailing `window`-byte carry so that secrets spanning chunk
/// boundaries are detected. Call `scrub_delta` for each incoming text
/// fragment; call `finish` once the stream ends to flush the carry buffer.
pub struct StreamingSecretScrubber {
    carry: String,
    window: usize,
}

impl StreamingSecretScrubber {
    /// Create a scrubber with the given lookahead window size.
    #[must_use]
    pub fn new(window: usize) -> Self {
        Self {
            carry: String::new(),
            window: window.max(64),
        }
    }

    /// Scrub secrets from an incoming text delta and return the safe portion.
    ///
    /// The last `window` bytes of the combined buffer are retained as carry
    /// for the next call. Only the prefix beyond the window is emitted, ensuring
    /// secrets that span chunk boundaries are not leaked prematurely.
    pub fn scrub_delta(&mut self, delta: &str) -> String {
        let mut combined = std::mem::take(&mut self.carry);
        combined.push_str(delta);

        let scrubbed = scrub_secrets(&combined).into_owned();
        if scrubbed.len() > self.window {
            let mut split_at = scrubbed.len() - self.window;
            while split_at > 0 && !scrubbed.is_char_boundary(split_at) {
                split_at -= 1;
            }

            let emitted = scrubbed[..split_at].to_string();
            self.carry = scrubbed[split_at..].to_string();
            emitted
        } else {
            self.carry = scrubbed;
            String::new()
        }
    }

    /// Flush the remaining buffer and return the final scrubbed text.
    #[must_use]
    pub fn finish(self) -> String {
        scrub_secrets(&self.carry).into_owned()
    }
}

#[cfg(test)]
mod tests;
