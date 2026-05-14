//! Per-channel adapter configuration for all nine I/O channels
//! (CLI, Telegram, Discord, Slack, Webhook, iMessage, Matrix,
//! `WhatsApp`, Email, IRC) plus routing and isolation rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::contracts::ids::UserId;
use crate::contracts::security::AutonomyLevel;

/// Configuration for all I/O channel adapters and routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsConfig {
    /// Whether the CLI channel is enabled. Default: true.
    #[serde(default = "default_cli_enabled")]
    pub cli: bool,
    /// Configured channels disabled by operator policy.
    #[serde(default)]
    pub disabled_channels: Vec<String>,
    /// If true, channel-level autonomy/tool restrictions are disabled for all channels.
    /// Global security controls still apply.
    #[serde(default)]
    pub high_freedom_all_channels: bool,
    /// Message coalescing window in milliseconds. 0 = disabled.
    #[serde(default = "default_coalescing_window_ms")]
    pub coalescing_window_ms: u64,
    /// Max messages to coalesce in one window. Default: 4.
    #[serde(default = "default_coalescing_max_messages")]
    pub coalescing_max_messages: usize,
    /// Global concurrency limit for message routing. Default: 1.
    #[serde(default = "default_routing_global_concurrency")]
    pub routing_global_concurrency: usize,
    /// Per-group queue capacity. Default: 32.
    #[serde(default = "default_routing_group_queue_capacity")]
    pub routing_group_queue_capacity: usize,
    /// Maximum number of routing groups. Default: 128.
    #[serde(default = "default_routing_max_groups")]
    pub routing_max_groups: usize,
    /// Rules for routing messages to named groups.
    #[serde(default)]
    pub routing_rules: Vec<RoutingRuleConfig>,
    /// Group isolation mode (off or global).
    #[serde(default)]
    pub group_isolation_mode: GroupIsolationMode,
    /// Per-group resource isolation rules.
    #[serde(default)]
    pub group_isolation_rules: Vec<GroupIsolationRuleConfig>,
    /// Event-driven trigger configuration.
    #[serde(default)]
    pub event_triggers: EventTriggerConfig,
    /// Per-channel model overrides.
    #[serde(default)]
    pub model_by_channel: BTreeMap<String, String>,
    /// Telegram bot adapter configuration.
    pub telegram: Option<TelegramConfig>,
    /// Discord bot adapter configuration.
    pub discord: Option<DiscordConfig>,
    /// Slack bot adapter configuration.
    pub slack: Option<SlackConfig>,
    /// Webhook channel adapter configuration.
    pub webhook: Option<WebhookConfig>,
    /// iMessage adapter configuration.
    pub imessage: Option<IMessageConfig>,
    /// Matrix adapter configuration.
    pub matrix: Option<MatrixConfig>,
    /// `WhatsApp` Business API adapter configuration.
    pub whatsapp: Option<WhatsAppConfig>,
    /// Email (IMAP/SMTP) adapter configuration.
    pub email: Option<EmailConfig>,
    /// IRC adapter configuration.
    pub irc: Option<IrcConfig>,
    /// X (Twitter) adapter configuration.
    pub twitter: Option<TwitterConfig>,
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            cli: true,
            disabled_channels: Vec::new(),
            high_freedom_all_channels: false,
            coalescing_window_ms: default_coalescing_window_ms(),
            coalescing_max_messages: default_coalescing_max_messages(),
            routing_global_concurrency: default_routing_global_concurrency(),
            routing_group_queue_capacity: default_routing_group_queue_capacity(),
            routing_max_groups: default_routing_max_groups(),
            routing_rules: Vec::new(),
            group_isolation_mode: GroupIsolationMode::default(),
            group_isolation_rules: Vec::new(),
            event_triggers: EventTriggerConfig::default(),
            model_by_channel: BTreeMap::new(),
            telegram: None,
            discord: None,
            slack: None,
            webhook: None,
            imessage: None,
            matrix: None,
            whatsapp: None,
            email: None,
            irc: None,
            twitter: None,
        }
    }
}

