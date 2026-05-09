//! Channel add-reaction tool — adds an emoji reaction to a channel message.
//!
//! `channel_add_reaction` adds a Unicode emoji or custom emoji identifier to a
//! specific message via the `ChannelActionBroker`. Both `message_id` and
//! `emoji` are required; the channel ID is resolved from the execution context
//! or the optional `channel_id` argument.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool for adding emoji reactions to channel messages.
pub struct ChannelAddReactionTool;

impl ChannelAddReactionTool {
    /// Create a new add-reaction tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ChannelAddReactionTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ChannelAddReactionTool {
    fn name(&self) -> &'static str {
        "channel_add_reaction"
    }

    fn description(&self) -> &'static str {
        "Add a reaction emoji to a message in the current channel"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message_id": { "type": "string", "description": "Message id to react to" },
                "emoji": { "type": "string", "description": "Unicode emoji or custom emoji name" },
                "channel_id": { "type": "string", "description": "Optional explicit channel id" }
            },
            "required": ["message_id", "emoji"]
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
            let message_id = args
                .get("message_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Missing 'message_id' parameter"))?;
            let emoji = args
                .get("emoji")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Missing 'emoji' parameter"))?;

            broker.add_reaction(channel_id, message_id, emoji).await?;
            Ok(ToolResult {
                success: true,
                output: format!("Added reaction '{emoji}' to message {message_id}"),
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
