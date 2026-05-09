//! Tests for provider streaming sinks and collectors.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{
    ChannelStreamSink, CliStreamSink, FanoutStreamSink, MAX_STREAM_TEXT_BYTES,
    MAX_TOOL_CALL_INPUT_JSON_BYTES, NullStreamSink, StreamCollector, StreamEvent, StreamSink,
    StreamingSecretScrubber,
};
use crate::core::providers::response::{ContentBlock, ProviderResponse, StopReason};
use crate::core::providers::streaming::resp_to_events;

fn text_delta(text: &str) -> StreamEvent {
    StreamEvent::TextDelta {
        text: text.to_string(),
    }
}

fn done_event() -> StreamEvent {
    StreamEvent::Done {
        stop_reason: None,
        input_tokens: None,
        output_tokens: None,
    }
}

fn captured_cli_sink() -> (Arc<Mutex<String>>, CliStreamSink) {
    let captured = Arc::new(Mutex::new(String::new()));
    let captured_clone = Arc::clone(&captured);
    let sink = CliStreamSink::with_writer(Arc::new(move |text| {
        let mut guard = captured_clone
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.push_str(text);
    }));
    (captured, sink)
}

fn captured_text(captured: &Arc<Mutex<String>>) -> String {
    captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn recv_all_available(rx: &mut mpsc::Receiver<String>) -> Vec<String> {
    let mut chunks = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        chunks.push(chunk);
    }
    chunks
}

#[test]
fn stream_event_text_delta_debug() {
    let event = text_delta("hello");
    let debug = format!("{event:?}");
    assert!(debug.contains("TextDelta"));
    assert!(debug.contains("hello"));
}

#[tokio::test]
async fn null_stream_sink_is_noop() {
    let sink = NullStreamSink;
    sink.on_event(&StreamEvent::ResponseStart { model: None })
        .await;
    sink.on_event(&text_delta("x")).await;
    sink.on_event(&done_event()).await;
}

#[tokio::test]
async fn cli_stream_sink_writes_text_delta() {
    let (captured, sink) = captured_cli_sink();
    sink.on_event(&text_delta("hello")).await;

    let output = captured_text(&captured);
    assert_eq!(output, "hello");
}

#[tokio::test]
async fn cli_stream_sink_ignores_non_text_events() {
    let (captured, sink) = captured_cli_sink();

    sink.on_event(&StreamEvent::ResponseStart { model: None })
        .await;
    sink.on_event(&done_event()).await;

    let output = captured_text(&captured);
    assert!(output.is_empty());
}

#[tokio::test]
async fn cli_stream_sink_prints_usage_when_enabled() {
    let captured = Arc::new(Mutex::new(String::new()));
    let captured_clone = Arc::clone(&captured);
    let sink = CliStreamSink::with_writer_reasoning_and_usage(
        Arc::new(move |text| {
            let mut guard = captured_clone
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.push_str(text);
        }),
        false,
        true,
    );

    sink.on_event(&StreamEvent::Done {
        stop_reason: Some(StopReason::EndTurn),
        input_tokens: Some(12),
        output_tokens: Some(5),
    })
    .await;

    let output = captured_text(&captured);
    assert!(output.contains("[usage]"));
    assert!(output.contains("input_tokens=12"));
    assert!(output.contains("output_tokens=5"));
    assert!(output.contains("total_tokens=17"));
}

