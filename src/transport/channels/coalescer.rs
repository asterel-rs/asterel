//! Message coalescer: merges rapid-fire messages from the same sender
//! within a configurable window before dispatching them as a single event.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::Receiver;

use super::traits::{ChannelEvent, ChannelMessage};
use crate::config::ChannelsConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoalescingKey {
    channel: String,
    sender: String,
    conversation_id: Option<String>,
    thread_id: Option<String>,
    reply_to: Option<String>,
}

impl CoalescingKey {
    fn from_message(message: &ChannelMessage) -> Self {
        Self {
            channel: message.channel.clone(),
            sender: message.sender.clone(),
            conversation_id: message.conversation_id.clone(),
            thread_id: message.thread_id.clone(),
            reply_to: message.reply_to.clone(),
        }
    }
}

/// Merges rapid-fire messages from the same sender within a time window.
pub(super) struct MessageCoalescer {
    pending: VecDeque<ChannelEvent>,
    window: Duration,
    max_messages: usize,
}

impl MessageCoalescer {
    /// Creates a coalescer from the channels configuration (window and
    /// max messages).
    pub(super) fn from_config(config: &ChannelsConfig) -> Self {
        let max_messages = config.coalescing_max_messages.max(1);
        Self {
            pending: VecDeque::new(),
            window: Duration::from_millis(config.coalescing_window_ms),
            max_messages,
        }
    }

    /// Returns the next coalesced event, merging consecutive messages
    /// from the same sender within the configured window.
    pub(super) async fn next_event(
        &mut self,
        rx: &mut Receiver<ChannelEvent>,
    ) -> Option<ChannelEvent> {
        let base = match self.pending.pop_front() {
            Some(event) => event,
            None => rx.recv().await?,
        };

        let ChannelEvent::Message(base_msg) = base else {
            return Some(base);
        };

        if self.window.is_zero() || self.max_messages <= 1 || base_msg.channel == "cli" {
            return Some(ChannelEvent::Message(base_msg));
        }

        Some(ChannelEvent::Message(
            self.collect_within_window(base_msg, rx).await,
        ))
    }

    async fn collect_within_window(
        &mut self,
        base: ChannelMessage,
        rx: &mut Receiver<ChannelEvent>,
    ) -> ChannelMessage {
        let deadline = Instant::now() + self.window;
        let key = CoalescingKey::from_message(&base);
        let mut merged = base;
        let mut merged_count = 1usize;

        while merged_count < self.max_messages {
            let next_candidate = if let Some(event) = self.pending.pop_front() {
                Some(event)
            } else {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    None
                } else {
                    tokio::time::timeout(remaining, rx.recv())
                        .await
                        .unwrap_or_default()
                }
            };

            let Some(next_event) = next_candidate else {
                break;
            };

            let next_message = match next_event {
                ChannelEvent::Message(message) => message,
                other => {
                    self.pending.push_front(other);
                    break;
                }
            };

            if CoalescingKey::from_message(&next_message) == key {
                merge_message(&mut merged, next_message);
                merged_count += 1;
                continue;
            }

            self.pending.push_front(ChannelEvent::Message(next_message));
            break;
        }

        merged
    }
}

