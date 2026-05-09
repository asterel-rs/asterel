//! CLI command dispatch and routing.
//!
//! Keeps top-level CLI routing thin while branch-specific handlers own each
//! subcommand family's behavior.

mod agent_handler;
mod eval_handlers;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use anyhow::{Result, bail};
use asterel::cli::commands::{Cli, Commands, ConfigCommands, SkillCommands};
use asterel::config::Config;
use tracing::info;

use self::agent_handler::dispatch_agent;
use self::eval_handlers::dispatch_eval;
use crate::app::status::render_status;

/// Route a parsed CLI invocation to the appropriate handler.
///
/// # Errors
///
/// Returns an error if the dispatched subcommand fails.
pub async fn dispatch(cli: Cli, config: Arc<Config>) -> Result<()> {
    if let Commands::Onboard { .. } = &cli.command {
        return dispatch_onboard(&cli.command).await;
    }

    ensure_runtime_setup(&cli.command, config.as_ref())?;

    dispatch_command(cli.command, config).await
}

async fn dispatch_onboard(command: &Commands) -> Result<()> {
    let Commands::Onboard {
        interactive,
        channels_only,
        api_key,
        provider,
        memory,
        postgres_setup,
        install_daemon,
    } = command
    else {
        unreachable!();
    };

    if *interactive && *channels_only {
        bail!("Use either --interactive or --channels-only, not both");
    }
    if *channels_only
        && (api_key.is_some() || provider.is_some() || memory.is_some() || postgres_setup.is_some())
    {
        bail!(
            "--channels-only does not accept --api-key, --provider, --memory, or --postgres-setup"
        );
    }

    let (config, autostart) = if *channels_only {
        asterel::onboard::run_channels_repair_wizard().await?
    } else if *interactive {
        asterel::onboard::run_wizard(*install_daemon, postgres_setup.as_deref()).await?
    } else {
        asterel::onboard::run_quick_setup(
            api_key.as_deref(),
            provider.as_deref(),
            memory.as_deref(),
            postgres_setup.as_deref(),
            *install_daemon,
        )?
    };
    if autostart {
        asterel::transport::channels::start_channels(Arc::new(config)).await?;
    }
    Ok(())
}

fn ensure_runtime_setup(command: &Commands, config: &Config) -> Result<()> {
    let provider_override = match command {
        Commands::Agent { provider, .. } => provider.as_deref(),
        _ => None,
    };

    if matches!(
        command,
        Commands::Agent { .. } | Commands::Gateway { .. } | Commands::Daemon { .. }
    ) && asterel::runtime::services::runtime_boot_requires_onboarding_for_provider(
        config,
        provider_override,
    ) {
        let command_label = match command {
            Commands::Agent { .. } => "agent",
            Commands::Gateway { .. } => "gateway",
            Commands::Daemon { .. } => "daemon",
            _ => unreachable!(),
        };
        bail!(
            "Asterel is not onboarded yet. Run `asterel onboard` before starting `{command_label}`."
        );
    }

    Ok(())
}

async fn dispatch_command(command: Commands, config: Arc<Config>) -> Result<()> {
    match command {
        Commands::Onboard { .. } => unreachable!(),

        Commands::Agent {
            message,
            provider,
            model,
            temperature,
        } => dispatch_agent(config, message, provider, model, temperature).await,

        Commands::Gateway { port, host } => dispatch_gateway(config, host, port).await,

        Commands::Daemon { port, host } => dispatch_daemon(config, host, port).await,

        Commands::Status => {
            println!("{}", render_status(&config));
            Ok(())
        }

        Commands::Eval { eval_command } => dispatch_eval(&config, eval_command).await,

        Commands::Model { set, provider } => dispatch_model(set, provider.as_deref(), &config),

        Commands::Cron { cron_command } => {
            super::cron_display::handle_cron_command(cron_command, &config)
        }

        Commands::Service { service_command } => {
            asterel::platform::service::handle_command(&service_command, &config)
        }

        Commands::Doctor { repair } => asterel::runtime::diagnostics::doctor::run(&config, repair),

        Commands::Config { config_command } => dispatch_config(&config_command, &config),

        Commands::Channel { channel_command } => dispatch_channel(channel_command, config).await,

        Commands::Integrations {
            integration_command,
        } => asterel::plugins::integrations::handle_command(integration_command, &config),

        Commands::Auth { auth_command } => {
            asterel::cli::auth_handler::handle_command(auth_command, &config)
        }

        Commands::Skills { skill_command } => dispatch_skills(skill_command, &config),
    }
}

async fn dispatch_gateway(config: Arc<Config>, host: String, port: u16) -> Result<()> {
    let bind = asterel::runtime::services::RuntimeBindAddress::new(host, port);
    info!("🚀 Starting Asterel Gateway on {}", bind.display());
    asterel::runtime::services::run_gateway_surface(config, bind).await
}

async fn dispatch_daemon(config: Arc<Config>, host: String, port: u16) -> Result<()> {
    let bind = asterel::runtime::services::RuntimeBindAddress::new(host, port);
    info!("🧠 Starting Asterel Daemon on {}", bind.display());
    asterel::runtime::services::run_daemon_surface(config, bind).await
}

fn dispatch_model(set: String, provider: Option<&str>, config: &Config) -> Result<()> {
    let mut updated = config.clone();
    let (model, effective_provider) = updated.update_model_defaults(set, provider)?;
    println!();
    println!("  {}", asterel::ui::style::section("Model Defaults"));
    println!(
        "{}",
        asterel::ui::style::field_line("Result", asterel::ui::style::ok_badge("updated"))
    );
    println!(
        "{}",
        asterel::ui::style::field_line(
            "Provider",
            effective_provider.as_deref().unwrap_or("(unset)")
        )
    );
    println!("{}", asterel::ui::style::field_line("Model", model));
    println!(
        "{}",
        asterel::ui::style::field_line("Config", updated.config_path.display())
    );
    Ok(())
}

fn dispatch_config(config_command: &ConfigCommands, config: &Config) -> Result<()> {
    match config_command {
        ConfigCommands::Validate => {
            config.validate_autonomy_controls()?;
            println!();
            println!("  {}", asterel::ui::style::section("Config Validation"));
            println!(
                "{}",
                asterel::ui::style::field_line("Result", asterel::ui::style::ok_badge("valid"))
            );
            println!(
                "{}",
                asterel::ui::style::field_line("Config", config.config_path.display())
            );
            Ok(())
        }
    }
}

async fn dispatch_channel(
    channel_command: asterel::ChannelCommands,
    config: Arc<Config>,
) -> Result<()> {
    match channel_command {
        asterel::ChannelCommands::Start => {
            asterel::transport::channels::start_channels(config).await
        }
        asterel::ChannelCommands::Doctor => {
            asterel::transport::channels::doctor_channels(config).await
        }
        asterel::ChannelCommands::List => {
            asterel::transport::channels::handle_command(asterel::ChannelCommands::List, &config)
        }
    }
}

fn dispatch_skills(skill_command: SkillCommands, config: &Config) -> Result<()> {
    let security = asterel::security::SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );
    asterel::plugins::skills::handle_command(
        skill_command,
        &config.workspace_dir,
        &security,
        &config.skills,
    )
}
