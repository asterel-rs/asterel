use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::config_store::{
    load_persisted_runtime_config, maybe_request_channel_surface_reload, runtime_apply_mode,
    save_persisted_runtime_config,
};
use super::{
    ChannelMutationResult, ManagedChannelInventory, ManagedChannelRecord, ManagedRuntimeOwner,
};
use crate::config::schema::{IrcConfig, WhatsAppConfig};
use crate::config::{
    ChannelsConfig, Config, DiscordConfig, EmailConfig, IMessageConfig, MatrixConfig, SlackConfig,
    TelegramConfig, WebhookConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManagedChannelKind {
    Cli,
    Telegram,
    Discord,
    Slack,
    Webhook,
    IMessage,
    Matrix,
    WhatsApp,
    Email,
    Irc,
}

impl ManagedChannelKind {
    pub(super) const ALL: [Self; 10] = [
        Self::Cli,
        Self::Telegram,
        Self::Discord,
        Self::Slack,
        Self::Webhook,
        Self::IMessage,
        Self::Matrix,
        Self::WhatsApp,
        Self::Email,
        Self::Irc,
    ];

    pub(super) fn parse(raw: &str) -> Result<Self> {
        match normalize_channel_selector(raw).as_str() {
            "cli" => Ok(Self::Cli),
            "telegram" => Ok(Self::Telegram),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            "webhook" => Ok(Self::Webhook),
            "imessage" => Ok(Self::IMessage),
            "matrix" => Ok(Self::Matrix),
            "whatsapp" => Ok(Self::WhatsApp),
            "email" => Ok(Self::Email),
            "irc" => Ok(Self::Irc),
            _ => bail!("unknown channel '{raw}'"),
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Telegram => "telegram",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Webhook => "webhook",
            Self::IMessage => "imessage",
            Self::Matrix => "matrix",
            Self::WhatsApp => "whatsapp",
            Self::Email => "email",
            Self::Irc => "irc",
        }
    }

    pub(super) fn display_name(self) -> &'static str {
        match self {
            Self::Cli => "CLI",
            Self::Telegram => "Telegram",
            Self::Discord => "Discord",
            Self::Slack => "Slack",
            Self::Webhook => "Webhook",
            Self::IMessage => "iMessage",
            Self::Matrix => "Matrix",
            Self::WhatsApp => "WhatsApp",
            Self::Email => "Email",
            Self::Irc => "IRC",
        }
    }

    fn owner(self) -> ManagedRuntimeOwner {
        match self {
            Self::Cli => ManagedRuntimeOwner::CliSurface,
            Self::Webhook => ManagedRuntimeOwner::GatewaySurface,
            Self::Telegram
            | Self::Discord
            | Self::Slack
            | Self::IMessage
            | Self::Matrix
            | Self::WhatsApp
            | Self::Email
            | Self::Irc => ManagedRuntimeOwner::ChannelsSurface,
        }
    }

    fn supported(self) -> bool {
        match self {
            Self::Cli | Self::Webhook => true,
            Self::Telegram => cfg!(feature = "telegram"),
            Self::Discord => cfg!(feature = "discord"),
            Self::Slack => cfg!(feature = "slack"),
            Self::IMessage => cfg!(feature = "imessage"),
            Self::Matrix => cfg!(feature = "matrix"),
            Self::WhatsApp => cfg!(feature = "whatsapp"),
            Self::Email => cfg!(feature = "email"),
            Self::Irc => cfg!(feature = "irc"),
        }
    }

    fn configured(self, config: &ChannelsConfig) -> bool {
        match self {
            Self::Cli => config.cli,
            Self::Telegram => config.telegram.is_some(),
            Self::Discord => config.discord.is_some(),
            Self::Slack => config.slack.is_some(),
            Self::Webhook => config.webhook.is_some(),
            Self::IMessage => config.imessage.is_some(),
            Self::Matrix => config.matrix.is_some(),
            Self::WhatsApp => config.whatsapp.is_some(),
            Self::Email => config.email.is_some(),
            Self::Irc => config.irc.is_some(),
        }
    }

    pub(super) fn ensure_mutable(self) -> Result<()> {
        if matches!(self, Self::Cli) {
            bail!("CLI is built in and is not managed through admin channel mutations");
        }
        if !self.supported() {
            bail!(
                "channel '{}' is not supported by this runtime build",
                self.display_name()
            );
        }
        Ok(())
    }

    pub(super) fn record(self, config: &ChannelsConfig) -> ManagedChannelRecord {
        ManagedChannelRecord {
            id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            configured: self.configured(config),
            enabled: config.is_channel_enabled(self.id()),
            supported: self.supported(),
            owner: self.owner(),
        }
    }

    fn set_config(self, channels: &mut ChannelsConfig, raw: Value) -> Result<()> {
        match self {
            Self::Cli => bail!("CLI does not accept config payloads"),
            Self::Telegram => {
                channels.telegram = Some(parse_json_config::<TelegramConfig>(raw, self.id())?);
            }
            Self::Discord => {
                channels.discord = Some(parse_json_config::<DiscordConfig>(raw, self.id())?);
            }
            Self::Slack => {
                channels.slack = Some(parse_json_config::<SlackConfig>(raw, self.id())?);
            }
            Self::Webhook => {
                channels.webhook = Some(parse_json_config::<WebhookConfig>(raw, self.id())?);
            }
            Self::IMessage => {
                channels.imessage = Some(parse_json_config::<IMessageConfig>(raw, self.id())?);
            }
            Self::Matrix => {
                channels.matrix = Some(parse_json_config::<MatrixConfig>(raw, self.id())?);
            }
            Self::WhatsApp => {
                channels.whatsapp = Some(parse_json_config::<WhatsAppConfig>(raw, self.id())?);
            }
            Self::Email => {
                channels.email = Some(parse_json_config::<EmailConfig>(raw, self.id())?);
            }
            Self::Irc => {
                channels.irc = Some(parse_json_config::<IrcConfig>(raw, self.id())?);
            }
        }
        Ok(())
    }
}

