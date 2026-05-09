//! Channel get-history tool — retrieves recent messages from a channel.
//!
//! `channel_get_history` fetches up to `limit` (max 100, default 20) recent
//! messages from the current or explicitly specified channel via the
//! `ChannelActionBroker`. An optional `before` message ID can be supplied for
//! cursor-based pagination.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{
    Tool, ToolResult, ToolResultCompactionTarget, ToolResultTextField,
};

/// Tool for fetching recent message history from a channel.
pub struct ChannelGetHistoryTool;

impl ChannelGetHistoryTool {
    /// Create a new get-history tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ChannelGetHistoryTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ChannelGetHistoryTool {
    fn name(&self) -> &'static str {
        "channel_get_history"
    }

    fn description(&self) -> &'static str {
        "Fetch recent message history from the current channel"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of messages",
                    "default": 20,
                    "maximum": 100
                },
                "before": { "type": "string", "description": "Message ID to fetch before" },
                "channel_id": { "type": "string", "description": "Optional explicit channel id" }
            }
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let Some(broker) = ctx.channel_action_broker.as_ref() else {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("channel action broker is not available".to_string()),
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            };

            let channel_id = ctx
                .source_channel_id
                .as_deref()
                .or_else(|| args.get("channel_id").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Missing channel id in context and args"))?;
            let limit = match args.get("limit").and_then(Value::as_u64) {
                Some(value) => {
                    let converted = u32::try_from(value)
                        .map_err(|_| anyhow::anyhow!("'limit' must fit into u32"))?;
                    converted.min(100)
                }
                None => 20,
            };
            let before = args
                .get("before")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());

            let messages = broker.get_messages(channel_id, Some(limit), before).await?;
            let count = messages.as_array().map_or(0, Vec::len);

            let output = serde_json::to_string(&json!({
                "channel_id": channel_id,
                "count": count,
                "messages": messages
            }))?;

            Ok(ToolResult::success(output)
                .with_output_kind("channel.history")
                .with_compaction_target(ToolResultCompactionTarget::Output)
                .with_source_fields([ToolResultTextField::Output]))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use anyhow::Result;
    use serde_json::json;

    use super::*;
    use crate::core::tools::channel::ChannelActionBroker;

    struct FakeBroker;

    impl ChannelActionBroker for FakeBroker {
        fn channel_name(&self) -> &str {
            "fake"
        }

        fn create_thread<'a>(
            &'a self,
            _channel_id: &'a str,
            _name: &'a str,
            _message_id: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
            Box::pin(async move { Ok("thread".to_string()) })
        }

        fn add_reaction<'a>(
            &'a self,
            _channel_id: &'a str,
            _message_id: &'a str,
            _emoji: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }

        fn send_with_components<'a>(
            &'a self,
            _channel_id: &'a str,
            _content: &'a str,
            _components: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send + 'a>> {
            Box::pin(async move { Ok(json!({"id": "message"})) })
        }

        fn get_messages<'a>(
            &'a self,
            _channel_id: &'a str,
            _limit: Option<u32>,
            _before: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send + 'a>> {
            Box::pin(async move {
                Ok(json!([
                    {
                        "id": "message-1",
                        "content": "hello world",
                        "timestamp": "2026-04-09T12:00:00Z",
                        "author": {
                            "username": "alice",
                            "id": "user-1"
                        }
                    }
                ]))
            })
        }

        fn send_embed<'a>(
            &'a self,
            _channel_id: &'a str,
            _title: &'a str,
            _description: &'a str,
            _color: Option<u32>,
        ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
            Box::pin(async move { Ok("embed".to_string()) })
        }
    }

    #[tokio::test]
    async fn channel_get_history_emits_semantic_metadata_on_success() {
        let mut ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        ctx.source_channel_id = Some("channel-1".to_string());
        ctx.channel_action_broker = Some(Arc::new(FakeBroker));

        let tool = ChannelGetHistoryTool::new();
        let result = tool.execute(json!({}), &ctx).await.unwrap();

        assert!(result.success);
        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("channel.history")
        );
        assert_eq!(
            result.semantic.compaction_target,
            ToolResultCompactionTarget::Output
        );
        assert_eq!(
            result
                .semantic
                .source_fields
                .iter()
                .map(|field| field.field)
                .collect::<Vec<_>>(),
            vec![ToolResultTextField::Output]
        );
        assert!(result.semantic.stats.is_some());
    }
}
