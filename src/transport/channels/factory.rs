//! Channel factory: constructs configured channel adapters and their
//! per-channel security policies from `ChannelsConfig`.
use std::collections::HashSet;
use std::sync::Arc;

use crate::config::{ChannelsConfig, Config};
#[cfg(feature = "discord")]
use crate::transport::channels::DiscordChannel;
#[cfg(feature = "email")]
use crate::transport::channels::EmailChannel;
#[cfg(feature = "imessage")]
use crate::transport::channels::IMessageChannel;
#[cfg(feature = "matrix")]
use crate::transport::channels::MatrixChannel;
#[cfg(feature = "slack")]
use crate::transport::channels::SlackChannel;
#[cfg(feature = "telegram")]
use crate::transport::channels::TelegramChannel;
#[cfg(feature = "twitter")]
use crate::transport::channels::TwitterChannel;
#[cfg(feature = "whatsapp")]
use crate::transport::channels::WhatsAppChannel;
use crate::transport::channels::policy::{ChannelEntry, ChannelPolicy};
#[cfg(feature = "irc")]
use crate::transport::channels::{IrcChannel, IrcChannelConfig};

fn build_policy(
    autonomy_level: Option<crate::security::AutonomyLevel>,
    tool_allowlist: Option<Vec<String>>,
    high_freedom_all_channels: bool,
) -> ChannelPolicy {
    if high_freedom_all_channels {
        return ChannelPolicy {
            autonomy_level: None,
            tool_allowlist: None,
        };
    }

    ChannelPolicy {
        autonomy_level,
        tool_allowlist: tool_allowlist.map(|tools| tools.into_iter().collect::<HashSet<_>>()),
    }
}

