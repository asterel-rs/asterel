use uuid::Uuid;

use super::channel::DiscordChannel;
use super::commands::DiscordContextMenuCommand;
use super::types::{InteractionCallbackType, InteractionType, message_flags};
use crate::transport::channels::traits::{ChannelEvent, ChannelMessage};

const DISCORD_EPHEMERAL_CONTEXT_HINT: &str = "[Channel Context: Ephemeral interaction — user-only delivery; source may be public, keep personal details minimal]";

fn direct_interaction_context_hint(is_dm: bool) -> Option<String> {
    super::addressability::channel_context_hint(
        super::addressability::AddressabilityMode::Direct,
        is_dm,
    )
    .map(ToString::to_string)
}

fn interaction_defer_flags(context_hint: Option<&str>) -> u64 {
    if context_hint.is_some_and(|hint| hint.contains("Ephemeral interaction")) {
        message_flags::EPHEMERAL
    } else {
        0
    }
}

pub(super) struct InteractionCreateParams<'a> {
    pub(super) tx: &'a tokio::sync::mpsc::Sender<ChannelEvent>,
    pub(super) interaction_id: &'a str,
    pub(super) interaction_token: &'a str,
    pub(super) interaction_type: u64,
    pub(super) channel_id: &'a str,
    pub(super) user_id: &'a str,
    pub(super) guild_id: Option<&'a str>,
    pub(super) data: &'a serde_json::Value,
}

impl DiscordChannel {
    pub(super) async fn handle_interaction_create(&self, params: InteractionCreateParams<'_>) {
        let InteractionCreateParams {
            tx,
            interaction_id,
            interaction_token,
            interaction_type,
            channel_id,
            user_id,
            guild_id,
            data,
        } = params;

        let Some(itype) = InteractionType::from_u64(interaction_type) else {
            return;
        };
        if !self.is_user_allowed(user_id) {
            return;
        }
        if !self.matches_guild_filter(guild_id) {
            return;
        }

        match itype {
            InteractionType::ApplicationCommand => {
                self.handle_application_command_interaction(
                    tx,
                    interaction_id,
                    interaction_token,
                    channel_id,
                    user_id,
                    guild_id.is_none(),
                    data,
                )
                .await;
            }
            InteractionType::MessageComponent => {
                self.handle_message_component_interaction(
                    interaction_id,
                    interaction_token,
                    user_id,
                    data,
                )
                .await;
            }
            InteractionType::ModalSubmit => {
                self.handle_modal_submit_interaction(
                    tx,
                    interaction_id,
                    interaction_token,
                    channel_id,
                    user_id,
                    guild_id.is_none(),
                    data,
                )
                .await;
            }
            InteractionType::ApplicationCommandAutocomplete => {
                self.handle_autocomplete_interaction(
                    interaction_id,
                    interaction_token,
                    user_id,
                    data,
                )
                .await;
            }
            InteractionType::Ping => {}
        }
    }

    async fn handle_application_command_interaction(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        interaction_id: &str,
        interaction_token: &str,
        channel_id: &str,
        user_id: &str,
        is_dm: bool,
        data: &serde_json::Value,
    ) {
        if let Some(command) = super::commands::extract_slash_command(data) {
            let input = Self::slash_command_input(command);
            let context_hint = direct_interaction_context_hint(is_dm);
            if let Err(e) = super::commands::defer_interaction(
                &self.http,
                interaction_id,
                interaction_token,
                interaction_defer_flags(context_hint.as_deref()),
            )
            .await
            {
                tracing::warn!("Discord: failed to defer interaction: {e}");
                return;
            }
            self.send_channel_message(
                tx,
                interaction_id,
                channel_id,
                Some(interaction_token),
                user_id,
                input,
                context_hint,
            )
            .await;
        } else if let Some(command) = super::commands::extract_context_menu_command(
            InteractionType::ApplicationCommand as u64,
            data,
        ) {
            self.handle_context_menu_command(
                tx,
                interaction_id,
                interaction_token,
                channel_id,
                user_id,
                command,
            )
            .await;
        }
    }

