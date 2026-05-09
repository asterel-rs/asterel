//! Interactive CLI prompts for workspace directory setup.
//!
//! Resolves the home directory, prompts for workspace path,
//! and creates the directory structure if it does not exist.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::super::domain::validate_non_empty;
use super::super::view::print_bullet;
use crate::ui::style as ui;

/// # Errors
///
/// Returns an error when home/workspace paths cannot be resolved, validated, or
/// created.
pub(crate) fn setup_workspace() -> Result<(PathBuf, PathBuf)> {
    let default_dir = crate::utils::dirs::asterel_home_dir()?;

    print_bullet(&t!(
        "onboard.workspace.default_location",
        path = ui::value(default_dir.display())
    ));

    let use_default: bool = cliclack::confirm(format!("  {}", t!("onboard.workspace.use_default")))
        .initial_value(true)
        .interact()?;

    let asterel_dir = if use_default {
        default_dir
    } else {
        let custom = input_workspace_path()?;
        PathBuf::from(custom)
    };

    let workspace_dir = asterel_dir.join("workspace");
    let config_path = asterel_dir.join("config.toml");

    fs::create_dir_all(&workspace_dir).context("Failed to create workspace directory")?;

    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.workspace.confirm",
            path = ui::value(workspace_dir.display())
        )
    );

    Ok((workspace_dir, config_path))
}

fn input_workspace_path() -> Result<String> {
    let custom: String =
        cliclack::input(format!("  {}", t!("onboard.workspace.enter_path"))).interact()?;
    let expanded = shellexpand::tilde(&custom).to_string();
    validate_non_empty("workspace path", &expanded)
}
