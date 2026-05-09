//! Interactive CLI prompts for tool-mode and secrets configuration.
//!
//! Configures Composio integration and secret-store preferences
//! during onboarding.

use anyhow::Result;

use super::super::view::print_bullet;
use crate::config::{ComposioConfig, SecretsConfig};
use crate::ui::style as ui;

/// # Errors
///
/// Returns an error when interactive prompt input fails.
pub(crate) fn setup_tool_mode() -> Result<(ComposioConfig, SecretsConfig)> {
    print_bullet(&t!("onboard.tool_mode.intro"));
    print_bullet(&t!("onboard.tool_mode.later_hint"));
    println!();

    let choice: usize = cliclack::select(format!("  {}", t!("onboard.tool_mode.select_prompt")))
        .item(0usize, t!("onboard.tool_mode.sovereign").to_string(), "")
        .item(1usize, t!("onboard.tool_mode.composio").to_string(), "")
        .initial_value(0usize)
        .interact()?;

    let composio_config = if choice == 1 {
        println!();
        println!(
            "  {} {}",
            ui::header(t!("onboard.tool_mode.composio_title")),
            ui::dim(format!("— {}", t!("onboard.tool_mode.composio_subtitle")))
        );
        print_bullet(&t!("onboard.tool_mode.composio_url_hint"));
        print_bullet(&t!("onboard.tool_mode.composio_desc"));
        println!();

        let api_key: String =
            cliclack::input(format!("  {}", t!("onboard.tool_mode.composio_key_prompt")))
                .required(false)
                .interact()?;

        if api_key.trim().is_empty() {
            println!(
                "  {} {}",
                ui::dim("→"),
                t!("onboard.tool_mode.composio_skipped")
            );
            ComposioConfig::default()
        } else {
            println!(
                "  {} {}",
                ui::success("✓"),
                t!("onboard.tool_mode.composio_confirm")
            );
            ComposioConfig {
                enabled: true,
                api_key: Some(api_key),
                ..ComposioConfig::default()
            }
        }
    } else {
        println!(
            "  {} {}",
            ui::success("✓"),
            t!("onboard.tool_mode.sovereign_confirm")
        );
        ComposioConfig::default()
    };

    // ── Encrypted secrets ──
    println!();
    print_bullet(&t!("onboard.tool_mode.encrypt_intro"));
    print_bullet(&t!("onboard.tool_mode.encrypt_desc"));

    let encrypt: bool = cliclack::confirm(format!("  {}", t!("onboard.tool_mode.encrypt_prompt")))
        .initial_value(true)
        .interact()?;

    let secrets_config = SecretsConfig { encrypt };

    if encrypt {
        println!(
            "  {} {}",
            ui::success("✓"),
            t!("onboard.tool_mode.encrypt_on")
        );
    } else {
        println!(
            "  {} {}",
            ui::success("✓"),
            t!("onboard.tool_mode.encrypt_off")
        );
    }

    Ok((composio_config, secrets_config))
}
