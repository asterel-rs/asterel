//! Autonomy-level transition tracking for the heartbeat worker.
//!
//! Detects changes in the configured autonomy mode (`ReadOnly`,
//! Supervised, Full) and emits lifecycle signals to the observer.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::runtime::observability::traits::AutonomySignal;

fn autonomy_level_to_str(level: crate::security::AutonomyLevel) -> &'static str {
    match level {
        crate::security::AutonomyLevel::ReadOnly => "read_only",
        crate::security::AutonomyLevel::Supervised => "supervised",
        crate::security::AutonomyLevel::Full => "full",
    }
}

fn read_last_autonomy_level(workspace_dir: &Path) -> Option<String> {
    let path = workspace_dir.join("state").join("autonomy_mode_state.json");
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to read autonomy state");
            return None;
        }
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => v
            .get("last")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to parse autonomy state");
            None
        }
    }
}

fn write_last_autonomy_level(workspace_dir: &Path, level: &str) -> Result<()> {
    let state_dir = workspace_dir.join("state");
    fs::create_dir_all(&state_dir)?;
    let path = state_dir.join("autonomy_mode_state.json");
    let payload = serde_json::json!({"last": level});
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&payload)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Detects and records a change in the configured autonomy
/// level, emitting a lifecycle signal to the observer.
pub(super) fn record_autonomy_mode_transition(
    config: &crate::config::Config,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    let current = autonomy_level_to_str(config.autonomy.effective_autonomy_lvl()).to_string();
    if let Some(previous) = read_last_autonomy_level(&config.workspace_dir)
        && previous != current
    {
        observer.emit_autonomy_signal(AutonomySignal::ModeTransition);
        tracing::info!(from = previous, to = current, "autonomy mode transitioned");
    }
    if let Err(error) = write_last_autonomy_level(&config.workspace_dir, &current) {
        tracing::warn!(%error, "failed to persist autonomy level state");
    }
}

// ── Autonomy Recommendations ─────────────────────────────────────
// Will be wired to daemon runtime when heartbeat worker gains
// consecutive success/failure counters.  Until then, tested only.

#[cfg(test)]
mod autonomy_rec_tests {
    use serde::{Deserialize, Serialize};

    use super::autonomy_level_to_str;
    use crate::config::Config;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum AutonomyDirection {
        Increase,
        Decrease,
        Hold,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct AutonomyRecommendation {
        direction: AutonomyDirection,
        reason: String,
        current_level: String,
        consecutive_successes: u32,
        consecutive_failures: u32,
    }

    const INCREASE_THRESHOLD: u32 = 10;
    const DECREASE_THRESHOLD: u32 = 3;

    fn generate_autonomy_recommendation(
        config: &Config,
        consecutive_successes: u32,
        consecutive_failures: u32,
        had_safety_violation: bool,
    ) -> AutonomyRecommendation {
        let current = autonomy_level_to_str(config.autonomy.effective_autonomy_lvl());

        let (direction, reason) =
            if had_safety_violation || consecutive_failures >= DECREASE_THRESHOLD {
                (
                    AutonomyDirection::Decrease,
                    if had_safety_violation {
                        "safety violation detected; decrease recommended".to_string()
                    } else {
                        format!(
                            "{consecutive_failures} consecutive failures exceed threshold \
                             ({DECREASE_THRESHOLD})"
                        )
                    },
                )
            } else if consecutive_successes >= INCREASE_THRESHOLD && current != "full" {
                (
                    AutonomyDirection::Increase,
                    format!(
                        "{consecutive_successes} consecutive successes exceed threshold \
                         ({INCREASE_THRESHOLD})"
                    ),
                )
            } else {
                (
                    AutonomyDirection::Hold,
                    "within normal operating band".to_string(),
                )
            };

        AutonomyRecommendation {
            direction,
            reason,
            current_level: current.to_string(),
            consecutive_successes,
            consecutive_failures,
        }
    }

    #[test]
    fn recommend_increase_after_consecutive_successes() {
        let config = Config::default();
        let rec = generate_autonomy_recommendation(&config, 12, 0, false);
        assert_eq!(rec.direction, AutonomyDirection::Increase);
    }

    #[test]
    fn recommend_decrease_after_consecutive_failures() {
        let config = Config::default();
        let rec = generate_autonomy_recommendation(&config, 0, 4, false);
        assert_eq!(rec.direction, AutonomyDirection::Decrease);
    }

    #[test]
    fn recommend_decrease_on_safety_violation() {
        let config = Config::default();
        let rec = generate_autonomy_recommendation(&config, 20, 0, true);
        assert_eq!(rec.direction, AutonomyDirection::Decrease);
        assert!(rec.reason.contains("safety violation"));
    }

    #[test]
    fn recommend_hold_in_normal_band() {
        let config = Config::default();
        let rec = generate_autonomy_recommendation(&config, 5, 1, false);
        assert_eq!(rec.direction, AutonomyDirection::Hold);
    }
}
