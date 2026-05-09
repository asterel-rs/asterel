//! Daemon status file writer.
//!
//! Periodically serializes a `DaemonStatus` snapshot to a JSON
//! file so external tools can query the daemon's liveness.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::config::Config;

#[derive(Debug, Clone, serde::Serialize)]
/// Serializable daemon health snapshot written to disk.
pub(super) struct DaemonStatus {
    #[serde(flatten)]
    snapshot: serde_json::Map<String, serde_json::Value>,
    written_at: String,
}

const STATUS_FLUSH_SECONDS: u64 = 5;

/// Returns the path to the daemon state JSON file.
#[must_use]
pub fn state_file_path(config: &Config) -> PathBuf {
    config.daemon_state_path()
}

/// Spawns a background task that periodically flushes daemon
/// health state to a JSON file.
pub(super) fn spawn_state_writer(config: Arc<Config>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let path = state_file_path(&config);
        if let Some(parent) = path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            tracing::warn!(%error, "failed to create state file directory");
        }

        let mut interval = tokio::time::interval(Duration::from_secs(STATUS_FLUSH_SECONDS));
        loop {
            interval.tick().await;
            let mut json = crate::runtime::diagnostics::health::snapshot_json();
            if let Some(snapshot) = json.as_object().cloned() {
                let status = DaemonStatus {
                    snapshot,
                    written_at: Utc::now().to_rfc3339(),
                };
                json = serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({}));
            }

            let data = serde_json::to_vec_pretty(&json).unwrap_or_else(|_| b"{}".to_vec());
            if let Err(error) = tokio::fs::write(&path, data).await {
                tracing::warn!(%error, "failed to write daemon state file");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn test_config(tmp: &TempDir) -> Config {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        config
    }

    #[test]
    fn state_file_path_uses_config_directory() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        let path = state_file_path(&config);
        assert_eq!(path, tmp.path().join("daemon_state.json"));
    }
}
