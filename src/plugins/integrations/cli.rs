//! CLI handler for the `integrations` command.
//!
//! Displays integration status and configuration details for a
//! named integration.

use anyhow::Result;

use crate::config::Config;
use crate::plugins::integrations::{IntegrationStatus, registry};
use crate::ui::style as ui;

/// Handle the `integrations` CLI command
///
/// # Errors
///
/// Returns an error when integration lookup fails or integration status output
/// cannot be produced.
pub fn handle_command(command: crate::IntegrationCommands, config: &Config) -> Result<()> {
    match command {
        crate::IntegrationCommands::Info { name } => show_integration_info(config, &name),
    }
}

fn print_integration_block(title: &str, notes: &[&str], commands: &[&str]) {
    println!("  {}", ui::subsection(title));
    for note in notes {
        println!("{}", ui::note_line(note));
    }
    for command in commands {
        println!("{}", ui::command_line(command));
    }
}

fn render_integration_hints(config: &Config, name: &str) {
    match name {
        "Telegram" => print_integration_block(
            "Setup",
            &[
                "1. Message @BotFather on Telegram",
                "2. Create a bot and copy the token",
                "3. Run onboarding",
                "4. Start channels",
            ],
            &["asterel onboard", "asterel channel start"],
        ),
        "Discord" => print_integration_block(
            "Setup",
            &[
                "1. Go to https://discord.com/developers/applications",
                "2. Create app -> Bot -> Copy token",
                "3. Enable MESSAGE CONTENT intent",
            ],
            &["asterel onboard"],
        ),
        "Slack" => print_integration_block(
            "Setup",
            &[
                "1. Go to https://api.slack.com/apps",
                "2. Create app -> Bot Token Scopes -> Install",
            ],
            &["asterel onboard"],
        ),
        "OpenRouter" => print_integration_block(
            "Setup",
            &[
                "1. Get API key at https://openrouter.ai/keys",
                "Access 200+ models with one key.",
            ],
            &["asterel onboard"],
        ),
        "Ollama" => print_integration_block(
            "Setup",
            &["Set provider to 'ollama' in config.toml."],
            &["brew install ollama", "ollama pull llama3"],
        ),
        "iMessage" => print_integration_block(
            "Setup (macOS only)",
            &[
                "Uses AppleScript bridge to send/receive iMessages.",
                "Requires Full Disk Access in System Settings -> Privacy.",
            ],
            &[],
        ),
        "Browser" => print_integration_block(
            "Built-in",
            &[
                "Asterel can control Chrome/Chromium for web tasks.",
                "Uses headless browser automation.",
            ],
            &[],
        ),
        "Cron" => {
            let note = cron_runtime_hint(config);
            println!("  {}", ui::subsection("Built-in"));
            println!("{}", ui::note_line(note));
            println!("{}", ui::command_line("asterel cron list"));
        }
        "Webhooks" => print_integration_block(
            "Built-in",
            &["HTTP endpoint for external triggers."],
            &["asterel gateway"],
        ),
        _ => {}
    }
}

fn show_integration_info(config: &Config, name: &str) -> Result<()> {
    let entries = registry::all_integrations();
    let name_lower = name.to_lowercase();

    let Some(entry) = entries.iter().find(|e| e.name.to_lowercase() == name_lower) else {
        anyhow::bail!(
            "Unknown integration: {name}. Check README for supported integrations or run `asterel onboard --interactive` to configure channels/providers."
        );
    };

    let status = (entry.status_fn)(config);
    let label = match &status {
        IntegrationStatus::Active => ui::ok_badge("active"),
        IntegrationStatus::Available => ui::muted_badge("available"),
    };

    println!();
    println!("  {}", ui::section(format!("Integration: {}", entry.name)));
    println!("{}", ui::field_line("Category", entry.category.label()));
    println!("{}", ui::field_line("Status", label));
    println!("{}", ui::field_line("Summary", entry.description));
    println!();

    render_integration_hints(config, entry.name);

    println!();
    Ok(())
}

fn cron_runtime_hint(config: &Config) -> String {
    let cron = crate::runtime::services::load_runtime_operational_snapshot(config).cron;
    match cron.status {
        crate::runtime::services::RuntimeCapabilityStatus::Supported => {
            "Cron uses PostgreSQL-backed scheduler state and is available in the current setup."
                .to_string()
        }
        crate::runtime::services::RuntimeCapabilityStatus::Degraded => format!(
            "Cron uses PostgreSQL-backed scheduler state and is degraded: {}.",
            cron.reason.as_deref().unwrap_or("configuration incomplete")
        ),
        crate::runtime::services::RuntimeCapabilityStatus::Unsupported => format!(
            "Cron uses PostgreSQL-backed scheduler state and is unavailable here: {}.",
            cron.reason
                .as_deref()
                .unwrap_or("PostgreSQL support missing")
        ),
    }
}
