//! Channel interaction tools — rich messaging primitives for platform channels.
//!
//! # Overview
//!
//! This submodule provides five tools that let the agent interact with a
//! messaging channel (e.g., `Discord`) at runtime. All tools delegate to the
//! `ChannelActionBroker` trait injected via `ExecutionContext`; without a
//! broker they return a non-success `ToolResult` describing the missing
//! dependency rather than panicking.
//!
//! | Tool | Purpose |
//! |------|---------|
//! | [`ChannelCreateThreadTool`] | Start a new thread from the current channel or a specific message. |
//! | [`ChannelAddReactionTool`] | Add an emoji reaction to a message. |
//! | [`ChannelSendRichTool`] | Send a message with optional buttons and an inline embed. |
//! | [`ChannelGetHistoryTool`] | Retrieve recent message history from a channel. |
//! | [`ChannelSendEmbedTool`] | Send a standalone rich-card (embed) message. |
//!
//! # Middleware integration
//!
//! The channel ID used for every operation is resolved from
//! `ctx.source_channel_id` first, then from an optional `channel_id`
//! parameter in the tool arguments. This lets the agent operate on the current
//! conversation channel by default and override it only when needed.
//!
//! Tool-local policy checks are intentionally minimal. The shared middleware
//! classifies broker calls as network-boundary tools, and classifies
//! side-effecting broker calls as external actions before the broker executes;
//! broker implementations and platform permissions remain the final adapter
//! boundary.

mod actions;
mod embed;
mod history;
mod reaction;
mod rich_message;
mod thread;

pub use actions::ChannelActionBroker;
pub use embed::ChannelSendEmbedTool;
pub use history::ChannelGetHistoryTool;
pub use reaction::ChannelAddReactionTool;
pub use rich_message::ChannelSendRichTool;
pub use thread::ChannelCreateThreadTool;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::{
        ChannelAddReactionTool, ChannelCreateThreadTool, ChannelGetHistoryTool,
        ChannelSendEmbedTool, ChannelSendRichTool,
    };
    use crate::core::tools::middleware::ExecutionContext;
    use crate::core::tools::traits::Tool;
    use crate::security::SecurityPolicy;

    fn ctx_without_broker() -> ExecutionContext {
        let mut ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        ctx.source_channel_id = Some("channel-1".to_string());
        ctx
    }

    #[test]
    fn channel_tool_schemas_are_valid_json() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(ChannelCreateThreadTool::new()),
            Box::new(ChannelAddReactionTool::new()),
            Box::new(ChannelSendRichTool::new()),
            Box::new(ChannelGetHistoryTool::new()),
            Box::new(ChannelSendEmbedTool::new()),
        ];

        for tool in tools {
            let schema = tool.parameters_schema();
            assert!(schema.is_object());
            assert!(serde_json::to_string(&schema).is_ok());
        }
    }

    #[tokio::test]
    async fn channel_tools_fail_without_broker() {
        let ctx = ctx_without_broker();
        let cases: Vec<(Box<dyn Tool>, serde_json::Value)> = vec![
            (
                Box::new(ChannelCreateThreadTool::new()),
                json!({ "name": "triage" }),
            ),
            (
                Box::new(ChannelAddReactionTool::new()),
                json!({ "message_id": "m1", "emoji": ":white_check_mark:" }),
            ),
            (
                Box::new(ChannelSendRichTool::new()),
                json!({ "content": "hello" }),
            ),
            (
                Box::new(ChannelGetHistoryTool::new()),
                json!({ "limit": 5 }),
            ),
            (
                Box::new(ChannelSendEmbedTool::new()),
                json!({ "title": "T", "description": "D" }),
            ),
        ];

        for (tool, args) in cases {
            let result = tool.execute(args, &ctx).await.unwrap();
            assert!(!result.success);
            assert!(
                result
                    .error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("broker")
            );
        }
    }
}
