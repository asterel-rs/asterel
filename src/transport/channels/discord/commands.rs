//! Discord application command definitions (slash commands and context
//! menus) and interaction response helpers.
use anyhow::Result;
use serde_json::json;

use super::http_client::DiscordHttpClient;
use super::types::{ApplicationCommandType, InteractionCallbackType, InteractionType};
use crate::contracts::ids::{MessageId, UserId};

/// Parsed slash commands supported by the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordSlashCommand {
    /// Send a free-form message to the assistant.
    Ask { message: String },
    /// Toggle or adjust thinking level/visibility.
    Think { setting: Option<String> },
}

/// Parsed context menu commands (right-click actions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordContextMenuCommand {
    /// Summarize the targeted message.
    SummarizeMessage { message_id: MessageId },
    /// Ask the assistant about the targeted user.
    AskAboutUser { user_id: UserId },
}

/// Build the default set of application commands to register.
#[must_use]
pub fn build_default_commands() -> Vec<serde_json::Value> {
    vec![
        json!({
            "name": "ask",
            "description": "Send a message to the AI assistant",
            "type": 1,
            "options": [
                {
                    "name": "message",
                    "description": "Your message to the assistant",
                    "type": 3,
                    "required": true
                }
            ]
        }),
        json!({
            "name": "think",
            "description": "Control thinking level/visibility (off|low|medium|high|show|hide|status)",
            "type": 1,
            "options": [
                {
                    "name": "setting",
                    "description": "Optional: off|low|medium|high|show|hide|status",
                    "type": 3,
                    "required": false
                }
            ]
        }),
        json!({
            "name": "Summarize",
            "type": 3,
        }),
        json!({
            "name": "Ask About User",
            "type": 2,
        }),
    ]
}

/// Register application commands globally or for a specific guild.
///
/// # Errors
/// Returns an error if command registration through Discord API fails.
pub async fn register_commands(
    http: &DiscordHttpClient,
    application_id: &str,
    guild_id: Option<&str>,
    commands: &[serde_json::Value],
) -> Result<()> {
    http.register_commands(application_id, guild_id, commands)
        .await
}

/// Extract a slash command from interaction data, if recognized.
#[must_use]
pub fn extract_slash_command(data: &serde_json::Value) -> Option<DiscordSlashCommand> {
    let name = data.get("name")?.as_str()?;
    match name {
        "ask" => data
            .get("options")
            .and_then(|opts| opts.as_array())
            .and_then(|opts| {
                opts.iter().find_map(|opt| {
                    let opt_name = opt.get("name")?.as_str()?;
                    if opt_name == "message" {
                        opt.get("value")?.as_str().map(String::from)
                    } else {
                        None
                    }
                })
            })
            .map(|message| DiscordSlashCommand::Ask { message }),
        "think" => {
            let setting = data
                .get("options")
                .and_then(|opts| opts.as_array())
                .and_then(|opts| {
                    opts.iter().find_map(|opt| {
                        let opt_name = opt.get("name")?.as_str()?;
                        if opt_name == "setting" {
                            opt.get("value")
                                .and_then(serde_json::Value::as_str)
                                .map(String::from)
                        } else {
                            None
                        }
                    })
                });
            Some(DiscordSlashCommand::Think { setting })
        }
        _ => None,
    }
}

/// Extract a context menu command from interaction data, if recognized.
#[must_use]
pub fn extract_context_menu_command(
    interaction_type: u64,
    data: &serde_json::Value,
) -> Option<DiscordContextMenuCommand> {
    if interaction_type != InteractionType::ApplicationCommand as u64 {
        return None;
    }

    let name = data.get("name")?.as_str()?;
    let command_type = data
        .get("type")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .and_then(ApplicationCommandType::from_u8)?;

    match (command_type, name) {
        (ApplicationCommandType::Message, "Summarize") => {
            let target_id = MessageId::new(data.get("target_id")?.as_str()?);
            Some(DiscordContextMenuCommand::SummarizeMessage {
                message_id: target_id,
            })
        }
        (ApplicationCommandType::User, "Ask About User") => {
            let target_id = UserId::new(data.get("target_id")?.as_str()?);
            Some(DiscordContextMenuCommand::AskAboutUser { user_id: target_id })
        }
        _ => None,
    }
}

/// Extract text input values from a modal submit interaction's data.
///
/// Flattens the nested `components → components → value` structure into a list
/// of `(custom_id, value)` pairs in document order.
#[must_use]
pub fn extract_modal_fields(data: &serde_json::Value) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let Some(rows) = data.get("components").and_then(serde_json::Value::as_array) else {
        return fields;
    };
    for row in rows {
        let Some(inputs) = row.get("components").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for input in inputs {
            let Some(id) = input.get("custom_id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(val) = input.get("value").and_then(serde_json::Value::as_str) else {
                continue;
            };
            fields.push((id.to_string(), val.to_string()));
        }
    }
    fields
}