#[tokio::test]
async fn channel_stream_sink_flushes_at_threshold_with_boundary() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 5);

    sink.on_event(&text_delta("hello ")).await;

    assert_eq!(rx.recv().await, Some("hello ".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_keeps_buffer_without_boundary() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 5);

    sink.on_event(&text_delta("hello")).await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn channel_stream_sink_flushes_on_done() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 80);

    sink.on_event(&text_delta("partial")).await;
    sink.on_event(&done_event()).await;

    assert_eq!(rx.recv().await, Some("partial".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_does_not_flush_empty_on_done() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 10);

    sink.on_event(&done_event()).await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn channel_stream_sink_non_text_event_does_not_flush() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 4);

    sink.on_event(&text_delta("abc")).await;
    sink.on_event(&StreamEvent::ToolCallDelta {
        index: 0,
        id: None,
        name: None,
        input_json_delta: "{".to_string(),
    })
    .await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn channel_stream_sink_flushes_long_chunk_without_boundary() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 5);

    sink.on_event(&text_delta("abcdefghij")).await;

    assert_eq!(rx.recv().await, Some("abcdefghij".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_flushes_when_sentence_ends() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 6);

    sink.on_event(&text_delta("hello.")).await;

    assert_eq!(rx.recv().await, Some("hello.".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_accumulates_multiple_deltas_before_flush() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 8);

    sink.on_event(&text_delta("hel")).await;
    sink.on_event(&text_delta("lo ")).await;
    sink.on_event(&text_delta("world ")).await;

    assert_eq!(rx.recv().await, Some("hello world ".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_emits_multiple_chunks_in_order() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 5);

    sink.on_event(&text_delta("alpha ")).await;
    sink.on_event(&text_delta("beta ")).await;

    assert_eq!(rx.recv().await, Some("alpha ".to_string()));
    assert_eq!(rx.recv().await, Some("beta ".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_ignores_response_start() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 4);

    sink.on_event(&StreamEvent::ResponseStart {
        model: Some("model".to_string()),
    })
    .await;

    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn channel_stream_sink_hides_reasoning_blocks_by_default() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 1);

    sink.on_event(&text_delta("answer <think>secret</think> done"))
        .await;
    sink.on_event(&done_event()).await;

    assert_eq!(rx.recv().await, Some("answer  done".to_string()));
}

#[tokio::test]
async fn channel_stream_sink_can_show_reasoning_when_enabled() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new_with_reasoning(tx, 1, true);

    sink.on_event(&text_delta("answer <think>secret</think> done"))
        .await;
    sink.on_event(&done_event()).await;

    assert_eq!(
        rx.recv().await,
        Some("answer <think>secret</think> done".to_string())
    );
}

#[tokio::test]
async fn channel_stream_sink_hides_reasoning_when_tag_spans_chunks() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 1);

    sink.on_event(&text_delta("answer <thi")).await;
    sink.on_event(&text_delta("nk>secret</thin")).await;
    sink.on_event(&text_delta("k> done")).await;
    sink.on_event(&done_event()).await;

    let first = rx.recv().await.expect("visible output");
    let mut chunks = vec![first];
    chunks.extend(recv_all_available(&mut rx));
    assert_eq!(chunks.concat(), "answer  done");
    assert!(chunks.iter().all(|chunk| !chunk.contains("secret")));
    assert!(chunks.iter().all(|chunk| !chunk.contains("<think")));
}

#[tokio::test]
async fn cli_stream_sink_flushes_pending_tail_when_reasoning_tag_spans_chunks() {
    let (captured, sink) = captured_cli_sink();

    sink.on_event(&text_delta("answer <thi")).await;
    sink.on_event(&text_delta("nk>secret</thin")).await;
    sink.on_event(&text_delta("k> done")).await;
    sink.on_event(&done_event()).await;

    assert_eq!(captured_text(&captured), "answer  done");
}

#[tokio::test]
async fn channel_stream_sink_preserves_reasoning_tags_inside_code_fence() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 1);

    sink.on_event(&text_delta("```xml\n<think>visible</think>\n```"))
        .await;
    sink.on_event(&done_event()).await;

    assert_eq!(
        rx.recv().await,
        Some("```xml\n<think>visible</think>\n```".to_string())
    );
}

#[tokio::test]
async fn channel_stream_sink_preserves_reasoning_tags_inside_inline_code() {
    let (tx, mut rx) = mpsc::channel(16);
    let sink = ChannelStreamSink::new(tx, 1);

    sink.on_event(&text_delta("use `<think>visible</think>` literally"))
        .await;
    sink.on_event(&done_event()).await;

    assert_eq!(
        rx.recv().await,
        Some("use `<think>visible</think>` literally".to_string())
    );
}

