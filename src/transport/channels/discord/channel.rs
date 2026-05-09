//! Discord channel `Channel` trait implementation: gateway connection,
//! message/event dispatch, slash commands, and media sending.
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;

use super::gateway::{DiscordGateway, DiscordGatewayState, GatewayEvent};
use super::http_client::DiscordHttpClient;
use super::types::{DEFAULT_INTENTS, MAX_MESSAGE_LENGTH};
use crate::config::schema::DiscordConfig;
use crate::contracts::ids::UserId;
use crate::transport::channels::attachments::media_attachment_url;
use crate::transport::channels::policy::{AllowlistMatch, is_allowed_user};
use crate::transport::channels::traits::{
    Channel, ChannelCapabilities, ChannelEvent, MediaAttachment, MediaContent,
};

/// Parse an interaction-routed conversation ID.
///
/// Format: `"discord_interaction|{real_channel_id}|{route_id}"`
///
/// Returns `(real_channel_id, route_id)` on match, `None` otherwise.
/// The `|` separator is safe: Discord snowflakes are numeric-only and generated
/// route IDs are local UUID/interaction identifiers, neither contains `|`.
pub(super) fn parse_interaction_routing(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("discord_interaction|")?;
    let (channel_id, route_id) = rest.split_once('|')?;
    Some((channel_id, route_id))
}

fn typing_channel_for_recipient(recipient: &str) -> Option<&str> {
    if parse_interaction_routing(recipient).is_some() {
        None
    } else {
        Some(recipient)
    }
}

#[derive(Clone)]
struct InteractionRoute {
    application_id: String,
    token: String,
    created_at: Instant,
}

const INTERACTION_ROUTE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_INTERACTION_ROUTES: usize = 1024;

/// Discord channel adapter implementing the `Channel` trait.
pub struct DiscordChannel {
    pub(super) http: DiscordHttpClient,
    pub(super) gateway_state: Arc<DiscordGatewayState>,
    pub(super) config: DiscordConfig,
    pub(super) bot_user_id: std::sync::Mutex<Option<UserId>>,
    pub(super) ambient_reply_history: std::sync::Mutex<VecDeque<u64>>,
    interaction_routes: std::sync::Mutex<HashMap<String, InteractionRoute>>,
}

