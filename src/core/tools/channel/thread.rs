//! Channel create-thread tool — creates a new thread in a channel.
//!
//! `channel_create_thread` starts a threaded conversation via the
//! `ChannelActionBroker`. The thread can be rooted on the current channel
//! (standalone thread) or forked from a specific message by supplying
//! `message_id`. The platform-assigned thread ID is returned on success.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool for creating threaded conversations in a channel.
pub struct ChannelCreateThreadTool;

impl ChannelCreateThreadTool {
    /// Create a new create-thread tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ChannelCreateThreadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ChannelCreateThreadTool {
    fn name(&self) -> &'static str {
        "channel_create_thread"
    }

    fn description(&self) -> &'static str {
        "Create a channel thread from the current channel or a specified message"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Thread name" },
                "message_id": { "type": "string", "description": "Create thread from this message" },
                "channel_id": { "type": "string", "description": "Optional explicit channel id" }
            },
            "required": ["name"]
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

            let name = args
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

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
                .filter(|value| !value.is_empty());

            let thread_id = broker.create_thread(channel_id, name, message_id).await?;
            Ok(ToolResult {
                success: true,
                output: format!("Created thread '{name}' ({thread_id})"),
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