#[tokio::test]
async fn fanout_stream_sink_forwards_to_all_sinks() {
    let (primary_sink_output, sink_a) = captured_cli_sink();
    let (secondary_sink_output, sink_b) = captured_cli_sink();
    let sink_a = Arc::new(sink_a) as Arc<dyn StreamSink>;
    let sink_b = Arc::new(sink_b) as Arc<dyn StreamSink>;

    let fanout = FanoutStreamSink::new(vec![sink_a, sink_b]);
    fanout.on_event(&text_delta("hello")).await;

    assert_eq!(captured_text(&primary_sink_output), "hello");
    assert_eq!(captured_text(&secondary_sink_output), "hello");
}

#[test]
fn collector_text_only() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::ResponseStart {
        model: Some("model".to_string()),
    });
    collector.feed(&StreamEvent::TextDelta {
        text: "hello world".to_string(),
    });
    collector.feed(&StreamEvent::Done {
        stop_reason: Some(StopReason::EndTurn),
        input_tokens: Some(10),
        output_tokens: Some(2),
    });

    let response = collector.finish();
    assert_eq!(response.text, "hello world");
    assert_eq!(response.model, Some("model".to_string()));
}

#[test]
fn collector_tool_call_complete() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::ResponseStart {
        model: Some("model".to_string()),
    });
    collector.feed(&StreamEvent::ToolCallComplete {
        id: "call-1".to_string(),
        name: "shell".to_string(),
        input: serde_json::json!({"command": "ls"}),
    });
    collector.feed(&StreamEvent::Done {
        stop_reason: Some(StopReason::ToolUse),
        input_tokens: None,
        output_tokens: None,
    });

    let response = collector.finish();
    assert_eq!(response.content_blocks.len(), 1);
    assert!(matches!(
        response.content_blocks[0],
        ContentBlock::ToolUse { .. }
    ));
}

#[test]
fn collector_tool_call_delta_assembly() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::ResponseStart { model: None });
    collector.feed(&StreamEvent::ToolCallDelta {
        index: 0,
        id: Some("call-1".to_string()),
        name: Some("shell".to_string()),
        input_json_delta: "{\"co".to_string(),
    });
    collector.feed(&StreamEvent::ToolCallDelta {
        index: 0,
        id: None,
        name: None,
        input_json_delta: "mmand\"".to_string(),
    });
    collector.feed(&StreamEvent::ToolCallDelta {
        index: 0,
        id: None,
        name: None,
        input_json_delta: ": \"ls\"}".to_string(),
    });
    collector.feed(&StreamEvent::Done {
        stop_reason: Some(StopReason::ToolUse),
        input_tokens: None,
        output_tokens: None,
    });

    let response = collector.finish();
    assert_eq!(response.content_blocks.len(), 1);
    match &response.content_blocks[0] {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "call-1");
            assert_eq!(name, "shell");
            assert_eq!(input, &serde_json::json!({"command": "ls"}));
        }
        _ => panic!("expected tool use block"),
    }
}

#[test]
fn collector_mixed_text_and_tools() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::ResponseStart { model: None });
    collector.feed(&StreamEvent::TextDelta {
        text: "running".to_string(),
    });
    collector.feed(&StreamEvent::ToolCallComplete {
        id: "call-1".to_string(),
        name: "shell".to_string(),
        input: serde_json::json!({"command": "pwd"}),
    });
    collector.feed(&StreamEvent::Done {
        stop_reason: Some(StopReason::ToolUse),
        input_tokens: Some(1),
        output_tokens: Some(1),
    });

    let response = collector.finish();
    assert_eq!(response.text, "running");
    assert_eq!(response.content_blocks.len(), 2);
    assert!(matches!(
        response.content_blocks[0],
        ContentBlock::Text { .. }
    ));
    assert!(matches!(
        response.content_blocks[1],
        ContentBlock::ToolUse { .. }
    ));
}

#[test]
fn collector_invalid_tool_json_skipped() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::ResponseStart { model: None });
    collector.feed(&StreamEvent::ToolCallDelta {
        index: 0,
        id: Some("call-1".to_string()),
        name: Some("shell".to_string()),
        input_json_delta: "{\"command\": }".to_string(),
    });
    collector.feed(&StreamEvent::Done {
        stop_reason: Some(StopReason::Error),
        input_tokens: None,
        output_tokens: None,
    });

    let response = collector.finish();
    assert!(response.content_blocks.is_empty());
}