#[cfg(feature = "telegram")]
fn build_telegram_entry(
    tg: crate::config::TelegramConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "Telegram",
        channel: Arc::new(TelegramChannel::new(tg.bot_token, tg.allowed_users)),
        policy: build_policy(
            tg.security.autonomy_level,
            tg.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[cfg(feature = "discord")]
fn build_discord_entry(
    dc: crate::config::DiscordConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    let policy = build_policy(
        dc.security.autonomy_level,
        dc.security.tool_allowlist.clone(),
        high_freedom_all_channels,
    );
    ChannelEntry {
        name: "Discord",
        channel: Arc::new(DiscordChannel::new(dc)),
        policy,
    }
}

#[cfg(feature = "slack")]
fn build_slack_entry(
    sl: crate::config::SlackConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "Slack",
        channel: Arc::new(SlackChannel::new(
            sl.bot_token,
            sl.channel_id,
            sl.allowed_users,
        )),
        policy: build_policy(
            sl.security.autonomy_level,
            sl.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[cfg(feature = "imessage")]
fn build_imessage_entry(
    im: crate::config::IMessageConfig,
    security: &Arc<crate::security::SecurityPolicy>,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "iMessage",
        channel: Arc::new(IMessageChannel::with_security(
            im.allowed_contacts,
            Arc::clone(security),
        )),
        policy: build_policy(
            im.security.autonomy_level,
            im.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[cfg(feature = "matrix")]
fn build_matrix_entry(
    mx: crate::config::MatrixConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "Matrix",
        channel: Arc::new(MatrixChannel::new(
            mx.homeserver,
            mx.access_token,
            mx.room_id,
            mx.allowed_users,
        )),
        policy: build_policy(
            mx.security.autonomy_level,
            mx.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[cfg(feature = "whatsapp")]
fn build_whatsapp_entry(
    wa: crate::config::schema::WhatsAppConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "WhatsApp",
        channel: Arc::new(WhatsAppChannel::new(
            wa.access_token,
            wa.phone_number_id,
            wa.verify_token,
            wa.allowed_numbers,
        )),
        policy: build_policy(
            wa.security.autonomy_level,
            wa.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[cfg(feature = "email")]
fn build_email_entry(
    email_cfg: crate::config::EmailConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    let policy = build_policy(
        email_cfg.security.autonomy_level,
        email_cfg.security.tool_allowlist.clone(),
        high_freedom_all_channels,
    );
    ChannelEntry {
        name: "Email",
        channel: Arc::new(EmailChannel::new(email_cfg)),
        policy,
    }
}

#[cfg(feature = "twitter")]
fn build_twitter_entry(
    tw: crate::config::schema::TwitterConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "Twitter",
        channel: std::sync::Arc::new(TwitterChannel::new(
            tw.client_id,
            tw.client_secret,
            tw.access_token,
            tw.refresh_token,
            tw.user_id,
            tw.allowed_users,
            tw.mention_poll_interval_secs,
            tw.dm_poll_interval_secs,
        )),
        policy: build_policy(
            tw.security.autonomy_level,
            tw.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[cfg(feature = "irc")]
fn build_irc_entry(
    irc: crate::config::schema::IrcConfig,
    high_freedom_all_channels: bool,
) -> ChannelEntry {
    ChannelEntry {
        name: "IRC",
        channel: Arc::new(IrcChannel::new(IrcChannelConfig {
            server: irc.server,
            port: irc.port,
            nickname: irc.nickname,
            username: irc.username,
            channels: irc.channels,
            allowed_users: irc.allowed_users,
            server_password: irc.server_password,
            nickserv_password: irc.nickserv_password,
            sasl_password: irc.sasl_password,
            verify_tls: irc.verify_tls.unwrap_or(true),
        })),
        policy: build_policy(
            irc.security.autonomy_level,
            irc.security.tool_allowlist,
            high_freedom_all_channels,
        ),
    }
}

#[must_use]
pub fn build_channels(
    channels_config: ChannelsConfig,
    #[allow(unused_variables)] security: &Arc<crate::security::SecurityPolicy>,
) -> Vec<ChannelEntry> {
    let high_freedom_all_channels = channels_config.high_freedom_all_channels;
    #[allow(unused_variables)]
    let telegram_enabled = channels_config.is_channel_enabled("telegram");
    let discord_enabled = channels_config.is_channel_enabled("discord");
    #[allow(unused_variables)]
    let slack_enabled = channels_config.is_channel_enabled("slack");
    #[allow(unused_variables)]
    let imessage_enabled = channels_config.is_channel_enabled("imessage");
    #[allow(unused_variables)]
    let matrix_enabled = channels_config.is_channel_enabled("matrix");
    #[allow(unused_variables)]
    let whatsapp_enabled = channels_config.is_channel_enabled("whatsapp");
    #[allow(unused_variables)]
    let email_enabled = channels_config.is_channel_enabled("email");
    #[allow(unused_variables)]
    let irc_enabled = channels_config.is_channel_enabled("irc");
    #[allow(unused_variables)]
    let twitter_enabled = channels_config.is_channel_enabled("twitter");
    let mut channels = Vec::with_capacity(9);

    #[cfg(feature = "telegram")]
    if telegram_enabled && let Some(tg) = channels_config.telegram {
        channels.push(build_telegram_entry(tg, high_freedom_all_channels));
    }

    #[cfg(feature = "discord")]
    if discord_enabled && let Some(dc) = channels_config.discord {
        channels.push(build_discord_entry(dc, high_freedom_all_channels));
    }

    #[cfg(feature = "slack")]
    if slack_enabled && let Some(sl) = channels_config.slack {
        channels.push(build_slack_entry(sl, high_freedom_all_channels));
    }

    #[cfg(feature = "imessage")]
    if imessage_enabled && let Some(im) = channels_config.imessage {
        channels.push(build_imessage_entry(
            im,
            security,
            high_freedom_all_channels,
        ));
    }

    #[cfg(feature = "matrix")]
    if matrix_enabled && let Some(mx) = channels_config.matrix {
        channels.push(build_matrix_entry(mx, high_freedom_all_channels));
    }

    #[cfg(feature = "whatsapp")]
    if whatsapp_enabled && let Some(wa) = channels_config.whatsapp {
        channels.push(build_whatsapp_entry(wa, high_freedom_all_channels));
    }

    #[cfg(feature = "email")]
    if email_enabled && let Some(email_cfg) = channels_config.email {
        channels.push(build_email_entry(email_cfg, high_freedom_all_channels));
    }

    #[cfg(feature = "irc")]
    if irc_enabled && let Some(irc) = channels_config.irc {
        channels.push(build_irc_entry(irc, high_freedom_all_channels));
    }

    #[cfg(feature = "twitter")]
    if twitter_enabled && let Some(tw) = channels_config.twitter {
        channels.push(build_twitter_entry(tw, high_freedom_all_channels));
    }

    channels
}

#[must_use]
pub fn has_listener_channels(config: &Config) -> bool {
    let security = Arc::new(crate::security::SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    ));
    !build_channels(config.channels_config.clone(), &security).is_empty()
}

#[cfg(test)]
mod tests {
    use super::build_policy;
    use crate::security::AutonomyLevel;

    #[test]
    fn build_policy_forces_unrestricted_when_high_freedom_is_enabled() {
        let policy = build_policy(
            Some(AutonomyLevel::ReadOnly),
            Some(vec!["read_file".to_string(), "list_files".to_string()]),
            true,
        );

        assert_eq!(policy.autonomy_level, None);
        assert_eq!(policy.tool_allowlist, None);
    }

    #[test]
    fn build_policy_keeps_channel_restrictions_when_high_freedom_is_disabled() {
        let policy = build_policy(
            Some(AutonomyLevel::ReadOnly),
            Some(vec!["read_file".to_string(), "list_files".to_string()]),
            false,
        );

        assert_eq!(policy.autonomy_level, Some(AutonomyLevel::ReadOnly));
        let allowlist = policy
            .tool_allowlist
            .as_ref()
            .expect("tool allowlist should be present when not overridden");
        assert!(allowlist.contains("read_file"));
        assert!(allowlist.contains("list_files"));
    }
}
