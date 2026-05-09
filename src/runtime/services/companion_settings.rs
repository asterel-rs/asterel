//! Companion settings persistence service.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::contracts::channels::GatewayCompanionSettings;

fn gateway_admin_state_dir(config: &Config) -> PathBuf {
    config.workspace_dir.join(".asterel").join("gateway")
}

#[must_use]
pub fn companion_settings_path(config: &Config) -> PathBuf {
    gateway_admin_state_dir(config).join("companion-settings.json")
}

#[allow(clippy::missing_errors_doc)]
pub fn load_companion_admin_settings(config: &Config) -> Result<GatewayCompanionSettings> {
    load_json_or_default(&companion_settings_path(config))
}

#[allow(clippy::missing_errors_doc)]
pub fn save_companion_admin_settings(
    config: &Config,
    settings: &GatewayCompanionSettings,
) -> Result<()> {
    save_json(&companion_settings_path(config), settings)
}

fn load_json_or_default<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read persisted admin state at {}", path.display()))?;

    serde_json::from_str(&raw)
        .with_context(|| format!("parse persisted admin state at {}", path.display()))
}

fn save_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create admin state directory {}", parent.display()))?;
    }

    let serialized = serde_json::to_vec_pretty(value)
        .with_context(|| format!("serialize persisted admin state for {}", path.display()))?;

    std::fs::write(path, serialized)
        .with_context(|| format!("write persisted admin state to {}", path.display()))
}