#[test]
fn collector_caps_accumulated_text_bytes() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::TextDelta {
        text: "a".repeat(MAX_STREAM_TEXT_BYTES + 128),
    });
    collector.feed(&StreamEvent::TextDelta {
        text: "extra".to_string(),
    });

    let response = collector.finish();
    assert_eq!(response.text.len(), MAX_STREAM_TEXT_BYTES);
    assert!(response.text.chars().all(|ch| ch == 'a'));
}

#[test]
fn collector_caps_streamed_tool_json_per_call() {
    let mut collector = StreamCollector::new();
    collector.feed(&StreamEvent::ToolCallDelta {
        index: 0,
        id: Some("call-1".to_string()),
        name: Some("shell".to_string()),
        input_json_delta: "{\"payload\":\"".to_string(),
    });
    collector.feed(&StreamEvent::ToolCallDelta {
        index: 0,
        id: None,
        name: None,
        input_json_delta: "x".repeat(MAX_TOOL_CALL_INPUT_JSON_BYTES + 128),
    });

    let response = collector.finish();
    assert!(response.content_blocks.is_empty());
}

#[test]
fn resp_to_events_roundtrip() {
    let original = ProviderResponse {
        text: "hello".to_string(),
        input_tokens: Some(10),
        output_tokens: Some(4),
        model: Some("model".to_string()),
        content_blocks: vec![
            ContentBlock::Text {
                text: "hello".to_string(),
            },
            ContentBlock::ToolUse {
                id: "call-1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"command": "ls"}),
            },
        ],
        stop_reason: Some(StopReason::ToolUse),
        logprobs: None,
    };

    let events = resp_to_events(original.clone());
    let mut collector = StreamCollector::new();
    for event in events {
        collector.feed(&event.expect("event should be ok"));
    }

    let reconstructed = collector.finish();
    assert_eq!(reconstructed.text, original.text);
    assert_eq!(reconstructed.model, original.model);
    assert_eq!(reconstructed.stop_reason, original.stop_reason);
    assert_eq!(reconstructed.input_tokens, original.input_tokens);
    assert_eq!(reconstructed.output_tokens, original.output_tokens);
    assert_eq!(
        reconstructed.content_blocks.len(),
        original.content_blocks.len()
    );
    match (
        &reconstructed.content_blocks[0],
        &original.content_blocks[0],
    ) {
        (ContentBlock::Text { text: left }, ContentBlock::Text { text: right }) => {
            assert_eq!(left, right);
        }
        _ => panic!("expected first content block to be text"),
    }
    match (
        &reconstructed.content_blocks[1],
        &original.content_blocks[1],
    ) {
        (
            ContentBlock::ToolUse {
                id: left_id,
                name: left_name,
                input: left_input,
            },
            ContentBlock::ToolUse {
                id: right_id,
                name: right_name,
                input: right_input,
            },
        ) => {
            assert_eq!(left_id, right_id);
            assert_eq!(left_name, right_name);
            assert_eq!(left_input, right_input);
        }
        _ => panic!("expected second content block to be tool_use"),
    }
}

#[test]
fn scrubber_passes_clean_text() {
    let mut scrubber = StreamingSecretScrubber::new(64);
    let first = scrubber.scrub_delta("hello world");
    let rest = scrubber.finish();
    assert_eq!(format!("{first}{rest}"), "hello world");
}

#[test]
fn scrubber_redacts_secret() {
    let mut scrubber = StreamingSecretScrubber::new(64);
    let mut output = scrubber.scrub_delta("key is sk-abc123def456");
    output.push_str(&scrubber.finish());
    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains("sk-abc123def456"));
}

#[test]
fn scrubber_finish_flushes_carry() {
    let mut scrubber = StreamingSecretScrubber::new(64);
    let prefix = scrubber.scrub_delta("partial");
    let suffix = scrubber.finish();
    assert_eq!(format!("{prefix}{suffix}"), "partial");
}

#[test]
fn scrubber_split_across_chunks() {
    let mut scrubber = StreamingSecretScrubber::new(64);
    let mut output = scrubber.scrub_delta("key is sk-");
    output.push_str(&scrubber.scrub_delta("abc123def456 ok"));
    output.push_str(&scrubber.finish());

    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains("sk-abc123def456"));
}
