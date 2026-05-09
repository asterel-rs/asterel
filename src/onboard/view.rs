//! Terminal output helpers for the CLI onboarding wizard.
//!
//! Renders banners, step progress, configuration summaries,
//! and styled bullet points to stdout.

use super::domain::provider_env_var;
use crate::config::Config;
use crate::ui::style as ui;

/// Prints the ASCII art welcome banner to stdout.
pub(crate) fn print_welcome_banner() {
    println!("{}", ui::accent(t!("onboard.banner.art")));

    println!("  {}", ui::header(t!("onboard.banner.welcome")));
    println!("  {}", ui::dim(t!("onboard.banner.subtitle")));
    println!();
}

/// Prints a numbered step header (e.g. `[2/6] Provider`).
pub(crate) fn print_step(current: u8, total: u8, title: &str) {
    println!();
    println!(
        "  {} {}",
        ui::accent(format!("[{current}/{total}]")),
        ui::header(title)
    );
    println!("  {}", ui::dim("─".repeat(50)));
}

/// Prints a single styled bullet-point line.
pub(crate) fn print_bullet(text: &str) {
    println!("  {} {}", ui::cyan("›"), text);
}
/// Prints the full post-onboarding configuration summary.
pub(crate) fn print_summary(config: &Config) {
    print_summary_header(config);
    print_summary_details(config);
    print_summary_next_steps(config);
}