    async fn handle_context_menu_command(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        interaction_id: &str,
        interaction_token: &str,
        channel_id: &str,
        user_id: &str,
        command: DiscordContextMenuCommand,
    ) {
        let context_hint = Some(DISCORD_EPHEMERAL_CONTEXT_HINT.to_string());
        if let Err(e) = super::commands::defer_interaction(
            &self.http,
            interaction_id,
            interaction_token,
            interaction_defer_flags(context_hint.as_deref()),
        )
        .await
        {
            tracing::warn!("Discord: failed to defer context menu interaction: {e}");
            return;
        }

        let (content, command_context_hint) = match command {
            DiscordContextMenuCommand::SummarizeMessage { message_id } => {
                match self.http.get_message(channel_id, message_id.as_str()).await {
                    Ok(msg) => {
                        let text = msg
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        if text.is_empty() {
                            tracing::warn!(
                                message_id = %message_id.as_str(),
                                "Discord: SummarizeMessage target has no text content"
                            );
                            return;
                        }
                        (
                            format!("Please summarize this message: {text}"),
                            Some("discord:context_menu:summarize".to_string()),
                        )
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Discord: failed to fetch target message for summarize: {e}"
                        );
                        return;
                    }
                }
            }
            DiscordContextMenuCommand::AskAboutUser {
                user_id: target_user,
            } => (
                format!("Tell me about Discord user <@{}>", target_user.as_str()),
                Some(format!(
                    "discord:context_menu:ask_user:{}",
                    target_user.as_str()
                )),
            ),
        };

        let context_hint = command_context_hint
            .map(|hint| format!("{DISCORD_EPHEMERAL_CONTEXT_HINT}\n{hint}"))
            .or(context_hint);

