//! Shared onboarding completion and post-action helpers.
//!
//! Keeps config finalization, service install, and channel launch
//! follow-ups consistent across interactive CLI onboarding, repair, and quick setup.

use anyhow::Result;

use super::config_builder::OnboardingPlan;
use crate::config::{Config, DEFAULT_PROVIDER};
use crate::security::auth::AuthBroker;
use crate::ui::style as ui;

#[derive(Debug, Clone, Default)]
pub(crate) struct OnboardingPostActionOptions {
    pub(crate) install_daemon_flag: bool,
    pub(crate) interactive_followups: bool,
    pub(crate) skip_followup_notice: Option<&'static str>,
    pub(crate) allow_channel_launch_prompt: bool,
}

/// Finalize a shared onboarding plan and run any configured post-actions.
///
/// # Errors
///
/// Returns an error when config persistence or a post-action fails.
pub(crate) fn finalize_plan_with_post_actions(
    plan: OnboardingPlan,
    options: &OnboardingPostActionOptions,
) -> Result<(Config, bool)> {
    let config = plan.finalize()?;
    let autostart = run_post_actions(&config, options)?;
    Ok((config, autostart))
}

/// Apply post-onboarding actions to an already persisted config.
///
/// # Errors
///
/// Returns an error when service installation or interactive prompts fail.
pub(crate) fn run_post_actions(
    config: &Config,
    options: &OnboardingPostActionOptions,
) -> Result<bool> {
    install_daemon_if_requested(config, options)?;

    if !options.interactive_followups {
        if let Some(message) = options.skip_followup_notice {
            println!("  {} {}", ui::dim("›"), message);
        }
        return Ok(false);
    }

    maybe_offer_daemon_install(config, options)?;

    if options.allow_channel_launch_prompt {
        offer_launch_channels(config)
    } else {
        Ok(false)
    }
}

fn install_daemon_if_requested(
    config: &Config,
    options: &OnboardingPostActionOptions,
) -> Result<()> {
    if options.install_daemon_flag {
        crate::platform::service::handle_command(&crate::ServiceCommands::Install, config)?;
        println!("  {} Daemon installed as OS service", ui::success("✓"));
    }
    Ok(())
}

fn maybe_offer_daemon_install(
    config: &Config,
    options: &OnboardingPostActionOptions,
) -> Result<()> {
    if options.install_daemon_flag || !stdin_and_stdout_are_tty() {
        if !options.install_daemon_flag && !stdin_and_stdout_are_tty() {
            println!(
                "  {} Non-interactive mode detected; skipping OS service install prompt.",
                ui::dim("›")
            );
        }
        return Ok(());
    }

    let install: bool =
        cliclack::confirm("  › Install Asterel as an OS service (auto-start on boot)?")
            .initial_value(false)
            .interact()?;

    if install {
        crate::platform::service::handle_command(&crate::ServiceCommands::Install, config)?;
        println!("  {} Daemon installed as OS service", ui::success("✓"));
    } else {
        println!(
            "  {} You can install later with: asterel service install",
            ui::dim("›")
        );
    }

    Ok(())
}

fn offer_launch_channels(config: &Config) -> Result<bool> {
    if !has_launchable_channels(config) || !has_primary_credential(config) {
        return Ok(false);
    }

    if !stdin_and_stdout_are_tty() {
        println!(
            "  {} Non-interactive mode detected; skipping channel auto-start prompt.",
            ui::dim("›")
        );
        return Ok(false);
    }

    let launch: bool = cliclack::confirm(format!("  › {}", t!("onboard.launch_prompt")))
        .initial_value(true)
        .interact()?;

    if launch {
        println!();
        println!("  › {}", ui::header(t!("onboard.launching")));
        println!();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn has_launchable_channels(config: &Config) -> bool {
    config.channels_config.telegram.is_some()
        || config.channels_config.discord.is_some()
        || config.channels_config.slack.is_some()
        || config.channels_config.imessage.is_some()
        || config.channels_config.matrix.is_some()
        || config.channels_config.email.is_some()
}

fn has_primary_credential(config: &Config) -> bool {
    let provider = config
        .default_provider
        .as_deref()
        .unwrap_or(DEFAULT_PROVIDER);
    let config_api_key_present = config
        .api_key
        .as_deref()
        .is_some_and(|api_key| !api_key.trim().is_empty());

    config_api_key_present
        || AuthBroker::load_or_init(config)
            .ok()
            .and_then(|broker| broker.resolve_provider_key(provider))
            .is_some()
}

fn stdin_and_stdout_are_tty() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{OnboardingPostActionOptions, has_primary_credential, run_post_actions};
    use crate::config::{ChannelSecurityPolicy, Config, TelegramConfig};
    use crate::security::auth::{AuthProfile, AuthProfileStore};

    #[test]
    fn post_actions_skip_launch_without_channels() {
        let config = Config {
            api_key: Some("sk-test".to_string()),
            ..Config::default()
        };

        let autostart = run_post_actions(
            &config,
            &OnboardingPostActionOptions {
                interactive_followups: true,
                allow_channel_launch_prompt: true,
                ..OnboardingPostActionOptions::default()
            },
        )
        .expect("post actions should succeed");

        assert!(!autostart);
    }

    #[test]
    fn post_actions_skip_launch_without_credentials() {
        let mut config = Config::default();
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "bot-token".to_string(),
            allowed_users: Vec::new(),
            default_account: None,
            default_to: None,
            security: ChannelSecurityPolicy::default(),
        });

        let autostart = run_post_actions(
            &config,
            &OnboardingPostActionOptions {
                interactive_followups: true,
                allow_channel_launch_prompt: true,
                ..OnboardingPostActionOptions::default()
            },
        )
        .expect("post actions should succeed");

        assert!(!autostart);
    }

    #[test]
    fn oauth_only_profile_counts_as_primary_credential() {
        let tmp = TempDir::new().expect("temp dir");
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            default_provider: Some("openai-codex".to_string()),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");

        let mut store = AuthProfileStore::default();
        store
            .upsert_profile(
                AuthProfile {
                    id: "codex-default".to_string(),
                    provider: "openai".to_string(),
                    auth_route: Some("codex".to_string()),
                    label: Some("Codex".to_string()),
                    api_key: Some("oauth-token".to_string()),
                    refresh_token: None,
                    auth_scheme: Some("oauth".to_string()),
                    oauth_source: Some("codex".to_string()),
                    is_disabled: false,
                },
                true,
            )
            .expect("upsert profile");
        store.save_for_config(&config).expect("save auth store");

        assert!(has_primary_credential(&config));
    }
}