fn print_summary_header(config: &Config) {
    println!();
    println!(
        "  {}",
        ui::cyan("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    );
    println!("  ◆  {}", ui::header(t!("onboard.summary.ready")));
    println!(
        "  {}",
        ui::cyan("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    );
    println!();

    println!("  {}", ui::dim(t!("onboard.summary.config_saved")));
    println!("    {}", ui::value(config.config_path.display()));
    println!();
}

fn print_summary_details(config: &Config) {
    println!("  {}", ui::header(t!("onboard.summary.quick_summary")));
    println!(
        "    › {} {}",
        t!("onboard.summary.provider"),
        config
            .default_provider
            .as_deref()
            .unwrap_or(crate::config::DEFAULT_PROVIDER)
    );
    println!(
        "    › {} {}",
        t!("onboard.summary.model"),
        config.default_model.as_deref().unwrap_or("(default)")
    );
    println!(
        "    › {} {:?}",
        t!("onboard.summary.autonomy"),
        config.autonomy.effective_autonomy_lvl()
    );
    println!(
        "    › {} {} (auto-save: {})",
        t!("onboard.summary.memory"),
        config.memory.backend,
        if config.memory.auto_save { "on" } else { "off" }
    );

    println!(
        "    › {} {}",
        t!("onboard.summary.channels"),
        configured_channels_list(config).join(", ")
    );

    println!(
        "    › {} {}",
        t!("onboard.summary.api_key"),
        if config.api_key.is_some() {
            ui::value(t!("onboard.summary.api_key_set"))
        } else {
            ui::yellow(t!("onboard.summary.api_key_not_set"))
        }
    );

    println!(
        "    › {} {}",
        t!("onboard.summary.tunnel"),
        if config.tunnel.provider == crate::config::TunnelProvider::None {
            t!("onboard.summary.tunnel_none").to_string()
        } else {
            config.tunnel.provider.to_string()
        }
    );

    println!(
        "    › {} {}",
        t!("onboard.summary.composio"),
        if config.composio.enabled {
            ui::value(t!("onboard.summary.composio_enabled"))
        } else {
            t!("onboard.summary.composio_disabled").to_string()
        }
    );

    println!(
        "    › {} {}",
        t!("onboard.summary.secrets"),
        if config.secrets.encrypt {
            ui::value(t!("onboard.summary.secrets_encrypted"))
        } else {
            ui::yellow(t!("onboard.summary.secrets_plaintext"))
        }
    );

    println!(
        "    › {} {}",
        t!("onboard.summary.gateway"),
        if config.gateway.require_pairing {
            t!("onboard.summary.gateway_pairing")
        } else {
            t!("onboard.summary.gateway_no_pairing")
        }
    );
}

fn configured_channels_list(config: &Config) -> Vec<&'static str> {
    let mut channels: Vec<&str> = vec!["CLI"];
    if config.channels_config.telegram.is_some() {
        channels.push("Telegram");
    }
    if config.channels_config.discord.is_some() {
        channels.push("Discord");
    }
    if config.channels_config.slack.is_some() {
        channels.push("Slack");
    }
    if config.channels_config.imessage.is_some() {
        channels.push("iMessage");
    }
    if config.channels_config.matrix.is_some() {
        channels.push("Matrix");
    }
    if config.channels_config.email.is_some() {
        channels.push("Email");
    }
    if config.channels_config.webhook.is_some() {
        channels.push("Webhook");
    }
    channels
}

fn print_summary_next_steps(config: &Config) {
    let has_channels = config.channels_config.telegram.is_some()
        || config.channels_config.discord.is_some()
        || config.channels_config.slack.is_some()
        || config.channels_config.imessage.is_some()
        || config.channels_config.matrix.is_some()
        || config.channels_config.email.is_some();

    println!();
    println!("  {}", ui::header(t!("onboard.summary.next_steps")));
    println!();

    let mut step = 1u8;

    if config.api_key.is_none() {
        let env_var = provider_env_var(
            config
                .default_provider
                .as_deref()
                .unwrap_or(crate::config::DEFAULT_PROVIDER),
        );
        println!(
            "    {} {}",
            ui::accent(format!("{step}.")),
            t!("onboard.summary.set_api_key")
        );
        println!(
            "       {}",
            ui::yellow(format!("export {env_var}=\"sk-...\""))
        );
        println!();
        step += 1;
    }

    if has_channels {
        println!(
            "    {} {} {}",
            ui::accent(format!("{step}.")),
            ui::header(t!("onboard.summary.launch_channels")),
            t!("onboard.summary.launch_channels_hint")
        );
        println!("       {}", ui::yellow("asterel channel start"));
        println!();
        step += 1;
    }

    println!(
        "    {} {}",
        ui::accent(format!("{step}.")),
        t!("onboard.summary.send_message")
    );
    println!(
        "       {}",
        ui::yellow("asterel agent -m \"Hello, Asterel!\"")
    );
    println!();
    step += 1;

    println!(
        "    {} {}",
        ui::accent(format!("{step}.")),
        t!("onboard.summary.interactive_cli")
    );
    println!("       {}", ui::yellow("asterel agent"));
    println!();
    step += 1;

    println!(
        "    {} {}",
        ui::accent(format!("{step}.")),
        t!("onboard.summary.check_status")
    );
    println!("       {}", ui::yellow("asterel status"));

    println!();
    println!("  ◆ {}", ui::header(t!("onboard.summary.happy_hacking")));
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelSecurityPolicy, Config, TelegramConfig};

    #[test]
    fn happy_path_completion_summary_renders_with_api_key_and_channel() {
        let mut config = Config {
            api_key: Some("sk-test".to_string()),
            default_provider: Some("openrouter".to_string()),
            default_model: Some("anthropic/claude-sonnet-4.6".to_string()),
            ..Config::default()
        };
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "bot-token".to_string(),
            allowed_users: Vec::new(),
            default_account: None,
            default_to: None,
            security: ChannelSecurityPolicy::default(),
        });

        let channels = configured_channels_list(&config);
        assert_eq!(channels, vec!["CLI", "Telegram"]);

        print_summary(&config);
    }

    #[test]
    fn abort_cancel_mid_flow_summary_renders_without_api_key() {
        let config = Config {
            api_key: None,
            default_provider: Some("openrouter".to_string()),
            ..Config::default()
        };

        let channels = configured_channels_list(&config);
        assert_eq!(channels, vec!["CLI"]);

        print_summary(&config);
    }
}
