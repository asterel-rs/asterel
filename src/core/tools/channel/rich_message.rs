//! Channel send-rich tool — sends a message with optional buttons and an inline embed.
//!
//! `channel_send_rich` composes a Discord-style interactive message payload
//! from three optional building blocks:
//!
//! * **content** (required) — the plain-text message body.
//! * **buttons** — an array of `{ label, custom_id }` button objects assembled
//!   into a Discord `ActionRow` component (type 1 with type-2 buttons).
//! * **embed** — an inline rich card (`title`, `description`, optional `color`)
//!   appended to the message payload.
//!
//! The composed payload is forwarded to `ChannelActionBroker::send_with_components`
//! and the platform message ID is extracted from the response.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool for sending rich messages with interactive components.
pub struct ChannelSendRichTool;

impl ChannelSendRichTool {
    /// Create a new send-rich-message tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ChannelSendRichTool {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_channel_id<'a>(args: &'a Value, ctx: &'a ExecutionContext) -> anyhow::Result<&'a str> {
    ctx.source_channel_id
        .as_deref()
        .or_else(|| args.get("channel_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing channel id in context and args"))
}

fn resolve_content(args: &Value) -> anyhow::Result<&str> {
    args.get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))
}

fn build_button_components(args: &Value) -> anyhow::Result<Option<Value>> {
    let Some(buttons) = args.get("buttons").and_then(Value::as_array) else {
        return Ok(None);
    };
    if buttons.is_empty() {
        return Ok(None);
    }

    let components = buttons
        .iter()
        .map(|button| {
            let label = button
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Each button requires non-empty 'label'"))?;
            let custom_id = button
                .get("custom_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Each button requires non-empty 'custom_id'"))?;
            Ok(json!({
                "type": 2,
                "style": 1,
                "label": label,
                "custom_id": custom_id
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(json!([{ "type": 1, "components": components }])))
}

fn build_embed(args: &Value) -> anyhow::Result<Option<Value>> {
    let Some(embed_input) = args.get("embed") else {
        return Ok(None);
    };
    if !embed_input.is_object() {
        return Ok(None);
    }

    let title = embed_input
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Embed requires non-empty 'title'"))?;
    let description = embed_input
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Embed requires non-empty 'description'"))?;
    let color = embed_input
        .get("color")
        .and_then(Value::as_u64)
        .map(u32::try_from)
        .transpose()
        .map_err(|_| anyhow::anyhow!("Embed color must fit in u32"))?;

    let mut embed = json!({
        "title": title,
        "description": description
    });
    if let Some(embed_color) = color {
        embed["color"] = json!(embed_color);
    }
    Ok(Some(embed))
}

fn build_payload(args: &Value) -> anyhow::Result<Value> {
    let mut payload = json!({});
    if let Some(components) = build_button_components(args)? {
        payload["components"] = components;
    }
    if let Some(embed) = build_embed(args)? {
        payload["embeds"] = json!([embed]);
    }
    Ok(payload)
}

impl Tool for ChannelSendRichTool {
    fn name(&self) -> &'static str {
        "channel_send_rich"
    }

    fn description(&self) -> &'static str {
        "Send a rich channel message with optional buttons and embed"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Main message content" },
                "buttons": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "custom_id": { "type": "string" }
                        },
                        "required": ["label", "custom_id"]
                    }
                },
                "embed": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "color": { "type": "integer" }
                    },
                    "required": ["title", "description"]
                },
                "channel_id": { "type": "string", "description": "Optional explicit channel id" }
            },
            "required": ["content"]
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

            let channel_id = resolve_channel_id(&args, ctx)?;
            let content = resolve_content(&args)?;
            let payload = build_payload(&args)?;

            let response = broker
                .send_with_components(channel_id, content, payload)
                .await?;
            let message_id = response
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            Ok(ToolResult {
                success: true,
                output: format!(
                    "Sent rich message to channel {channel_id} (message_id: {message_id})"
                ),
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