/// Event-driven trigger configuration for reactions and edits.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventTriggerConfig {
    /// Trigger on reaction added events. Default: false.
    #[serde(default)]
    pub reaction_add: bool,
    /// Trigger on reaction removed events. Default: false.
    #[serde(default)]
    pub reaction_remove: bool,
    /// Trigger on message edit events. Default: false.
    #[serde(default)]
    pub message_edit: bool,
    /// Cooldown between event triggers in seconds. Default: 5.
    #[serde(default = "default_event_cooldown_secs")]
    pub cooldown_secs: u64,
}

impl Default for EventTriggerConfig {
    fn default() -> Self {
        Self {
            reaction_add: false,
            reaction_remove: false,
            message_edit: false,
            cooldown_secs: default_event_cooldown_secs(),
        }
    }
}

/// Group isolation enforcement mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupIsolationMode {
    /// No isolation between groups.
    #[default]
    Off,
    /// Apply isolation rules globally across all channels.
    Global,
}

/// Resource isolation level for a group dimension.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupIsolationLevel {
    /// Resources are shared across groups.
    #[default]
    Shared,
    /// Isolated to a per-group workspace directory.
    Workspace,
    /// Fully containerized isolation.
    Container,
}

/// Per-group resource isolation rule across three dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupIsolationRuleConfig {
    /// Name of the routing group this rule applies to.
    pub group: String,
    /// Filesystem isolation level. Default: shared.
    #[serde(default)]
    pub filesystem: GroupIsolationLevel,
    /// Process isolation level. Default: shared.
    #[serde(default)]
    pub process: GroupIsolationLevel,
    /// Network isolation level. Default: shared.
    #[serde(default)]
    pub network: GroupIsolationLevel,
}

/// Rule that routes matching messages to a named group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingRuleConfig {
    /// Channel name to match (e.g. "discord", "telegram").
    pub channel: String,
    /// Optional sender filter (exact match).
    #[serde(default)]
    pub sender: Option<String>,
    /// Optional conversation/thread ID filter.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Target routing group name.
    pub group: String,
}

/// Per-channel autonomy and tool-access security controls.
///
/// Flattened into each channel adapter config so the TOML/JSON schema is
/// unchanged: `autonomy_level` and `tool_allowlist` remain top-level fields
/// on each channel section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelSecurityPolicy {
    /// Per-channel autonomy level override. Effective level = min(global, channel).
    #[serde(default, deserialize_with = "deserialize_autonomy_level_opt")]
    pub autonomy_level: Option<AutonomyLevel>,
    /// Per-channel tool allowlist. None = all tools permitted.
    #[serde(default)]
    pub tool_allowlist: Option<Vec<String>>,
}

impl ChannelsConfig {
    fn normalized_channel_id(raw: &str) -> String {
        raw.trim()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect()
    }