/// Acknowledge an interaction with a deferred response.
///
/// Pass non-zero `flags` to control response visibility.
/// Use [`crate::transport::channels::discord::types::message_flags::EPHEMERAL`]
/// to make the final response visible only to the invoking user.
/// Pass `0` for a public (default) response.
///
/// # Errors
/// Returns an error if posting the interaction defer callback fails.
pub async fn defer_interaction(
    http: &DiscordHttpClient,
    interaction_id: &str,
    interaction_token: &str,
    flags: u64,
) -> Result<()> {
    let data = if flags != 0 {
        Some(serde_json::json!({ "flags": flags }))
    } else {
        None
    };
    http.create_interaction_response(
        interaction_id,
        interaction_token,
        InteractionCallbackType::DeferredChannelMessageWithSource as u8,
        data,
    )
    .await
}

/// Respond immediately to an interaction with a message (type 4).
///
/// Use when the response is ready within the 3-second deadline so no
/// loading state is shown.  Pass non-zero `flags` for e.g. ephemeral.
///
/// # Errors
/// Returns an error if posting the interaction response callback fails.
pub async fn respond_interaction(
    http: &DiscordHttpClient,
    interaction_id: &str,
    interaction_token: &str,
    content: &str,
    flags: u64,
) -> Result<()> {
    let mut data = serde_json::json!({
        "content": content,
        "allowed_mentions": { "parse": [] },
    });
    if flags != 0 {
        data["flags"] = serde_json::json!(flags);
    }
    http.create_interaction_response(
        interaction_id,
        interaction_token,
        InteractionCallbackType::ChannelMessageWithSource as u8,
        Some(data),
    )
    .await
}

/// Edit the original deferred interaction response with final content.
///
/// # Errors
/// Returns an error if editing the original interaction response fails.
pub async fn send_interaction_followup(
    http: &DiscordHttpClient,
    application_id: &str,
    interaction_token: &str,
    content: &str,
) -> Result<()> {
    http.edit_original_interaction_response(application_id, interaction_token, content)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_commands_include_ask_and_think() {
        let cmds = build_default_commands();
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[0]["name"], "ask");
        assert_eq!(cmds[0]["type"], 1);
        assert_eq!(cmds[1]["name"], "think");
        assert_eq!(cmds[1]["type"], 1);
        assert_eq!(cmds[2]["name"], "Summarize");
        assert_eq!(cmds[2]["type"], 3);
        assert_eq!(cmds[3]["name"], "Ask About User");
        assert_eq!(cmds[3]["type"], 2);
    }

    #[test]
    fn extract_ask_command_input() {
        let data = json!({
            "name": "ask",
            "options": [
                {"name": "message", "type": 3, "value": "Hello AI"}
            ]
        });
        assert_eq!(
            extract_slash_command(&data),
            Some(DiscordSlashCommand::Ask {
                message: "Hello AI".to_string()
            })
        );
    }

    #[test]
    fn extract_think_command_without_setting() {
        let data = json!({
            "name": "think",
            "options": []
        });
        assert_eq!(
            extract_slash_command(&data),
            Some(DiscordSlashCommand::Think { setting: None })
        );
    }

    #[test]
    fn extract_think_command_with_setting() {
        let data = json!({
            "name": "think",
            "options": [
                {"name": "setting", "type": 3, "value": "high"}
            ]
        });
        assert_eq!(
            extract_slash_command(&data),
            Some(DiscordSlashCommand::Think {
                setting: Some("high".to_string())
            })
        );
    }

    #[test]
    fn extract_unknown_command_returns_none() {
        let data = json!({"name": "unknown", "options": []});
        assert_eq!(extract_slash_command(&data), None);
    }

    #[test]
    fn extract_ask_command_missing_message_option() {
        let data = json!({
            "name": "ask",
            "options": [
                {"name": "other", "type": 3, "value": "stuff"}
            ]
        });
        assert_eq!(extract_slash_command(&data), None);
    }

    #[test]
    fn extract_ask_command_empty_options() {
        let data = json!({"name": "ask", "options": []});
        assert_eq!(extract_slash_command(&data), None);
    }

    #[test]
    fn extract_summarize_message_context_menu_command() {
        let data = json!({
            "name": "Summarize",
            "type": 3,
            "target_id": "12345"
        });
        assert_eq!(
            extract_context_menu_command(2, &data),
            Some(DiscordContextMenuCommand::SummarizeMessage {
                message_id: MessageId::new("12345"),
            })
        );
    }

    #[test]
    fn extract_ask_about_user_context_menu_command() {
        let data = json!({
            "name": "Ask About User",
            "type": 2,
            "target_id": "u-777"
        });
        assert_eq!(
            extract_context_menu_command(2, &data),
            Some(DiscordContextMenuCommand::AskAboutUser {
                user_id: UserId::new("u-777"),
            })
        );
    }

    #[test]
    fn extract_context_menu_rejects_wrong_type_or_name() {
        let wrong_interaction = json!({"name": "Summarize", "type": 3, "target_id": "1"});
        assert_eq!(extract_context_menu_command(3, &wrong_interaction), None);

        let wrong_command_type = json!({"name": "Summarize", "type": 2, "target_id": "1"});
        assert_eq!(extract_context_menu_command(2, &wrong_command_type), None);

        let unknown = json!({"name": "Nope", "type": 3, "target_id": "1"});
        assert_eq!(extract_context_menu_command(2, &unknown), None);
    }
}
