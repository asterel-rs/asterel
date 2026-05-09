//! CLI subcommand dispatcher for channel operations (`list`, `start`,
//! `doctor`). Async-requiring commands bail back to the caller.
use anyhow::Result;

use crate::config::Config;
use crate::runtime::services::load_runtime_operational_snapshot;
use crate::ui::style as ui;

/// # Errors
///
/// Returns an error for channel subcommands that require async runtime handling
/// outside this sync dispatch path.
#[allow(clippy::needless_pass_by_value)]
pub fn handle_command(command: crate::ChannelCommands, config: &Config) -> Result<()> {
    match command {
        crate::ChannelCommands::Start => {
            anyhow::bail!("Start must be handled in main.rs (requires async runtime)")
        }
        crate::ChannelCommands::Doctor => {
            anyhow::bail!("Doctor must be handled in main.rs (requires async runtime)")
        }
        crate::ChannelCommands::List => {
            let operational = load_runtime_operational_snapshot(config);
            println!();
            println!("  {}", ui::section("Channels"));
            for channel in operational.channels {
                println!(
                    "{}",
                    ui::field_line(
                        channel.label,
                        if channel.enabled {
                            ui::ok_badge("configured")
                        } else if channel.configured {
                            ui::warn_badge("configured but disabled")
                        } else {
                            ui::muted_badge("not configured")
                        }
                    )
                );
            }
            println!();
            println!("{}", ui::note_line(t!("channels.to_start")));
            println!("{}", ui::command_line("asterel channel start"));
            println!("{}", ui::note_line(t!("channels.to_check")));
            println!("{}", ui::command_line("asterel channel doctor"));
            println!("{}", ui::note_line(t!("channels.to_configure")));
            println!("{}", ui::command_line("asterel onboard --channels-only"));
            Ok(())
        }
    }
}
