use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelianceDrillConfig {
    pub frequency: usize,
    pub error_library_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelianceDrillResult {
    pub caught: bool,
    pub response_action: String,
    pub latency_ms: u64,
}

#[must_use]
pub fn should_run_reliance_drill(decision_count: usize, config: &RelianceDrillConfig) -> bool {
    config.frequency > 0 && decision_count > 0 && decision_count.is_multiple_of(config.frequency)
}

/// # Errors
/// Returns an error when the config is invalid or the response action is empty.
pub fn evaluate_reliance_drill(
    config: &RelianceDrillConfig,
    response_action: &str,
    latency_ms: u64,
) -> Result<RelianceDrillResult> {
    if config.frequency == 0 {
        bail!("reliance drill frequency must be greater than zero")
    }
    if config.error_library_path.as_os_str().is_empty() {
        bail!("reliance drill error library path must not be empty")
    }

    let normalized = response_action.trim();
    if normalized.is_empty() {
        bail!("reliance drill response action must not be empty")
    }

    let response_key = normalized.to_ascii_lowercase();
    let caught = !matches!(
        response_key.as_str(),
        "approve" | "approved" | "accept" | "accepted"
    );

    Ok(RelianceDrillResult {
        caught,
        response_action: normalized.to_string(),
        latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reliance_drill_runs_on_configured_frequency() {
        let config = RelianceDrillConfig {
            frequency: 5,
            error_library_path: PathBuf::from("fixtures/error-library.json"),
        };

        assert!(!should_run_reliance_drill(4, &config));
        assert!(should_run_reliance_drill(5, &config));
    }

    #[test]
    fn reliance_drill_marks_bad_approval_as_not_caught() {
        let config = RelianceDrillConfig {
            frequency: 3,
            error_library_path: PathBuf::from("fixtures/error-library.json"),
        };

        let result = evaluate_reliance_drill(&config, "approve", 1_250).expect("valid drill");
        assert!(!result.caught);
    }

    #[test]
    fn reliance_drill_marks_rejection_as_caught() {
        let config = RelianceDrillConfig {
            frequency: 3,
            error_library_path: PathBuf::from("fixtures/error-library.json"),
        };

        let result = evaluate_reliance_drill(&config, "reject", 1_250).expect("valid drill");
        assert!(result.caught);
    }
}