    fn configured_channel_id(name: &'static str) -> &'static str {
        match name {
            "Telegram" => "telegram",
            "Discord" => "discord",
            "Slack" => "slack",
            "Webhook" => "webhook",
            "iMessage" => "imessage",
            "Matrix" => "matrix",
            "WhatsApp" => "whatsapp",
            "Email" => "email",
            "IRC" => "irc",
            "Twitter" => "twitter",
            _ => "",
        }
    }

    /// Returns an array of (name, configured) pairs for all channels.
    #[must_use]
    pub fn configured_channel_flags(&self) -> [(&'static str, bool); 10] {
        [
            ("Telegram", self.telegram.is_some()),
            ("Discord", self.discord.is_some()),
            ("Slack", self.slack.is_some()),
            ("Webhook", self.webhook.is_some()),
            ("iMessage", self.imessage.is_some()),
            ("Matrix", self.matrix.is_some()),
            ("WhatsApp", self.whatsapp.is_some()),
            ("Email", self.email.is_some()),
            ("IRC", self.irc.is_some()),
            ("Twitter", self.twitter.is_some()),
        ]
    }

    /// Returns whether a configured channel is disabled by persisted operator state.
    #[must_use]
    pub fn is_channel_disabled(&self, channel_id: &str) -> bool {
        let normalized = Self::normalized_channel_id(channel_id);
        self.disabled_channels
            .iter()
            .any(|entry| Self::normalized_channel_id(entry) == normalized)
    }

    /// Returns whether a configured channel is currently enabled.
    #[must_use]
    pub fn is_channel_enabled(&self, channel_id: &str) -> bool {
        if Self::normalized_channel_id(channel_id) == "cli" {
            return self.cli;
        }

        let configured = self
            .configured_channel_flags()
            .into_iter()
            .find(|(name, _)| {
                Self::configured_channel_id(name) == Self::normalized_channel_id(channel_id)
            })
            .is_some_and(|(_, configured)| configured);
        configured && !self.is_channel_disabled(channel_id)
    }

    /// Returns names of all active channels (always includes "CLI").
    #[must_use]
    pub fn active_channel_names(&self) -> Vec<&'static str> {
        let mut active = Vec::with_capacity(11);
        if self.cli {
            active.push("CLI");
        }
        for (name, configured) in self.configured_channel_flags() {
            if configured && self.is_channel_enabled(Self::configured_channel_id(name)) {
                active.push(name);
            }
        }
        active
    }
}

/// Telegram bot adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Telegram Bot API token.
    pub bot_token: String,
    /// Allowed Telegram usernames (case-insensitive).
    pub allowed_users: Vec<String>,
    /// Default account label used for multi-account routing.
    #[serde(default)]
    pub default_account: Option<String>,
    /// Default recipient fallback when target resolution is unavailable.
    #[serde(default)]
    pub default_to: Option<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// Discord bot adapter configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiscordPickupMode {
    #[default]
    DirectOnly,
    SparseAmbient,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordPickupPolicyConfig {
    /// Whether the bot responds only to direct mentions (`DirectOnly`) or may
    /// occasionally join ambient public-channel conversations (`SparseAmbient`).
    /// Default: `DirectOnly`.
    #[serde(default)]
    pub mode: DiscordPickupMode,
    /// Maximum number of unsummoned (ambient) replies allowed per hour across
    /// all channels. 0 = no ambient replies permitted. Default: 0.
    #[serde(default)]
    pub max_unsummoned_replies_per_hour: u32,
    /// Minimum seconds that must elapse between consecutive unsummoned replies
    /// in the same channel to prevent flooding. Default: 600.
    #[serde(default = "default_discord_min_gap_seconds")]
    pub min_gap_seconds: u64,
}

impl Default for DiscordPickupPolicyConfig {
    fn default() -> Self {
        Self {
            mode: DiscordPickupMode::DirectOnly,
            max_unsummoned_replies_per_hour: 0,
            min_gap_seconds: default_discord_min_gap_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Discord bot token.
    pub bot_token: String,
    /// Discord application ID for interactions.
    #[serde(default)]
    pub application_id: Option<String>,
    /// Guild (server) ID to restrict the bot to.
    pub guild_id: Option<String>,
    /// Allowed Discord user IDs.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Gateway intent bitfield override.
    #[serde(default)]
    pub intents: Option<u64>,
    /// Custom bot status text.
    #[serde(default)]
    pub status: Option<String>,
    /// Default account label used for multi-account routing.
    #[serde(default)]
    pub default_account: Option<String>,
    /// Default recipient fallback when target resolution is unavailable.
    #[serde(default)]
    pub default_to: Option<String>,
    /// Discord activity type code (0=playing, 1=streaming, etc.).
    #[serde(default)]
    pub activity_type: Option<u8>,
    /// Discord activity display name.
    #[serde(default)]
    pub activity_name: Option<String>,
    /// Show live thinking/status updates as a Discord embed while streaming.
    #[serde(default = "default_discord_thinking_embed")]
    pub thinking_embed: bool,
    /// Include a short streaming preview in the thinking embed.
    #[serde(default = "default_discord_thinking_embed_preview")]
    pub thinking_embed_include_preview: bool,
    /// Policy for sparse ambient participation in public channels.
    #[serde(default)]
    pub pickup_policy: DiscordPickupPolicyConfig,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// Slack bot adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    /// Slack bot OAuth token (xoxb-...).
    pub bot_token: String,
    /// Slack app-level token for Socket Mode (xapp-...).
    pub app_token: Option<String>,
    /// Default channel ID to monitor.
    pub channel_id: Option<String>,
    /// Default account label used for multi-account routing.
    #[serde(default)]
    pub default_account: Option<String>,
    /// Default recipient fallback when target resolution is unavailable.
    #[serde(default)]
    pub default_to: Option<String>,
    /// Allowed Slack user IDs.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// Webhook channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Port to listen on for incoming webhooks.
    pub port: u16,
    /// Shared secret for HMAC signature verification.
    pub secret: Option<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// iMessage adapter configuration (macOS only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IMessageConfig {
    /// Allowed contact identifiers (phone or email).
    pub allowed_contacts: Vec<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// Matrix protocol adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// Matrix homeserver URL (e.g. `https://matrix.org`).
    pub homeserver: String,
    /// Matrix access token for the bot user.
    pub access_token: String,
    /// Room ID to monitor (e.g. `!abc:matrix.org`).
    pub room_id: String,
    /// Allowed Matrix user IDs (e.g. `@user:matrix.org`).
    pub allowed_users: Vec<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// `WhatsApp` Business API adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    /// Access token from Meta Business Suite
    pub access_token: String,
    /// Phone number ID from Meta Business API
    pub phone_number_id: String,
    /// Webhook verify token (you define this, Meta sends it back for verification)
    pub verify_token: String,
    /// App secret for webhook signature verification (X-Hub-Signature-256)
    #[serde(default)]
    pub app_secret: Option<String>,
    /// Allowed phone numbers (E.164 format: +1234567890) or "*" for all
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// IRC channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrcConfig {
    /// IRC server hostname
    pub server: String,
    /// IRC server port (default: 6697 for TLS)
    #[serde(default = "default_irc_port")]
    pub port: u16,
    /// Bot nickname
    pub nickname: String,
    /// Username (defaults to nickname if not set)
    pub username: Option<String>,
    /// Channels to join on connect
    #[serde(default)]
    pub channels: Vec<String>,
    /// Allowed nicknames (case-insensitive) or "*" for all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Server password (for bouncers like ZNC)
    pub server_password: Option<String>,
    /// `NickServ` IDENTIFY password
    pub nickserv_password: Option<String>,
    /// SASL PLAIN password (`IRCv3`)
    pub sasl_password: Option<String>,
    /// Verify TLS certificate (default: true)
    pub verify_tls: Option<bool>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// X (Twitter) channel adapter configuration (OAuth 2.0 PKCE).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitterConfig {
    /// OAuth 2.0 client ID (from developer.x.com).
    pub client_id: String,
    /// OAuth 2.0 client secret.
    pub client_secret: String,
    /// OAuth 2.0 access token (issued after PKCE flow).
    pub access_token: String,
    /// OAuth 2.0 refresh token (requires `offline.access` scope).
    pub refresh_token: String,
    /// Numeric Twitter user ID of the bot account.
    pub user_id: UserId,
    /// Twitter username of the bot account (without @).
    pub username: String,
    /// Allowed Twitter usernames (case-insensitive, without @).
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Mention poll interval in seconds. Default: 180 (5/15 min, well under 300 limit).
    #[serde(default = "default_twitter_mention_poll_interval")]
    pub mention_poll_interval_secs: u64,
    /// DM poll interval in seconds. Default: 300 (3/15 min, well under 15 limit).
    #[serde(default = "default_twitter_dm_poll_interval")]
    pub dm_poll_interval_secs: u64,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

/// Email (IMAP/SMTP) channel adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    /// IMAP server hostname.
    pub imap_host: String,
    /// IMAP server port. Default: 993.
    #[serde(default = "default_email_imap_port")]
    pub imap_port: u16,
    /// IMAP folder to monitor. Default: `"INBOX"`.
    #[serde(default = "default_email_imap_folder")]
    pub imap_folder: String,
    /// SMTP server hostname for outgoing mail.
    pub smtp_host: String,
    /// SMTP server port. Default: 587.
    #[serde(default = "default_email_smtp_port")]
    pub smtp_port: u16,
    /// Whether to use TLS for SMTP. Default: true.
    #[serde(default = "default_true")]
    pub smtp_tls: bool,
    /// Email account username for both IMAP and SMTP.
    pub username: String,
    /// Email account password.
    pub password: String,
    /// Sender address for outgoing messages.
    pub from_address: String,
    /// Polling interval for new mail in seconds. Default: 60.
    #[serde(default = "default_email_poll_interval")]
    pub poll_interval_secs: u64,
    /// Allowed sender addresses (empty = deny all).
    #[serde(default)]
    pub allowed_senders: Vec<String>,
    /// Per-channel autonomy and tool-access controls.
    #[serde(flatten)]
    pub security: ChannelSecurityPolicy,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            imap_host: String::new(),
            imap_port: default_email_imap_port(),
            imap_folder: default_email_imap_folder(),
            smtp_host: String::new(),
            smtp_port: default_email_smtp_port(),
            smtp_tls: true,
            username: String::new(),
            password: String::new(),
            from_address: String::new(),
            poll_interval_secs: default_email_poll_interval(),
            allowed_senders: Vec::new(),
            security: ChannelSecurityPolicy::default(),
        }
    }
}

fn deserialize_autonomy_level_opt<'de, D>(
    deserializer: D,
) -> Result<Option<AutonomyLevel>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|level| match level.as_str() {
            "read_only" => Ok(AutonomyLevel::ReadOnly),
            "supervised" => Ok(AutonomyLevel::Supervised),
            "full" => Ok(AutonomyLevel::Full),
            _ => Err(serde::de::Error::unknown_variant(
                &level,
                &["read_only", "supervised", "full"],
            )),
        })
        .transpose()
}

