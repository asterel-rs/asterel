//! Hot-reload detection for daemon configuration changes.
//!
//! Compares config file timestamps and diffs section-level
//! changes to decide whether a reload should be applied.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::config::Config;

/// Result of comparing old and new daemon configurations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReloadDecision {
    /// The configurations are identical; no action needed.
    NoChanges,
    /// The configurations differ in the listed top-level sections.
    Apply { changed_sections: Vec<String> },
}

/// Returns the last-modified timestamp of the config file, or
/// `None` if the metadata cannot be read.
pub(super) fn config_modified_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Loads and validates a fresh config from the same path as
/// `current`.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub(super) fn load_candidate_config(current: &Config) -> Result<Config> {
    Config::load_from_path(&current.config_path, &current.workspace_dir)
        .with_context(|| format!("reload config from {}", current.config_path.display()))
}

/// Compares two configs and returns which top-level sections
/// changed.
///
/// # Errors
///
/// Returns an error if the configs cannot be serialized.
pub(super) fn evaluate_reload(current: &Config, candidate: &Config) -> Result<ReloadDecision> {
    let current_json = serde_json::to_value(current).context("serialize current config")?;
    let candidate_json = serde_json::to_value(candidate).context("serialize candidate config")?;

    if current_json == candidate_json {
        return Ok(ReloadDecision::NoChanges);
    }

    let current_obj = current_json
        .as_object()
        .context("serialized current config is not an object")?;
    let candidate_obj = candidate_json
        .as_object()
        .context("serialized candidate config is not an object")?;

    let mut changed = BTreeSet::new();

    for (key, candidate_value) in candidate_obj {
        if current_obj.get(key) != Some(candidate_value) {
            changed.insert(key.clone());
        }
    }
    for key in current_obj.keys() {
        if !candidate_obj.contains_key(key) {
            changed.insert(key.clone());
        }
    }

    Ok(ReloadDecision::Apply {
        changed_sections: changed.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{ReloadDecision, evaluate_reload, load_candidate_config};
    use crate::config::Config;

    fn config_with_paths(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    fn write_config(config: &Config) {
        let toml = toml::to_string_pretty(config).expect("serialize config to toml");
        std::fs::create_dir_all(&config.workspace_dir).expect("create workspace directory");
        std::fs::write(&config.config_path, toml).expect("write config file");
    }

    #[test]
    fn evaluate_reload_returns_no_changes_for_identical_configs() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);

        let decision = evaluate_reload(&current, &current).expect("evaluate identical config");
        assert_eq!(decision, ReloadDecision::NoChanges);
    }

    #[test]
    fn evaluate_reload_reports_changed_top_level_sections() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);
        let mut candidate = current.clone();
        candidate.gateway.defense_kill_switch = true;
        candidate.reliability.scheduler_poll_secs =
            candidate.reliability.scheduler_poll_secs.saturating_add(5);

        let decision = evaluate_reload(&current, &candidate).expect("evaluate changed config");
        assert_eq!(
            decision,
            ReloadDecision::Apply {
                changed_sections: vec!["gateway".to_string(), "reliability".to_string()],
            }
        );
    }

    #[test]
    fn load_candidate_config_rejects_invalid_reliability_controls() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);
        let mut invalid = current.clone();
        invalid.reliability.scheduler_active_hours_start_utc = Some("09:00".to_string());
        invalid.reliability.scheduler_active_hours_end_utc = None;
        write_config(&invalid);

        let error =
            load_candidate_config(&current).expect_err("invalid candidate should be rejected");
        assert!(error.to_string().contains("reload config from"));
    }

    #[test]
    fn successful_reload_candidate_loads_and_evaluates_to_apply() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);
        let mut candidate = current.clone();
        candidate.runtime.enable_live_settings_reload =
            !candidate.runtime.enable_live_settings_reload;
        write_config(&candidate);

        let loaded_candidate =
            load_candidate_config(&current).expect("candidate config should load successfully");
        let decision =
            evaluate_reload(&current, &loaded_candidate).expect("reload evaluation should succeed");

        assert_eq!(
            decision,
            ReloadDecision::Apply {
                changed_sections: vec!["runtime".to_string()],
            }
        );
    }

    #[test]
    fn failed_reload_candidate_keeps_previous_config_for_rollback() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);
        let previous = current.clone();

        std::fs::write(
            &current.config_path,
            "[runtime\nenable_live_settings_reload = true\n",
        )
        .expect("write malformed candidate config");

        let error =
            load_candidate_config(&current).expect_err("malformed candidate should fail to load");
        assert!(error.to_string().contains("reload config from"));
        assert_eq!(
            current.runtime.enable_live_settings_reload,
            previous.runtime.enable_live_settings_reload
        );
        assert_eq!(current.workspace_dir, previous.workspace_dir);
    }

    #[test]
    fn invalid_config_rejected_during_reload_when_field_type_is_wrong() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);

        std::fs::write(
            &current.config_path,
            "[runtime]\nenable_live_settings_reload = \"true\"\n",
        )
        .expect("write invalid candidate config");

        let error =
            load_candidate_config(&current).expect_err("invalid field type should be rejected");
        assert!(error.to_string().contains("reload config from"));
    }

    #[test]
    fn partial_reload_failure_recovers_after_restoring_previous_config() {
        let tmp = TempDir::new().expect("temp dir");
        let current = config_with_paths(&tmp);
        write_config(&current);

        std::fs::write(&current.config_path, "[gateway]\ndefense_kill_switch = ")
            .expect("write partially-truncated config");
        load_candidate_config(&current).expect_err("partial write should fail validation");

        write_config(&current);
        let recovered =
            load_candidate_config(&current).expect("restored previous config should load again");
        let decision = evaluate_reload(&current, &recovered).expect("evaluate recovered config");
        assert_eq!(decision, ReloadDecision::NoChanges);
    }
}