impl DiscordChannel {
    /// Create a new Discord channel from configuration.
    #[must_use]
    pub fn new(config: DiscordConfig) -> Self {
        Self {
            http: DiscordHttpClient::new(&config.bot_token),
            gateway_state: Arc::new(DiscordGatewayState::default()),
            config,
            bot_user_id: std::sync::Mutex::new(None),
            ambient_reply_history: std::sync::Mutex::new(VecDeque::new()),
            interaction_routes: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn prune_interaction_routes(routes: &mut HashMap<String, InteractionRoute>, now: Instant) {
        routes.retain(|_, route| now.duration_since(route.created_at) <= INTERACTION_ROUTE_TTL);
        while routes.len() > MAX_INTERACTION_ROUTES {
            let Some(oldest_key) = routes
                .iter()
                .min_by_key(|(_, route)| route.created_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            routes.remove(&oldest_key);
        }
    }

    pub(super) fn register_interaction_route(
        &self,
        channel_id: &str,
        interaction_id: &str,
        application_id: &str,
        token: &str,
    ) -> String {
        let route_id = interaction_id.to_string();
        if let Ok(mut routes) = self.interaction_routes.lock() {
            let now = Instant::now();
            Self::prune_interaction_routes(&mut routes, now);
            routes.insert(
                route_id.clone(),
                InteractionRoute {
                    application_id: application_id.to_string(),
                    token: token.to_string(),
                    created_at: now,
                },
            );
            Self::prune_interaction_routes(&mut routes, now);
        }
        format!("discord_interaction|{channel_id}|{route_id}")
    }

    fn resolve_interaction_route(&self, route_id: &str) -> Option<InteractionRoute> {
        let mut routes = self.interaction_routes.lock().ok()?;
        Self::prune_interaction_routes(&mut routes, Instant::now());
        routes.get(route_id).cloned()
    }

    pub(super) fn is_user_allowed(&self, user_id: &str) -> bool {
        is_allowed_user(&self.config.allowed_users, user_id, AllowlistMatch::Exact)
    }

    fn intents(&self) -> u64 {
        self.config.intents.unwrap_or(DEFAULT_INTENTS)
    }

    fn build_presence(&self) -> Option<serde_json::Value> {
        let status = self.config.status.as_deref().unwrap_or("online");
        let activity_name = self.config.activity_name.as_deref()?;
        let activity_type = self.config.activity_type.unwrap_or(0);

        Some(serde_json::json!({
            "status": status,
            "activities": [{
                "name": activity_name,
                "type": activity_type,
            }],
            "since": null,
            "afk": false,
        }))
    }

    pub(super) fn matches_guild_filter(&self, guild_id: Option<&str>) -> bool {
        match &self.config.guild_id {
            Some(gid) => guild_id.is_some_and(|g| g == gid),
            None => true,
        }
    }

    pub(super) fn set_bot_user_id(&self, user_id: &UserId) {
        if let Ok(mut guard) = self.bot_user_id.lock() {
            *guard = Some(user_id.clone());
        }
    }

    pub(super) fn is_bot_user(&self, user_id: &str) -> bool {
        self.bot_user_id
            .lock()
            .ok()
            .is_some_and(|guard| guard.as_ref().is_some_and(|id| id.as_str() == user_id))
    }

    pub(super) fn current_bot_user_id(&self) -> Option<UserId> {
        self.bot_user_id.lock().ok().and_then(|guard| guard.clone())
    }

    pub(super) fn try_consume_ambient_pickup_budget(&self, now_secs: u64) -> bool {
        let policy = &self.config.pickup_policy;
        if policy.mode != crate::config::DiscordPickupMode::SparseAmbient
            || policy.max_unsummoned_replies_per_hour == 0
        {
            return false;
        }

        let Ok(mut history) = self.ambient_reply_history.lock() else {
            return false;
        };

        while let Some(oldest) = history.front().copied() {
            if now_secs.saturating_sub(oldest) < 3600 {
                break;
            }
            history.pop_front();
        }

        if history
            .back()
            .copied()
            .is_some_and(|last| now_secs.saturating_sub(last) < policy.min_gap_seconds)
        {
            return false;
        }

        if history.len() >= policy.max_unsummoned_replies_per_hour as usize {
            return false;
        }

        history.push_back(now_secs);
        true
    }

    pub(super) fn should_forward_ambient_message(&self, content: &str, now_secs: u64) -> bool {
        super::addressability::looks_like_ambient_pickup_candidate(content)
            && self.try_consume_ambient_pickup_budget(now_secs)
    }

    pub(super) fn attachment_to_media(att: &super::gateway::RawAttachment) -> MediaAttachment {
        media_attachment_url(
            att.url.clone(),
            att.content_type.as_deref(),
            att.filename.clone(),
        )
    }

    pub(super) fn slash_command_input(command: super::commands::DiscordSlashCommand) -> String {
        match command {
            super::commands::DiscordSlashCommand::Ask { message } => message,
            super::commands::DiscordSlashCommand::Think { setting } => match setting {
                Some(setting) => format!("/think {setting}"),
                None => "/think".to_string(),
            },
        }
    }
}

impl Channel for DiscordChannel {
    fn name(&self) -> &'static str {
        "discord"
    }

    fn max_message_length(&self) -> usize {
        MAX_MESSAGE_LENGTH
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            can_edit_message: true,
            can_delete_message: true,
            can_send_media: true,
            can_send_embed: true,
            can_send_typing: true,
            max_message_length: MAX_MESSAGE_LENGTH,
            can_create_thread: true,
            can_manage_thread_members: true,
            can_add_reaction: true,
            can_read_reactions: true,
            can_send_buttons: true,
            can_send_select_menu: true,
            can_send_modal: true,
            can_fetch_history: true,
            can_receive_reactions: true,
            can_receive_edits: true,
            can_receive_deletes: true,
            can_receive_typing: true,
            ack_deadline_ms: 3000,
        }
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        channel_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Some((_, route_id)) = parse_interaction_routing(channel_id) {
                let Some(route) = self.resolve_interaction_route(route_id) else {
                    anyhow::bail!("Discord interaction route is no longer available");
                };
                self.http
                    .edit_original_interaction_response(
                        &route.application_id,
                        &route.token,
                        message,
                    )
                    .await
            } else {
                self.http
                    .send_message(channel_id, message)
                    .await
                    .map(|_| ())
            }
        })
    }

    fn listen<'a>(
        &'a self,
        tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let gateway = DiscordGateway::new(
                self.config.bot_token.clone(),
                self.intents(),
                Arc::clone(&self.gateway_state),
                self.build_presence(),
            );

            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<GatewayEvent>(100);

            let mut gateway_handle = {
                let http = DiscordHttpClient::new(&self.config.bot_token);
                tokio::spawn(async move { gateway.connect_and_listen(&http, &event_tx).await })
            };

            loop {
                tokio::select! {
                    event = event_rx.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        self.handle_gateway_event(event, &tx).await;
                    }
                    result = &mut gateway_handle => {
                        match result {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => return Err(e),
                            Err(e) => anyhow::bail!("Discord gateway task panicked: {e}"),
                        }
                        break;
                    }
                }
            }

            Ok(())
        })
    }

    fn send_typing<'a>(
        &'a self,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let Some(channel_id) = typing_channel_for_recipient(recipient) else {
                return Ok(());
            };
            self.http.send_typing(channel_id).await
        })
    }

    fn send_media<'a>(
        &'a self,
        attachment: &'a MediaAttachment,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let bytes = match &attachment.data {
                MediaContent::Url(media_url) => {
                    crate::security::validate_fetch_url(media_url, false)
                        .await
                        .context("validate Discord media URL")?;
                    self.http
                        .client()
                        .get(media_url)
                        .send()
                        .await
                        .context("download media for Discord upload")?
                        .bytes()
                        .await
                        .context("read media bytes")?
                        .to_vec()
                }
                MediaContent::Bytes(b) => b.clone(),
            };
            let filename = attachment
                .filename
                .as_deref()
                .unwrap_or("attachment")
                .to_string();
            self.http
                .send_media(recipient, bytes, &filename, &attachment.mime_type)
                .await
        })
    }

    fn edit_message<'a>(
        &'a self,
        channel_id: &'a str,
        message_id: &'a str,
        content: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.http
                .edit_message(channel_id, message_id, content)
                .await
        })
    }

    fn delete_message<'a>(
        &'a self,
        channel_id: &'a str,
        message_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.http.delete_message(channel_id, message_id).await })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move { self.http.get_current_user().await.is_ok() })
    }
}