fn default_twitter_mention_poll_interval() -> u64 {
    180
}

fn default_twitter_dm_poll_interval() -> u64 {
    300
}

fn default_irc_port() -> u16 {
    6697
}

fn default_email_imap_port() -> u16 {
    993
}

fn default_email_smtp_port() -> u16 {
    587
}

fn default_email_imap_folder() -> String {
    "INBOX".into()
}

fn default_email_poll_interval() -> u64 {
    60
}

use super::default_true;

fn default_cli_enabled() -> bool {
    true
}

fn default_coalescing_window_ms() -> u64 {
    0
}

fn default_coalescing_max_messages() -> usize {
    4
}

fn default_discord_thinking_embed() -> bool {
    false
}

fn default_discord_thinking_embed_preview() -> bool {
    false
}

fn default_discord_min_gap_seconds() -> u64 {
    600
}

fn default_routing_global_concurrency() -> usize {
    1
}

fn default_routing_group_queue_capacity() -> usize {
    32
}

fn default_routing_max_groups() -> usize {
    128
}

fn default_event_cooldown_secs() -> u64 {
    5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_configs_deserialize_without_policy_fields() {
        let telegram: TelegramConfig =
            serde_json::from_str(r#"{"bot_token":"token","allowed_users":["u"]}"#).unwrap();
        assert!(telegram.default_account.is_none());
        assert!(telegram.default_to.is_none());
        assert!(telegram.security.autonomy_level.is_none());
        assert!(telegram.security.tool_allowlist.is_none());

        let discord: DiscordConfig =
            serde_json::from_str(r#"{"bot_token":"token","guild_id":null,"allowed_users":[]}"#)
                .unwrap();
        assert!(discord.default_account.is_none());
        assert!(discord.default_to.is_none());
        assert!(discord.security.autonomy_level.is_none());
        assert!(discord.security.tool_allowlist.is_none());
        assert!(!discord.thinking_embed);
        assert!(!discord.thinking_embed_include_preview);
        assert_eq!(discord.pickup_policy.mode, DiscordPickupMode::DirectOnly);
        assert_eq!(discord.pickup_policy.max_unsummoned_replies_per_hour, 0);
        assert_eq!(discord.pickup_policy.min_gap_seconds, 600);

        let slack: SlackConfig = serde_json::from_str(
            r#"{"bot_token":"token","app_token":null,"channel_id":null,"allowed_users":[]}"#,
        )
        .unwrap();
        assert!(slack.default_account.is_none());
        assert!(slack.default_to.is_none());
        assert!(slack.security.autonomy_level.is_none());
        assert!(slack.security.tool_allowlist.is_none());

        let webhook: WebhookConfig =
            serde_json::from_str(r#"{"port":8080,"secret":null}"#).unwrap();
        assert!(webhook.security.autonomy_level.is_none());
        assert!(webhook.security.tool_allowlist.is_none());

        let imessage: IMessageConfig =
            serde_json::from_str(r#"{"allowed_contacts":["*"]}"#).unwrap();
        assert!(imessage.security.autonomy_level.is_none());
        assert!(imessage.security.tool_allowlist.is_none());

        let matrix: MatrixConfig = serde_json::from_str(
            r#"{"homeserver":"https://example.org","access_token":"token","room_id":"!r:example.org","allowed_users":["*"]}"#,
        )
        .unwrap();
        assert!(matrix.security.autonomy_level.is_none());
        assert!(matrix.security.tool_allowlist.is_none());

        let whatsapp: WhatsAppConfig = serde_json::from_str(
            r#"{"access_token":"token","phone_number_id":"id","verify_token":"verify","allowed_numbers":["*"],"app_secret":null}"#,
        )
        .unwrap();
        assert!(whatsapp.security.autonomy_level.is_none());
        assert!(whatsapp.security.tool_allowlist.is_none());

        let irc: IrcConfig = serde_json::from_str(
            r#"{"server":"irc.example.com","nickname":"bot","port":6697,"username":null,"channels":[],"allowed_users":[],"server_password":null,"nickserv_password":null,"sasl_password":null,"verify_tls":null}"#,
        )
        .unwrap();
        assert!(irc.security.autonomy_level.is_none());
        assert!(irc.security.tool_allowlist.is_none());

        let email: EmailConfig = serde_json::from_str(
            r#"{"imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"bot@example.com","password":"secret","from_address":"bot@example.com"}"#,
        )
        .unwrap();
        assert!(email.security.autonomy_level.is_none());
        assert!(email.security.tool_allowlist.is_none());
    }

    #[test]
    fn channels_config_defaults_coalescing_to_disabled() {
        let config = ChannelsConfig::default();
        assert!(config.disabled_channels.is_empty());
        assert_eq!(config.coalescing_window_ms, 0);
        assert_eq!(config.coalescing_max_messages, 4);
        assert_eq!(config.routing_global_concurrency, 1);
        assert!(!config.high_freedom_all_channels);
        assert_eq!(config.routing_group_queue_capacity, 32);
        assert_eq!(config.routing_max_groups, 128);
        assert!(config.routing_rules.is_empty());
        assert_eq!(config.group_isolation_mode, GroupIsolationMode::Off);
        assert!(config.group_isolation_rules.is_empty());
        assert!(config.model_by_channel.is_empty());
        assert!(!config.event_triggers.reaction_add);
        assert!(!config.event_triggers.reaction_remove);
        assert!(!config.event_triggers.message_edit);
        assert_eq!(config.event_triggers.cooldown_secs, 5);
    }

    #[test]
    fn discord_pickup_policy_defaults_to_direct_only() {
        let config: DiscordConfig =
            serde_json::from_str(r#"{"bot_token":"token","guild_id":null,"allowed_users":[]}"#)
                .unwrap();

        assert_eq!(config.pickup_policy.mode, DiscordPickupMode::DirectOnly);
        assert_eq!(config.pickup_policy.max_unsummoned_replies_per_hour, 0);
        assert_eq!(config.pickup_policy.min_gap_seconds, 600);
    }

    #[test]
    fn channels_config_deserializes_coalescing_fields() {
        let config: ChannelsConfig = serde_json::from_str(
            r#"{
                "cli": true,
                "disabled_channels": ["discord"],
                "high_freedom_all_channels": true,
                "coalescing_window_ms": 750,
                "coalescing_max_messages": 6,
                "routing_global_concurrency": 3,
                "routing_group_queue_capacity": 24,
                "routing_max_groups": 64,
                "routing_rules": [
                    {
                        "channel": "discord",
                        "sender": "ops-user",
                        "conversation_id": null,
                        "group": "ops"
                    }
                ],
                "group_isolation_mode": "global",
                "group_isolation_rules": [
                    {
                        "group": "ops",
                        "filesystem": "container",
                        "process": "workspace",
                        "network": "shared"
                    }
                ],
                "event_triggers": {
                    "reaction_add": true,
                    "reaction_remove": false,
                    "message_edit": true,
                    "cooldown_secs": 9
                },
                "model_by_channel": {
                    "discord": "anthropic/claude-sonnet-4.6",
                    "telegram": "openai/gpt-5-mini"
                },
                "telegram": null,
                "discord": null,
                "slack": null,
                "webhook": null,
                "imessage": null,
                "matrix": null,
                "whatsapp": null,
                "email": null,
                "irc": null
            }"#,
        )
        .unwrap();

        assert_eq!(config.disabled_channels, vec!["discord".to_string()]);
        assert_eq!(config.coalescing_window_ms, 750);
        assert_eq!(config.coalescing_max_messages, 6);
        assert_eq!(config.routing_global_concurrency, 3);
        assert!(config.high_freedom_all_channels);
        assert_eq!(config.routing_group_queue_capacity, 24);
        assert_eq!(config.routing_max_groups, 64);
        assert_eq!(
            config.routing_rules,
            vec![RoutingRuleConfig {
                channel: "discord".to_string(),
                sender: Some("ops-user".to_string()),
                conversation_id: None,
                group: "ops".to_string(),
            }]
        );
        assert_eq!(config.group_isolation_mode, GroupIsolationMode::Global);
        assert_eq!(
            config.group_isolation_rules,
            vec![GroupIsolationRuleConfig {
                group: "ops".to_string(),
                filesystem: GroupIsolationLevel::Container,
                process: GroupIsolationLevel::Workspace,
                network: GroupIsolationLevel::Shared,
            }]
        );
        assert!(config.event_triggers.reaction_add);
        assert!(!config.event_triggers.reaction_remove);
        assert!(config.event_triggers.message_edit);
        assert_eq!(config.event_triggers.cooldown_secs, 9);
        assert_eq!(
            config.model_by_channel.get("discord"),
            Some(&"anthropic/claude-sonnet-4.6".to_string())
        );
        assert_eq!(
            config.model_by_channel.get("telegram"),
            Some(&"openai/gpt-5-mini".to_string())
        );
    }

    #[test]
    fn active_channel_names_skip_disabled_entries() {
        let config = ChannelsConfig {
            telegram: Some(TelegramConfig {
                bot_token: "token".to_string(),
                allowed_users: vec!["u".to_string()],
                default_account: None,
                default_to: None,
                security: ChannelSecurityPolicy::default(),
            }),
            disabled_channels: vec!["telegram".to_string()],
            ..ChannelsConfig::default()
        };

        assert_eq!(config.active_channel_names(), vec!["CLI"]);
        assert!(config.is_channel_disabled("telegram"));
        assert!(!config.is_channel_enabled("telegram"));
    }

    #[test]
    fn event_trigger_config_defaults_to_all_disabled() {
        let triggers = EventTriggerConfig::default();
        assert!(!triggers.reaction_add);
        assert!(!triggers.reaction_remove);
        assert!(!triggers.message_edit);
        assert_eq!(triggers.cooldown_secs, 5);
    }

    #[test]
    fn channel_config_deserializes_policy_fields() {
        let telegram: TelegramConfig = serde_json::from_str(
            r#"{"bot_token":"token","allowed_users":["u"],"autonomy_level":"read_only","tool_allowlist":["file_read"]}"#,
        )
        .unwrap();

        assert_eq!(
            telegram.security.autonomy_level,
            Some(AutonomyLevel::ReadOnly)
        );
        assert_eq!(
            telegram.security.tool_allowlist,
            Some(vec!["file_read".to_string()])
        );
    }
}
