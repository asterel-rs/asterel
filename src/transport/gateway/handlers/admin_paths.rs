use crate::config::Config;
use std::path::PathBuf;

fn gateway_admin_state_dir(config: &Config) -> PathBuf {
    config.workspace_dir.join(".asterel").join("gateway")
}

pub(super) fn admin_uploads_dir(config: &Config) -> PathBuf {
    gateway_admin_state_dir(config).join("uploads")
}
