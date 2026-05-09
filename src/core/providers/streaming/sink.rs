use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use super::StreamEvent;
use super::redactor::ReasoningStreamRedactor;

/// Async sink that receives streaming events for side-effects.
pub trait StreamSink: Send + Sync {
    /// Process a single streaming event.
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// No-op stream sink that discards all events.
#[derive(Debug, Default)]
pub struct NullStreamSink;

impl StreamSink for NullStreamSink {
    fn on_event<'a>(
        &'a self,
        _event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

/// Stream sink that buffers text deltas and flushes to an mpsc channel.
pub struct ChannelStreamSink {
    sender: mpsc::Sender<String>,
    state: Mutex<ChannelStreamSinkState>,
    flush_threshold: usize,
}

#[derive(Default)]
struct ChannelStreamSinkState {
    buffer: String,
    redactor: ReasoningStreamRedactor,
}

impl ChannelStreamSink {
    /// Create a channel sink with reasoning blocks hidden.
    #[must_use]
    pub fn new(sender: mpsc::Sender<String>, flush_threshold: usize) -> Self {
        Self::new_with_reasoning(sender, flush_threshold, false)
    }

    /// Create a channel sink with configurable reasoning visibility.
    #[must_use]
    pub fn new_with_reasoning(
        sender: mpsc::Sender<String>,
        flush_threshold: usize,
        show_reasoning: bool,
    ) -> Self {
        Self {
            sender,
            state: Mutex::new(ChannelStreamSinkState {
                buffer: String::new(),
                redactor: ReasoningStreamRedactor::new(show_reasoning),
            }),
            flush_threshold: flush_threshold.max(1),
        }
    }

    fn at_flush_boundary(text: &str) -> bool {
        text.ends_with(char::is_whitespace)
            || text.ends_with('.')
            || text.ends_with('!')
            || text.ends_with('?')
    }

    fn should_flush(buffer: &str, flush_threshold: usize) -> bool {
        buffer.len() >= flush_threshold
            && (Self::at_flush_boundary(buffer) || buffer.len() >= flush_threshold * 2)
    }

    async fn send_payload(&self, payload: String, warning_message: &'static str) {
        if self.sender.send(payload).await.is_err() {
            tracing::warn!("{warning_message}");
        }
    }

    async fn flush_buffer(&self) {
        let payload = self.state.lock().await.flush_all();
        if let Some(payload) = payload {
            self.send_payload(
                payload,
                "stream sink: channel closed, buffered data dropped",
            )
            .await;
        }
    }
}

impl StreamSink for ChannelStreamSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            match event {
                StreamEvent::TextDelta { text } => {
                    let payload = self.state.lock().await.push_delta(
                        text,
                        self.flush_threshold,
                        Self::should_flush,
                    );
                    if let Some(payload) = payload {
                        self.send_payload(
                            payload,
                            "stream sink: channel closed, text delta dropped",
                        )
                        .await;
                    }
                }
                StreamEvent::Done { .. } => {
                    self.flush_buffer().await;
                }
                StreamEvent::ResponseStart { .. }
                | StreamEvent::ToolCallDelta { .. }
                | StreamEvent::ToolCallComplete { .. } => {}
            }
        })
    }
}

/// Stream sink that prints text deltas to stderr for CLI output.
pub struct CliStreamSink {
    writer: Arc<dyn Fn(&str) + Send + Sync>,
    redactor: Mutex<ReasoningStreamRedactor>,
    show_usage: bool,
}

impl CliStreamSink {
    /// Create a CLI sink with reasoning blocks hidden.
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_reasoning(false)
    }

    /// Create a CLI sink with configurable reasoning visibility.
    #[must_use]
    pub fn new_with_reasoning(show_reasoning: bool) -> Self {
        Self::new_with_reasoning_and_usage(show_reasoning, false)
    }

    /// Create a CLI sink with configurable reasoning visibility and
    /// usage footer output.
    #[must_use]
    pub fn new_with_reasoning_and_usage(show_reasoning: bool, show_usage: bool) -> Self {
        Self {
            writer: Arc::new(|text| {
                eprint!("{text}");
            }),
            redactor: Mutex::new(ReasoningStreamRedactor::new(show_reasoning)),
            show_usage,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_writer(writer: Arc<dyn Fn(&str) + Send + Sync>) -> Self {
        Self::with_writer_reasoning_and_usage(writer, false, false)
    }

    #[cfg(test)]
    pub(crate) fn with_writer_reasoning_and_usage(
        writer: Arc<dyn Fn(&str) + Send + Sync>,
        show_reasoning: bool,
        show_usage: bool,
    ) -> Self {
        Self {
            writer,
            redactor: Mutex::new(ReasoningStreamRedactor::new(show_reasoning)),
            show_usage,
        }
    }

    fn usage_footer(input_tokens: Option<u64>, output_tokens: Option<u64>) -> String {
        match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => {
                let total = input.saturating_add(output);
                format!(
                    "\n[usage] input_tokens={input} output_tokens={output} total_tokens={total}\n"
                )
            }
            (Some(input), None) => {
                format!("\n[usage] input_tokens={input} output_tokens=unknown\n")
            }
            (None, Some(output)) => {
                format!("\n[usage] input_tokens=unknown output_tokens={output}\n")
            }
            (None, None) => "\n[usage] input_tokens=unknown output_tokens=unknown\n".to_string(),
        }
    }
}

impl Default for CliStreamSink {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamSink for CliStreamSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            match event {
                StreamEvent::TextDelta { text } => {
                    let visible_text = {
                        let mut redactor = self.redactor.lock().await;
                        redactor.visible_delta(text)
                    };
                    if visible_text.is_empty() {
                        return;
                    }
                    (self.writer)(&visible_text);
                }
                StreamEvent::Done {
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    let tail = {
                        let mut redactor = self.redactor.lock().await;
                        redactor.finish_visible()
                    };
                    if !tail.is_empty() {
                        (self.writer)(&tail);
                    }

                    if self.show_usage {
                        let usage_text = Self::usage_footer(*input_tokens, *output_tokens);
                        (self.writer)(&usage_text);
                    }
                }
                StreamEvent::ResponseStart { .. }
                | StreamEvent::ToolCallDelta { .. }
                | StreamEvent::ToolCallComplete { .. } => {}
            }
        })
    }
}

/// Stream sink that broadcasts events to multiple inner sinks.
pub struct FanoutStreamSink {
    sinks: Vec<Arc<dyn StreamSink>>,
}

impl FanoutStreamSink {
    /// Create a fanout sink wrapping the given sink list.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn StreamSink>>) -> Self {
        Self { sinks }
    }
}

impl StreamSink for FanoutStreamSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            for sink in &self.sinks {
                sink.on_event(event).await;
            }
        })
    }
}

impl ChannelStreamSinkState {
    fn push_delta<F>(
        &mut self,
        delta: &str,
        flush_threshold: usize,
        should_flush: F,
    ) -> Option<String>
    where
        F: Fn(&str, usize) -> bool,
    {
        let visible = self.redactor.visible_delta(delta);
        if visible.is_empty() {
            return None;
        }

        self.buffer.push_str(&visible);
        if self.redactor.should_hold_output() {
            return None;
        }
        should_flush(&self.buffer, flush_threshold).then(|| std::mem::take(&mut self.buffer))
    }

    fn flush_all(&mut self) -> Option<String> {
        let tail = self.redactor.finish_visible();
        if !tail.is_empty() {
            self.buffer.push_str(&tail);
        }

        (!self.buffer.is_empty()).then(|| std::mem::take(&mut self.buffer))
    }
}