fn normalize_channel_selector(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_json_config<T>(raw: Value, channel_id: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(raw)
        .with_context(|| format!("parse config payload for channel '{channel_id}'"))
}

pub(super) fn set_channel_enabled(
    channels: &mut ChannelsConfig,
    channel_id: &str,
    enabled: bool,
) -> bool {
    let normalized = normalize_channel_selector(channel_id);
    let before_len = channels.disabled_channels.len();
    channels
        .disabled_channels
        .retain(|entry| normalize_channel_selector(entry) != normalized);
    let mut changed = channels.disabled_channels.len() != before_len;
    if !enabled {
        channels.disabled_channels.push(normalized);
        changed = true;
    }
    channels.disabled_channels.sort();
    channels.disabled_channels.dedup();
    changed
}

pub(super) fn list_admin_channels(current: &Config) -> Result<ManagedChannelInventory> {
    let config = load_persisted_runtime_config(current)?;
    let items = ManagedChannelKind::ALL
        .into_iter()
        .map(|kind| kind.record(&config.channels_config))
        .collect::<Vec<_>>();
    let active_names = config
        .channels_config
        .active_channel_names()
        .into_iter()
        .map(ToString::to_string)
        .collect();

    Ok(ManagedChannelInventory {
        items,
        active_names,
        high_freedom: config.channels_config.high_freedom_all_channels,
    })
}

pub(super) fn create_admin_channel(
    current: &Config,
    channel_type: &str,
    raw_config: Option<Value>,
) -> Result<ChannelMutationResult> {
    let kind = ManagedChannelKind::parse(channel_type)?;
    kind.ensure_mutable()?;
    let raw_config = raw_config.ok_or_else(|| {
        anyhow::anyhow!(
            "channel '{}' requires a config payload to be created",
            kind.display_name()
        )
    })?;

    let mut config = load_persisted_runtime_config(current)?;
    if kind.configured(&config.channels_config) {
        bail!("channel '{}' is already configured", kind.display_name());
    }

    kind.set_config(&mut config.channels_config, raw_config)?;
    set_channel_enabled(&mut config.channels_config, kind.id(), true);
    save_persisted_runtime_config(&config)?;
    let record = kind.record(&config.channels_config);

    Ok(ChannelMutationResult {
        reload_requested: maybe_request_channel_surface_reload(&config, &record),
        record,
        changes: vec!["configured".to_string(), "enabled".to_string()],
        apply_mode: runtime_apply_mode(&config),
    })
}

pub(super) fn update_channel(
    current: &Config,
    channel_id: &str,
    enabled: Option<bool>,
    raw_config: Option<Value>,
) -> Result<ChannelMutationResult> {
    let kind = ManagedChannelKind::parse(channel_id)?;
    kind.ensure_mutable()?;

    let mut config = load_persisted_runtime_config(current)?;
    if !kind.configured(&config.channels_config) {
        bail!("channel '{}' is not configured", kind.display_name());
    }

    let mut changes = Vec::new();
    if let Some(raw_config) = raw_config {
        kind.set_config(&mut config.channels_config, raw_config)?;
        changes.push("config".to_string());
    }
    if let Some(enabled) = enabled
        && set_channel_enabled(&mut config.channels_config, kind.id(), enabled)
    {
        changes.push("enabled".to_string());
    }

    if !changes.is_empty() {
        save_persisted_runtime_config(&config)?;
    }
    let record = kind.record(&config.channels_config);

    Ok(ChannelMutationResult {
        reload_requested: if changes.is_empty() {
            false
        } else {
            maybe_request_channel_surface_reload(&config, &record)
        },
        record,
        changes,
        apply_mode: runtime_apply_mode(&config),
    })
}