fn merge_message(base: &mut ChannelMessage, incoming: ChannelMessage) {
    if !incoming.content.trim().is_empty() {
        if !base.content.trim().is_empty() {
            base.content.push('\n');
        }
        base.content.push_str(incoming.content.trim());
    }
    base.timestamp = base.timestamp.max(incoming.timestamp);
    base.attachments.extend(incoming.attachments);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::MessageCoalescer;
    use crate::config::ChannelsConfig;
    use crate::contracts::ids::{ChannelId, UserId};
    use crate::transport::channels::traits::{ChannelEvent, ChannelMessage};

    fn message(id: &str, content: &str, timestamp: u64) -> ChannelMessage {
        ChannelMessage {
            id: id.to_string(),
            sender: "user-1".to_string(),
            content: content.to_string(),
            channel: "discord".to_string(),
            context_hint: None,
            conversation_id: Some("room-1".to_string()),
            thread_id: None,
            reply_to: None,
            message_id: Some(id.to_string()),
            timestamp,
            attachments: Vec::new(),
        }
    }

    fn unwrap_message(event: ChannelEvent) -> ChannelMessage {
        let ChannelEvent::Message(message) = event else {
            panic!("expected Message event");
        };
        message
    }

    #[tokio::test]
    async fn coalescer_merges_matching_messages_within_window() {
        let config = ChannelsConfig {
            coalescing_window_ms: 250,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        coalescer
            .pending
            .push_back(ChannelEvent::Message(message("m1", "hello", 1)));
        coalescer
            .pending
            .push_back(ChannelEvent::Message(message("m2", "there", 2)));
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        drop(tx);

        let merged = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("merged message should exist"),
        );

        assert_eq!(merged.content, "hello\nthere");
        assert_eq!(merged.timestamp, 2);
    }

    #[tokio::test]
    async fn coalescer_preserves_non_matching_message_order() {
        let config = ChannelsConfig {
            coalescing_window_ms: 250,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let _ = tx
            .send(ChannelEvent::Message(message("m1", "first", 1)))
            .await;
        let mut second = message("m2", "second", 2);
        second.sender = "user-2".to_string();
        let _ = tx.send(ChannelEvent::Message(second)).await;

        let first = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first message should exist"),
        );
        assert_eq!(first.content, "first");

        let second_out = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("second message should remain queued"),
        );
        assert_eq!(second_out.content, "second");
        assert_eq!(second_out.sender, "user-2");
    }

    #[tokio::test]
    async fn coalescer_disabled_window_returns_single_message() {
        let mut coalescer = MessageCoalescer::from_config(&ChannelsConfig::default());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let _ = tx
            .send(ChannelEvent::Message(message("m1", "solo", 1)))
            .await;
        let _ = tx
            .send(ChannelEvent::Message(message("m2", "next", 2)))
            .await;

        let first = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first message should exist"),
        );
        assert_eq!(first.content, "solo");

        let second = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("second message should exist"),
        );
        assert_eq!(second.content, "next");
    }

    #[tokio::test]
    async fn coalescer_respects_conversation_boundaries() {
        let config = ChannelsConfig {
            coalescing_window_ms: 250,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let _ = tx
            .send(ChannelEvent::Message(message("m1", "room one", 1)))
            .await;
        let mut different_room = message("m2", "room two", 2);
        different_room.conversation_id = Some("room-2".to_string());
        let _ = tx.send(ChannelEvent::Message(different_room)).await;

        let first = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first message should exist"),
        );
        assert_eq!(first.content, "room one");

        let second = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("second message should exist"),
        );
        assert_eq!(second.content, "room two");
        assert_eq!(second.conversation_id.as_deref(), Some("room-2"));
    }

    #[tokio::test]
    async fn coalescer_stops_at_max_messages() {
        let config = ChannelsConfig {
            coalescing_window_ms: 250,
            coalescing_max_messages: 2,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        coalescer
            .pending
            .push_back(ChannelEvent::Message(message("m1", "one", 1)));
        coalescer
            .pending
            .push_back(ChannelEvent::Message(message("m2", "two", 2)));
        coalescer
            .pending
            .push_back(ChannelEvent::Message(message("m3", "three", 3)));
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        drop(tx);

        let first = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first batch should exist"),
        );
        assert_eq!(first.content, "one\ntwo");

        let second = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("remaining message should stay queued"),
        );
        assert_eq!(second.content, "three");
    }

    #[tokio::test]
    async fn coalescer_skips_cli_channel_merging() {
        let config = ChannelsConfig {
            coalescing_window_ms: 250,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut first = message("m1", "line one", 1);
        first.channel = "cli".to_string();
        let mut second = message("m2", "line two", 2);
        second.channel = "cli".to_string();
        let _ = tx.send(ChannelEvent::Message(first)).await;
        let _ = tx.send(ChannelEvent::Message(second)).await;

        let first_out = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first message should exist"),
        );
        assert_eq!(first_out.content, "line one");

        let second_out = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("second message should exist"),
        );
        assert_eq!(second_out.content, "line two");
    }

    #[tokio::test]
    async fn coalescer_handles_burst_messages_across_multiple_batches() {
        let config = ChannelsConfig {
            coalescing_window_ms: 200,
            coalescing_max_messages: 3,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        drop(tx);

        let mut expected = Vec::new();
        for idx in 0_u64..10 {
            let content = format!("burst-{idx}");
            expected.push(content.clone());
            coalescer.pending.push_back(ChannelEvent::Message(message(
                &format!("m{idx}"),
                &content,
                idx,
            )));
        }

        let mut observed = Vec::new();
        let mut batch_sizes = Vec::new();
        while let Some(out) = coalescer.next_event(&mut rx).await {
            let out = unwrap_message(out);
            let parts: Vec<String> = out.content.lines().map(ToString::to_string).collect();
            batch_sizes.push(parts.len());
            observed.extend(parts);
        }

        assert_eq!(observed, expected);
        assert_eq!(batch_sizes, vec![3, 3, 3, 1]);
    }

    #[tokio::test]
    async fn coalescer_window_expiry_keeps_late_message_separate() {
        let config = ChannelsConfig {
            coalescing_window_ms: 25,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let _ = tx
            .send(ChannelEvent::Message(message("m1", "first", 1)))
            .await;

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx
                .send(ChannelEvent::Message(message("m2", "late", 2)))
                .await;
        });

        let first = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first message should exist"),
        );
        let second = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("late message should remain separate"),
        );

        assert_eq!(first.content, "first");
        assert_eq!(second.content, "late");
    }

    #[tokio::test]
    async fn coalescer_property_preserves_all_input_content_in_order() {
        for case_len in 1_u64..=16 {
            let config = ChannelsConfig {
                coalescing_window_ms: 120,
                coalescing_max_messages: 5,
                ..ChannelsConfig::default()
            };
            let mut coalescer = MessageCoalescer::from_config(&config);
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);

            let mut expected = Vec::new();
            for idx in 0_u64..case_len {
                let content = format!("case-{case_len}-msg-{idx}");
                expected.push(content.clone());
                let _ = tx
                    .send(ChannelEvent::Message(message(
                        &format!("m{case_len}-{idx}"),
                        &content,
                        idx,
                    )))
                    .await;
            }
            drop(tx);

            let mut observed = Vec::new();
            while let Some(out) = coalescer.next_event(&mut rx).await {
                let out = unwrap_message(out);
                observed.extend(out.content.lines().map(ToString::to_string));
            }

            assert_eq!(observed, expected, "case_len={case_len}");
        }
    }

    #[tokio::test]
    async fn coalescer_returns_none_for_empty_closed_channel() {
        let config = ChannelsConfig {
            coalescing_window_ms: 200,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        drop(tx);

        assert!(coalescer.next_event(&mut rx).await.is_none());
    }

    #[tokio::test]
    async fn coalescer_single_message_round_trip() {
        let config = ChannelsConfig {
            coalescing_window_ms: 200,
            coalescing_max_messages: 8,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);

        let _ = tx
            .send(ChannelEvent::Message(message("m1", "only", 42)))
            .await;
        drop(tx);

        let out = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("single message should exist"),
        );
        assert_eq!(out.content, "only");
        assert_eq!(out.timestamp, 42);
        assert!(coalescer.next_event(&mut rx).await.is_none());
    }

    #[tokio::test]
    async fn coalescer_clamps_zero_max_messages_to_one() {
        let config = ChannelsConfig {
            coalescing_window_ms: 200,
            coalescing_max_messages: 0,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let _ = tx
            .send(ChannelEvent::Message(message("m1", "first", 1)))
            .await;
        let _ = tx
            .send(ChannelEvent::Message(message("m2", "second", 2)))
            .await;

        let first = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("first message should exist"),
        );
        let second = unwrap_message(
            coalescer
                .next_event(&mut rx)
                .await
                .expect("second message should exist"),
        );

        assert_eq!(first.content, "first");
        assert_eq!(second.content, "second");
    }

    #[tokio::test]
    async fn coalescer_passes_through_non_message_events_immediately() {
        let config = ChannelsConfig {
            coalescing_window_ms: 250,
            coalescing_max_messages: 4,
            ..ChannelsConfig::default()
        };
        let mut coalescer = MessageCoalescer::from_config(&config);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let _ = tx
            .send(ChannelEvent::TypingStart {
                channel_name: "discord".to_string(),
                channel_id: ChannelId::new("room-1"),
                user_id: UserId::new("user-1"),
            })
            .await;
        let _ = tx
            .send(ChannelEvent::Message(message("m1", "after", 1)))
            .await;

        let first = coalescer
            .next_event(&mut rx)
            .await
            .expect("first event should exist");
        assert!(matches!(first, ChannelEvent::TypingStart { .. }));

        let second = coalescer
            .next_event(&mut rx)
            .await
            .expect("second event should exist");
        let second = unwrap_message(second);
        assert_eq!(second.content, "after");
    }
}