        self.send_channel_message(
            tx,
            interaction_id,
            channel_id,
            Some(interaction_token),
            user_id,
            content,
            context_hint,
        )
        .await;
    }

    async fn handle_message_component_interaction(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        user_id: &str,
        data: &serde_json::Value,
    ) {
        if let Err(e) = self
            .http
            .create_interaction_response(
                interaction_id,
                interaction_token,
                InteractionCallbackType::DeferredUpdateMessage as u8,
                None,
            )
            .await
        {
            tracing::warn!("Discord: failed to ACK component interaction: {e}");
            return;
        }
        let custom_id = data
            .get("custom_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let (action, payload) = custom_id.split_once(':').unwrap_or((custom_id, ""));
        tracing::info!(
            custom_id = %custom_id,
            action = %action,
            payload = %payload,
            user_id = %user_id,
            "channel.interaction.component"
        );
        match action {
            "approve" | "deny" => {
                // Tool approval button — not yet wired to approval broker.
                tracing::debug!(action, payload, "component approval button clicked");
            }
            _ => {
                tracing::debug!(action, "unrecognized component action");
            }
        }
    }

    async fn handle_modal_submit_interaction(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        interaction_id: &str,
        interaction_token: &str,
        channel_id: &str,
        user_id: &str,
        is_dm: bool,
        data: &serde_json::Value,
    ) {
        let context_hint = direct_interaction_context_hint(is_dm);
        if let Err(e) = super::commands::defer_interaction(
            &self.http,
            interaction_id,
            interaction_token,
            interaction_defer_flags(context_hint.as_deref()),
        )
        .await
        {
            tracing::warn!("Discord: failed to defer modal submit: {e}");
            return;
        }
        let custom_id = data
            .get("custom_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let fields = super::commands::extract_modal_fields(data);
        tracing::info!(
            custom_id = %custom_id,
            user_id = %user_id,
            field_count = fields.len(),
            "channel.interaction.modal_submit"
        );

        if custom_id == "ask" {
            let Some(message) = fields
                .into_iter()
                .find_map(|(id, val)| (id == "message_text").then_some(val))
            else {
                tracing::warn!(custom_id, "modal 'ask' missing message_text field");
                return;
            };
            self.send_channel_message(
                tx,
                interaction_id,
                channel_id,
                Some(interaction_token),
                user_id,
                message,
                context_hint,
            )
            .await;
        }
    }

    async fn handle_autocomplete_interaction(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        user_id: &str,
        data: &serde_json::Value,
    ) {
        let command_name = data
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let choices = match command_name {
            "think" => Self::autocomplete_think(data),
            _ => vec![],
        };
        if let Err(e) = self
            .http
            .create_interaction_response(
                interaction_id,
                interaction_token,
                InteractionCallbackType::ApplicationCommandAutocompleteResult as u8,
                Some(serde_json::json!({ "choices": choices })),
            )
            .await
        {
            tracing::warn!(
                user_id = %user_id,
                command_name = %command_name,
                "Discord: failed to send autocomplete response: {e}"
            );
        }
    }

    fn autocomplete_think(data: &serde_json::Value) -> Vec<serde_json::Value> {
        const SETTINGS: &[&str] = &["off", "low", "medium", "high", "show", "hide", "status"];
        let typed = data
            .get("options")
            .and_then(serde_json::Value::as_array)
            .and_then(|opts| {
                opts.iter().find_map(|opt| {
                    opt.get("focused")
                        .and_then(serde_json::Value::as_bool)
                        .filter(|&f| f)
                        .and_then(|_| opt.get("value"))
                        .and_then(serde_json::Value::as_str)
                })
            })
            .unwrap_or("");
        let typed_lower = typed.to_ascii_lowercase();
        SETTINGS
            .iter()
            .filter(|&&s| s.starts_with(typed_lower.as_str()))
            .map(|&s| serde_json::json!({ "name": s, "value": s }))
            .collect()
    }

    async fn send_channel_message(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        interaction_id: &str,
        channel_id: &str,
        interaction_token: Option<&str>,
        user_id: &str,
        content: String,
        context_hint: Option<String>,
    ) {
        // When an interaction token is available and application_id is configured,
        // register the secret token in an in-memory route table and put only a
        // non-secret route id in conversation_id. Channel::send() can still edit
        // the deferred interaction response without leaking the token into
        // transcripts, autosave metadata, session IDs, or logs.
        let conversation_id = if let Some(token) = interaction_token {
            if let Some(app_id) = self.config.application_id.as_deref() {
                Some(self.register_interaction_route(channel_id, interaction_id, app_id, token))
            } else {
                tracing::warn!(
                    channel_id = %channel_id,
                    "Discord: interaction_token provided but application_id is not configured; \
                     deferred response will appear as a separate channel message"
                );
                Some(channel_id.to_string())
            }
        } else {
            Some(channel_id.to_string())
        };

        let msg = ChannelMessage {
            id: Uuid::new_v4().to_string(),
            sender: user_id.to_string(),
            content,
            channel: "discord".to_string(),
            context_hint,
            conversation_id,
            thread_id: None,
            reply_to: None,
            message_id: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            attachments: vec![],
        };
        if tx.send(ChannelEvent::Message(msg)).await.is_err() {
            tracing::warn!("Discord: channel message receiver dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelSecurityPolicy, DiscordConfig};
    use serde_json::json;

    fn test_config() -> DiscordConfig {
        DiscordConfig {
            bot_token: "fake-token".to_string(),
            application_id: Some("app-123".to_string()),
            guild_id: None,
            allowed_users: vec![],
            intents: None,
            status: None,
            default_account: None,
            default_to: None,
            activity_type: None,
            activity_name: None,
            thinking_embed: false,
            thinking_embed_include_preview: false,
            pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
            security: ChannelSecurityPolicy::default(),
        }
    }

    // ── extract_modal_fields ──────────────────────────────────────────────────

    #[test]
    fn extract_modal_fields_returns_all_inputs() {
        let data = json!({
            "custom_id": "ask",
            "components": [
                {
                    "type": 1,
                    "components": [
                        { "type": 4, "custom_id": "message_text", "value": "Hello world" }
                    ]
                }
            ]
        });
        let fields = super::super::commands::extract_modal_fields(&data);
        assert_eq!(
            fields,
            vec![("message_text".to_string(), "Hello world".to_string())]
        );
    }

    #[test]
    fn extract_modal_fields_multiple_rows() {
        let data = json!({
            "custom_id": "form",
            "components": [
                {
                    "type": 1,
                    "components": [{ "type": 4, "custom_id": "name", "value": "Alice" }]
                },
                {
                    "type": 1,
                    "components": [{ "type": 4, "custom_id": "note", "value": "Hi" }]
                }
            ]
        });
        let fields = super::super::commands::extract_modal_fields(&data);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("name".to_string(), "Alice".to_string()));
        assert_eq!(fields[1], ("note".to_string(), "Hi".to_string()));
    }

    #[test]
    fn extract_modal_fields_empty_components() {
        let data = json!({ "custom_id": "x", "components": [] });
        assert!(super::super::commands::extract_modal_fields(&data).is_empty());
    }

    #[test]
    fn extract_modal_fields_missing_components_key() {
        let data = json!({ "custom_id": "x" });
        assert!(super::super::commands::extract_modal_fields(&data).is_empty());
    }

    // ── autocomplete_think ────────────────────────────────────────────────────

    #[test]
    fn autocomplete_think_empty_typed_returns_all() {
        let data = json!({
            "name": "think",
            "options": [{ "name": "setting", "focused": true, "value": "" }]
        });
        let choices = DiscordChannel::autocomplete_think(&data);
        assert_eq!(choices.len(), 7);
    }

    #[test]
    fn autocomplete_think_prefix_filters_correctly() {
        // "hi" matches both "high" and "hide"
        let data = json!({
            "name": "think",
            "options": [{ "name": "setting", "focused": true, "value": "hi" }]
        });
        let choices = DiscordChannel::autocomplete_think(&data);
        let values: Vec<&str> = choices
            .iter()
            .map(|c| c["value"].as_str().unwrap_or(""))
            .collect();
        assert!(values.contains(&"high"));
        assert!(values.contains(&"hide"));
        assert_eq!(choices.len(), 2);
    }

    #[test]
    fn autocomplete_think_case_insensitive() {
        // "HIG" matches only "high" after lowercasing
        let data = json!({
            "name": "think",
            "options": [{ "name": "setting", "focused": true, "value": "HIG" }]
        });
        let choices = DiscordChannel::autocomplete_think(&data);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0]["value"], "high");
    }

    #[test]
    fn autocomplete_think_no_match_returns_empty() {
        let data = json!({
            "name": "think",
            "options": [{ "name": "setting", "focused": true, "value": "xyz" }]
        });
        let choices = DiscordChannel::autocomplete_think(&data);
        assert!(choices.is_empty());
    }

    #[test]
    fn autocomplete_think_no_focused_option_returns_all() {
        let data = json!({
            "name": "think",
            "options": [{ "name": "setting", "value": "hi" }]
        });
        // No focused:true → typed = "" → all 7
        let choices = DiscordChannel::autocomplete_think(&data);
        assert_eq!(choices.len(), 7);
    }

    #[test]
    fn autocomplete_think_s_prefix_matches_show_status() {
        let data = json!({
            "name": "think",
            "options": [{ "name": "setting", "focused": true, "value": "s" }]
        });
        let choices = DiscordChannel::autocomplete_think(&data);
        let values: Vec<&str> = choices
            .iter()
            .map(|c| c["value"].as_str().unwrap_or(""))
            .collect();
        assert!(values.contains(&"show"));
        assert!(values.contains(&"status"));
        assert_eq!(choices.len(), 2);
    }

    #[test]
    fn direct_interaction_context_hint_reflects_public_vs_dm_surface() {
        let public = direct_interaction_context_hint(false).expect("public hint");
        assert!(public.contains("Direct mention"));
        assert!(!public.contains("DM"));
        assert_eq!(interaction_defer_flags(Some(&public)), 0);

        let dm = direct_interaction_context_hint(true).expect("dm hint");
        assert!(dm.contains("DM"));
        assert_eq!(interaction_defer_flags(Some(&dm)), 0);
    }

    #[test]
    fn ephemeral_interaction_context_uses_ephemeral_defer_flags() {
        assert_eq!(
            interaction_defer_flags(Some(DISCORD_EPHEMERAL_CONTEXT_HINT)),
            message_flags::EPHEMERAL
        );
    }

    #[tokio::test]
    async fn interaction_conversation_id_omits_secret_token() {
        let ch = DiscordChannel::new(test_config());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        ch.send_channel_message(
            &tx,
            "interaction-42",
            "channel-99",
            Some("secret.interaction-token"),
            "user-7",
            "hello".to_string(),
            None,
        )
        .await;

        let event = rx.recv().await.expect("message event");
        let ChannelEvent::Message(message) = event else {
            panic!("expected message event");
        };
        let conversation_id = message.conversation_id.expect("conversation id");
        assert_eq!(
            conversation_id,
            "discord_interaction|channel-99|interaction-42"
        );
        assert!(!conversation_id.contains("secret.interaction-token"));

        assert_eq!(
            super::super::channel::parse_interaction_routing(&conversation_id),
            Some(("channel-99", "interaction-42"))
        );
    }
}
