use std::fmt::Write as _;

use serde_json::Value;

use super::{EXAMPLE_LIMIT, finalize_compaction, truncate_preview};
use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

#[derive(Debug, Default)]
pub(crate) struct ChannelHistoryFormatter;

impl SemanticFormatter for ChannelHistoryFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(channel_id) = value.get("channel_id").and_then(Value::as_str) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(count) = value.get("count").and_then(Value::as_u64) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(messages) = value.get("messages").and_then(Value::as_array) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let mut compacted = String::from("channel history\n");
        let _ = writeln!(compacted, "channel: {channel_id}");
        let _ = writeln!(compacted, "count: {count}");
        for message in messages.iter().take(EXAMPLE_LIMIT) {
            let Some(id) = message.get("id").and_then(Value::as_str) else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let content = message.get("content").and_then(Value::as_str).unwrap_or("");
            let author = message
                .get("author")
                .and_then(Value::as_object)
                .and_then(|author| author.get("username"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let _ = writeln!(
                compacted,
                "message: {id} [{author}] {}",
                truncate_preview(content.trim(), 120)
            );
        }

        finalize_compaction(raw, compacted.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ChannelHistoryFormatter;
    use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
    use crate::core::tools::traits::ToolResultSemanticMetadata;

    #[test]
    fn compacts_channel_history_json() {
        let formatter = ChannelHistoryFormatter;
        let raw = serde_json::to_string(&json!({
            "channel_id": "channel-1",
            "count": 8,
            "messages": [
                {
                    "id": "message-1",
                    "content": format!("hello {}", "world ".repeat(200)),
                    "author": {"username": "alice"}
                },
                {
                    "id": "message-2",
                    "content": format!("second {}", "payload ".repeat(200)),
                    "author": {"username": "bob"}
                }
            ]
        }))
        .unwrap();

        let outcome = formatter.compact(&raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted { content, .. } => {
                assert!(content.contains("channel history"));
                assert!(content.contains("channel: channel-1"));
            }
            other => panic!("expected compacted outcome, got {other:?}"),
        }
    }
}