#[cfg(test)]
mod tests {
    use super::super::gateway;
    use super::*;
    use crate::config::{ChannelSecurityPolicy, DiscordPickupMode};
    use crate::contracts::ids::{ChannelId, MessageId, UserId};
    use crate::transport::channels::traits::{ChannelEvent, MediaContent};

    fn test_config() -> DiscordConfig {
        DiscordConfig {
            bot_token: "fake-token".to_string(),
            application_id: None,
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

    #[test]
    fn parse_interaction_routing_valid() {
        let s = "discord_interaction|123456789|interaction-42";
        let result = parse_interaction_routing(s);
        assert_eq!(result, Some(("123456789", "interaction-42")));
    }

    #[test]
    fn parse_interaction_routing_plain_channel_id() {
        assert!(parse_interaction_routing("123456789").is_none());
    }

    #[test]
    fn parse_interaction_routing_incomplete_returns_none() {
        // Only channel_id, no app_id or token
        assert!(parse_interaction_routing("discord_interaction|123456789").is_none());
    }

    #[test]
    fn interaction_routed_replies_do_not_emit_public_typing_channel() {
        assert_eq!(
            typing_channel_for_recipient("discord_interaction|123|route"),
            None
        );
        assert_eq!(typing_channel_for_recipient("123"), Some("123"));
    }

    #[test]
    fn discord_channel_name() {
        let ch = DiscordChannel::new(test_config());
        assert_eq!(ch.name(), "discord");
    }

    #[test]
    fn discord_max_message_length() {
        let ch = DiscordChannel::new(test_config());
        assert_eq!(ch.max_message_length(), 2000);
    }

    #[test]
    fn default_intents_used_when_not_configured() {
        let ch = DiscordChannel::new(test_config());
        assert_eq!(ch.intents(), DEFAULT_INTENTS);
    }

    #[test]
    fn custom_intents_used_when_configured() {
        let mut cfg = test_config();
        cfg.intents = Some(12345);
        let ch = DiscordChannel::new(cfg);
        assert_eq!(ch.intents(), 12345);
    }

    #[test]
    fn empty_allowlist_denies_everyone() {
        let ch = DiscordChannel::new(test_config());
        assert!(!ch.is_user_allowed("12345"));
    }

    #[test]
    fn wildcard_allows_everyone() {
        let mut cfg = test_config();
        cfg.allowed_users = vec!["*".into()];
        let ch = DiscordChannel::new(cfg);
        assert!(ch.is_user_allowed("12345"));
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn specific_allowlist_filters() {
        let mut cfg = test_config();
        cfg.allowed_users = vec!["111".into(), "222".into()];
        let ch = DiscordChannel::new(cfg);
        assert!(ch.is_user_allowed("111"));
        assert!(ch.is_user_allowed("222"));
        assert!(!ch.is_user_allowed("333"));
    }

    #[test]
    fn guild_filter_none_accepts_all() {
        let ch = DiscordChannel::new(test_config());
        assert!(ch.matches_guild_filter(Some("any-guild")));
        assert!(ch.matches_guild_filter(None));
    }

    #[test]
    fn guild_filter_specific_rejects_mismatch() {
        let mut cfg = test_config();
        cfg.guild_id = Some("my-guild".into());
        let ch = DiscordChannel::new(cfg);
        assert!(ch.matches_guild_filter(Some("my-guild")));
        assert!(!ch.matches_guild_filter(Some("other-guild")));
        assert!(!ch.matches_guild_filter(None));
    }

    #[test]
    fn presence_none_without_activity_name() {
        let ch = DiscordChannel::new(test_config());
        assert!(ch.build_presence().is_none());
    }

    #[test]
    fn presence_built_with_activity_name() {
        let mut cfg = test_config();
        cfg.activity_name = Some("Watching you".into());
        cfg.activity_type = Some(3);
        cfg.status = Some("dnd".into());
        let ch = DiscordChannel::new(cfg);
        let presence = ch.build_presence().expect("should build presence");
        assert_eq!(presence["status"], "dnd");
        assert_eq!(presence["activities"][0]["name"], "Watching you");
        assert_eq!(presence["activities"][0]["type"], 3);
    }

    #[test]
    fn ambient_pickup_budget_rejects_when_direct_only() {
        let ch = DiscordChannel::new(test_config());
        assert!(!ch.try_consume_ambient_pickup_budget(1_000));
    }

    #[test]
    fn ambient_pickup_budget_enforces_gap_and_hourly_cap() {
        let mut cfg = test_config();
        cfg.pickup_policy.mode = DiscordPickupMode::SparseAmbient;
        cfg.pickup_policy.max_unsummoned_replies_per_hour = 2;
        cfg.pickup_policy.min_gap_seconds = 300;
        let ch = DiscordChannel::new(cfg);

        assert!(ch.try_consume_ambient_pickup_budget(1_000));
        assert!(!ch.try_consume_ambient_pickup_budget(1_100));
        assert!(ch.try_consume_ambient_pickup_budget(1_400));
        assert!(!ch.try_consume_ambient_pickup_budget(1_800));
        assert!(ch.try_consume_ambient_pickup_budget(4_700));
    }

    #[test]
    fn bot_user_id_tracking() {
        let ch = DiscordChannel::new(test_config());
        assert!(!ch.is_bot_user("123"));
        ch.set_bot_user_id(&UserId::new("123"));
        assert!(ch.is_bot_user("123"));
        assert!(!ch.is_bot_user("456"));
    }

    #[test]
    fn attachment_conversion() {
        let raw = gateway::RawAttachment {
            url: "https://cdn.discordapp.com/file.png".to_string(),
            filename: Some("file.png".to_string()),
            content_type: Some("image/png".to_string()),
        };
        let media = DiscordChannel::attachment_to_media(&raw);
        assert_eq!(media.mime_type, "image/png");
        assert_eq!(media.filename.as_deref(), Some("file.png"));
        assert!(matches!(media.data, MediaContent::Url(u) if u.contains("file.png")));
    }

    #[test]
    fn attachment_conversion_default_mime() {
        let raw = gateway::RawAttachment {
            url: "https://cdn.discordapp.com/blob".to_string(),
            filename: None,
            content_type: None,
        };
        let media = DiscordChannel::attachment_to_media(&raw);
        assert_eq!(media.mime_type, "application/octet-stream");
    }

    #[tokio::test]
    async fn handle_message_update_routes_message_edit_event() {
        let ch = DiscordChannel::new(test_config());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        ch.handle_gateway_event(
            GatewayEvent::MessageUpdate {
                channel_id: ChannelId::new("ch-1"),
                message_id: MessageId::new("msg-1"),
                content: Some("edited text".to_string()),
                author_id: Some(UserId::new("user-1")),
                guild_id: Some("guild-1".to_string()),
            },
            &tx,
        )
        .await;

        let event = rx.recv().await;
        assert!(event.is_some());
        if let Some(ChannelEvent::MessageEdit {
            channel_name,
            channel_id,
            message_id,
            new_content,
            user_id,
        }) = event
        {
            assert_eq!(channel_name, "discord");
            assert_eq!(channel_id, ChannelId::new("ch-1"));
            assert_eq!(message_id, MessageId::new("msg-1"));
            assert_eq!(new_content, "edited text");
            assert_eq!(user_id, UserId::new("user-1"));
        } else {
            panic!("expected MessageEdit event");
        }
    }

    #[tokio::test]
    async fn handle_message_update_without_content_is_ignored() {
        let ch = DiscordChannel::new(test_config());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        ch.handle_gateway_event(
            GatewayEvent::MessageUpdate {
                channel_id: ChannelId::new("ch-1"),
                message_id: MessageId::new("msg-1"),
                content: None,
                author_id: Some(UserId::new("user-1")),
                guild_id: Some("guild-1".to_string()),
            },
            &tx,
        )
        .await;

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn handle_message_delete_routes_message_delete_event() {
        let ch = DiscordChannel::new(test_config());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        ch.handle_gateway_event(
            GatewayEvent::MessageDelete {
                channel_id: ChannelId::new("ch-1"),
                message_id: MessageId::new("msg-2"),
                guild_id: Some("guild-1".to_string()),
            },
            &tx,
        )
        .await;

        let event = rx.recv().await;
        assert!(event.is_some());
        if let Some(ChannelEvent::MessageDelete {
            channel_name,
            channel_id,
            message_id,
        }) = event
        {
            assert_eq!(channel_name, "discord");
            assert_eq!(channel_id, ChannelId::new("ch-1"));
            assert_eq!(message_id, MessageId::new("msg-2"));
        } else {
            panic!("expected MessageDelete event");
        }
    }

    #[tokio::test]
    async fn handle_typing_start_routes_typing_start_event() {
        let ch = DiscordChannel::new(test_config());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        ch.handle_gateway_event(
            GatewayEvent::TypingStart {
                channel_id: ChannelId::new("ch-1"),
                user_id: UserId::new("user-1"),
                guild_id: Some("guild-1".to_string()),
                timestamp: 1_700_000_000,
            },
            &tx,
        )
        .await;

        let event = rx.recv().await;
        assert!(event.is_some());
        if let Some(ChannelEvent::TypingStart {
            channel_name,
            channel_id,
            user_id,
        }) = event
        {
            assert_eq!(channel_name, "discord");
            assert_eq!(channel_id, ChannelId::new("ch-1"));
            assert_eq!(user_id, UserId::new("user-1"));
        } else {
            panic!("expected TypingStart event");
        }
    }
}
