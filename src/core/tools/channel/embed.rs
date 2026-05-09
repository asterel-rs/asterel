//! Channel send-embed tool — sends a rich card (embed) to a channel.
//!
//! `channel_send_embed` dispatches a single embed (title, description,
//! optional color) to the current or explicitly specified channel via the
//! `ChannelActionBroker`. Returns the platform message ID on success.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool for sending embed (rich card) messages to a channel.
pub struct ChannelSendEmbedTool;

impl ChannelSendEmbedTool {
    /// Create a new send-embed tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ChannelSendEmbedTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ChannelSendEmbedTool {
    fn name(&self) -> &'static str {
        "channel_send_embed"
    }

    fn description(&self) -> &'static str {
        "Send an embed message to the current channel"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Embed title" },
                "description": { "type": "string", "description": "Embed description" },
                "color": { "type": "integer", "description": "Optional embed color" },
                "channel_id": { "type": "string", "description": "Optional explicit channel id" }
            },
            "required": ["title", "description"]
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
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Missing 'title' parameter"))?;
            let description = args
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Missing 'description' parameter"))?;
            let color = args
                .get("color")
                .and_then(Value::as_u64)
                .map(u32::try_from)
                .transpose()
                .map_err(|_| anyhow::anyhow!("'color' must fit into u32"))?;

            let message_id = broker
                .send_embed(channel_id, title, description, color)
                .await?;

            Ok(ToolResult {
                success: true,
                output: format!("Sent embed to channel {channel_id} (message_id: {message_id})"),
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
