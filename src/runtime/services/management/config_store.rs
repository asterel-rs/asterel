use anyhow::{Context, Result};

use super::{ManagedChannelRecord, ManagedRuntimeOwner, RuntimeApplyMode};
use crate::config::Config;

pub(super) fn load_persisted_runtime_config(current: &Config) -> Result<Config> {
    if !current.config_path.exists() {
        return Ok(current.clone());
    }

    let raw = std::fs::read_to_string(&current.config_path)
        .with_context(|| format!("read runtime config '{}'", current.config_path.display()))?;
    let mut config: Config = toml::from_str(&raw)
        .with_context(|| format!("parse '{}'", current.config_path.display()))?;
    config.config_path.clone_from(&current.config_path);
    config.workspace_dir.clone_from(&current.workspace_dir);
    Ok(config)
}

pub(super) fn save_persisted_runtime_config(config: &Config) -> Result<()> {
    config
        .validate_autonomy_controls()
        .context("validate persisted runtime config")?;
    config.save().context("save persisted runtime config")
}

pub(super) fn runtime_apply_mode(config: &Config) -> RuntimeApplyMode {
    if config.runtime.enable_live_settings_reload {
        RuntimeApplyMode::DaemonLiveReload
    } else {
        RuntimeApplyMode::RestartRequired
    }
}

pub(super) fn maybe_request_channel_surface_reload(
    config: &Config,
    record: &ManagedChannelRecord,
) -> bool {
    config.runtime.enable_live_settings_reload
        && matches!(record.owner, ManagedRuntimeOwner::ChannelsSurface)
        && crate::transport::channels::request_channel_surface_reload()
}
