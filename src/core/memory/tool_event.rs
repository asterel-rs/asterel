//! Tool execution event recording for the memory event ledger.
//!
//! Records successful tool executions as `MemoryEventType::ToolExecuted`
//! events, enabling a unified timeline of agent actions.

use serde::{Deserialize, Serialize};

use crate::core::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemorySource, PrivacyLevel, SourceKind,
};
use crate::utils::text::truncate_ellipsis;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolExecutionSummary {
    tool_name: String,
    arguments_summary: String,
    result_summary: String,
    duration_ms: u64,
}

fn truncate_summary(input: &str, max_chars: usize) -> String {
    if input.len() <= max_chars || input.chars().count() <= max_chars {
        return input.to_string();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    truncate_ellipsis(input, max_chars - 3)
}

#[must_use]
pub fn build_tool_execution_event(
    entity_id: &str,
    tool_name: &str,
    arguments_summary: &str,
    result_summary: &str,
    duration_ms: u64,
    session_id: &str,
) -> MemoryEventInput {
    let summary = ToolExecutionSummary {
        tool_name: tool_name.to_string(),
        arguments_summary: truncate_summary(arguments_summary, 200),
        result_summary: truncate_summary(result_summary, 500),
        duration_ms,
    };

    let value = serde_json::to_string(&summary).unwrap_or_else(|error| {
        serde_json::json!({
            "tool_name": tool_name,
            "arguments_summary": truncate_summary(arguments_summary, 200),
            "result_summary": format!(
                "serialization_error:{}",
                truncate_summary(&error.to_string(), 200)
            ),
            "duration_ms": duration_ms,
        })
        .to_string()
    });

    MemoryEventInput::new(
        entity_id,
        format!("tool.execution.{tool_name}"),
        MemoryEventType::ToolExecuted,
        value,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Episodic)
    .with_importance(0.3)
    .with_source_kind(SourceKind::Api)
    .with_source_ref(format!("tool.{tool_name}.{session_id}"))
}

#[must_use]
pub fn tool_execution_idempotency_key(
    tool_name: &str,
    arguments_hash: &str,
    session_id: &str,
) -> String {
    format!("tool_exec:{tool_name}:{arguments_hash}:{session_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tool_event_sets_correct_slot_key() {
        let arguments = "query=r".repeat(10);
        let event =
            build_tool_execution_event("agent-1", "web_search", &arguments, "ok", 42, "session-1");

        assert!(event.slot_key.as_str().starts_with("tool.execution."));
    }

    #[test]
    fn build_tool_event_uses_episodic_layer() {
        let event = build_tool_execution_event(
            "agent-1",
            "web_search",
            "query=rust",
            "ok",
            42,
            "session-1",
        );

        assert_eq!(event.layer, MemoryLayer::Episodic);
    }

    #[test]
    fn build_tool_event_truncates_long_summaries() {
        let long_arguments = "a".repeat(250);
        let event = build_tool_execution_event(
            "agent-1",
            "web_search",
            &long_arguments,
            "ok",
            42,
            "session-1",
        );

        let parsed: serde_json::Value = serde_json::from_str(&event.value).unwrap();
        let arguments = parsed["arguments_summary"].as_str().unwrap();
        assert!(arguments.len() <= 200);
        assert_ne!(arguments, long_arguments);
    }

    #[test]
    fn idempotency_key_is_deterministic() {
        let left = tool_execution_idempotency_key("web_search", "abc123", "session-1");
        let right = tool_execution_idempotency_key("web_search", "abc123", "session-1");

        assert_eq!(left, right);
    }

    #[test]
    fn tool_event_value_deserializes_to_summary() {
        let event = build_tool_execution_event(
            "agent-1",
            "web_search",
            "query=rust",
            "found docs",
            42,
            "session-1",
        );

        let parsed: ToolExecutionSummary = serde_json::from_str(&event.value).unwrap();
        assert_eq!(parsed.tool_name, "web_search");
        assert_eq!(parsed.arguments_summary, "query=rust");
        assert_eq!(parsed.result_summary, "found docs");
        assert_eq!(parsed.duration_ms, 42);
    }
}
