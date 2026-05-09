//! Autonomy level, rollout stages, temperature bands, and action-rate
//! limits that govern how much independence the agent has.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::contracts::security::{AutonomyLevel, ExternalActionExecution};

/// Agent autonomy level, rate limits, and safety controls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyConfig {
    /// Base autonomy level (may be capped by rollout stage).
    pub level: AutonomyLevel,
    /// Whether external action execution is allowed.
    #[serde(default)]
    pub external_action_execution: ExternalActionExecution,
    /// Gradual rollout configuration for autonomy staging.
    #[serde(default)]
    pub rollout: RolloutConfig,
    /// Restrict tool execution to the workspace directory.
    pub workspace_only: bool,
    /// Shell commands allowed in supervised/full modes.
    pub allowed_commands: Vec<String>,
    /// Filesystem paths the agent must never access.
    pub forbidden_paths: Vec<String>,
    /// Global rate limit on tool actions per hour.
    pub max_actions_per_hour: u32,
    /// Per-entity rate limit on tool actions per hour. Default: 20.
    #[serde(default = "default_max_actions_per_entity_per_hour")]
    pub max_actions_per_entity_per_hour: u32,
    /// Per-conversation rate limit on tool actions per hour. Default: 60.
    #[serde(default = "default_max_actions_per_conversation_per_hour")]
    pub max_actions_per_conversation_per_hour: u32,
    /// Per-workspace rate limit on tool actions per hour. Default: 200.
    #[serde(default = "default_max_actions_per_workspace_per_hour")]
    pub max_actions_per_workspace_per_hour: u32,
    /// Per-entity short-window burst cap. 0 = disabled. Default: 10.
    #[serde(default = "default_burst_max_per_entity")]
    pub burst_max_per_entity: u32,
    /// Burst window duration in seconds. Default: 60.
    #[serde(default = "default_burst_window_secs")]
    pub burst_window_secs: u64,
    /// Maximum daily cost budget in cents.
    pub max_cost_per_day_cents: u32,
    /// Max verify-repair loop attempts. Default: 3.
    #[serde(default = "default_verify_repair_max_attempts")]
    pub verify_repair_max_attempts: u32,
    /// Max repair depth within a verify loop. Default: 2.
    #[serde(default = "default_verify_repair_max_repair_depth")]
    pub verify_repair_max_repair_depth: u32,
    /// Max iterations for a single tool loop. Default: 10.
    #[serde(default = "default_max_tool_loop_iterations")]
    pub max_tool_loop_iterations: u32,
    /// Per-autonomy-level temperature band constraints.
    #[serde(default)]
    pub temperature_bands: TemperatureBands,
}

/// Staged rollout phase for gradual autonomy escalation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyRolloutStage {
    /// Read-only access; no mutations allowed.
    ReadOnly,
    /// Human approval required for each action.
    Supervised,
    /// Full autonomy within configured limits.
    Full,
}

/// Gradual rollout schedule for autonomy level transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutConfig {
    /// Whether staged rollout is active. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Current rollout stage (caps the configured autonomy level).
    pub stage: Option<AutonomyRolloutStage>,
    /// Days to remain in read-only stage. Default: 14.
    pub read_only_days: Option<u32>,
    /// Days to remain in supervised stage. Default: 14.
    pub supervised_days: Option<u32>,
}

impl Default for RolloutConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stage: None,
            read_only_days: Some(14),
            supervised_days: Some(14),
        }
    }
}

/// Per-autonomy-level temperature clamping bands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperatureBands {
    /// Temperature range for read-only mode. Default: [0.0, 0.2].
    #[serde(default = "default_temperature_band_read_only")]
    pub read_only: TemperatureBand,
    /// Temperature range for supervised mode. Default: [0.2, 0.7].
    #[serde(default = "default_temperature_band_supervised")]
    pub supervised: TemperatureBand,
    /// Temperature range for full autonomy. Default: [0.2, 1.0].
    #[serde(default = "default_temperature_band_full")]
    pub full: TemperatureBand,
}

/// A [min, max] range used to clamp LLM temperature.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TemperatureBand {
    /// Lower bound (inclusive). Must be in [0.0, 2.0].
    pub min: f64,
    /// Upper bound (inclusive). Must be in [0.0, 2.0] and >= min.
    pub max: f64,
}

fn default_temperature_band_read_only() -> TemperatureBand {
    TemperatureBand { min: 0.0, max: 0.2 }
}

fn default_temperature_band_supervised() -> TemperatureBand {
    TemperatureBand { min: 0.2, max: 0.7 }
}

fn default_temperature_band_full() -> TemperatureBand {
    TemperatureBand { min: 0.2, max: 1.0 }
}

fn default_verify_repair_max_attempts() -> u32 {
    3
}

fn default_verify_repair_max_repair_depth() -> u32 {
    2
}

fn default_max_tool_loop_iterations() -> u32 {
    10
}

fn default_max_actions_per_entity_per_hour() -> u32 {
    30
}

fn default_max_actions_per_conversation_per_hour() -> u32 {
    60
}

fn default_max_actions_per_workspace_per_hour() -> u32 {
    200
}

fn default_burst_max_per_entity() -> u32 {
    10
}

fn default_burst_window_secs() -> u64 {
    60
}

impl Default for TemperatureBands {
    fn default() -> Self {
        Self {
            read_only: default_temperature_band_read_only(),
            supervised: default_temperature_band_supervised(),
            full: default_temperature_band_full(),
        }
    }
}

impl TemperatureBand {
    fn validate(self, label: &str) -> Result<()> {
        if self.min.is_nan() || self.max.is_nan() {
            anyhow::bail!("autonomy.temperature_bands.{label} min/max must not be NaN");
        }
        if !(0.0..=2.0).contains(&self.min) {
            anyhow::bail!("autonomy.temperature_bands.{label} min must be in [0.0, 2.0]");
        }
        if !(0.0..=2.0).contains(&self.max) {
            anyhow::bail!("autonomy.temperature_bands.{label} max must be in [0.0, 2.0]");
        }
        if self.min > self.max {
            anyhow::bail!("autonomy.temperature_bands.{label} min must be <= max");
        }
        Ok(())
    }
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            external_action_execution: ExternalActionExecution::Disabled,
            rollout: RolloutConfig::default(),
            workspace_only: true,
            allowed_commands: crate::contracts::security::default_allowed_commands(),
            forbidden_paths: crate::contracts::security::default_forbidden_paths(),
            max_actions_per_hour: 300,
            max_actions_per_entity_per_hour: default_max_actions_per_entity_per_hour(),
            max_actions_per_conversation_per_hour: default_max_actions_per_conversation_per_hour(),
            max_actions_per_workspace_per_hour: default_max_actions_per_workspace_per_hour(),
            burst_max_per_entity: default_burst_max_per_entity(),
            burst_window_secs: default_burst_window_secs(),
            max_cost_per_day_cents: 500,
            verify_repair_max_attempts: default_verify_repair_max_attempts(),
            verify_repair_max_repair_depth: default_verify_repair_max_repair_depth(),
            max_tool_loop_iterations: default_max_tool_loop_iterations(),
            temperature_bands: TemperatureBands::default(),
        }
    }
}

impl AutonomyConfig {
    /// Returns the effective autonomy level after applying rollout caps.
    #[must_use]
    pub fn effective_autonomy_lvl(&self) -> AutonomyLevel {
        if !self.rollout.enabled {
            return self.level;
        }

        let Some(stage) = self.rollout.stage else {
            return self.level;
        };

        min_autonomy(self.level, rollout_stage_to_autonomy(stage))
    }

    /// Returns the temperature band for the effective autonomy level.
    #[must_use]
    pub fn selected_temp_band(&self) -> TemperatureBand {
        match self.effective_autonomy_lvl() {
            AutonomyLevel::ReadOnly => self.temperature_bands.read_only,
            AutonomyLevel::Supervised => self.temperature_bands.supervised,
            AutonomyLevel::Full => self.temperature_bands.full,
        }
    }

    /// Clamp the given temperature to the current autonomy band.
    #[must_use]
    pub fn clamp_temperature(&self, temperature: f64) -> f64 {
        let band = self.selected_temp_band();
        temperature.clamp(band.min, band.max)
    }

    /// # Errors
    ///
    /// Returns an error when any configured temperature band has invalid
    /// bounds (NaN values, out-of-range values, or `min > max`).
    pub fn validate_temperature_bands(&self) -> Result<()> {
        self.temperature_bands.read_only.validate("read_only")?;
        self.temperature_bands.supervised.validate("supervised")?;
        self.temperature_bands.full.validate("full")?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when `verify_repair_max_attempts` is `0` or when
    /// `verify_repair_max_repair_depth` is not strictly less than
    /// `verify_repair_max_attempts`.
    pub fn validate_verify_repair_caps(&self) -> Result<()> {
        if self.verify_repair_max_attempts == 0 {
            anyhow::bail!("autonomy.verify_repair_max_attempts must be >= 1");
        }
        if self.verify_repair_max_repair_depth >= self.verify_repair_max_attempts {
            anyhow::bail!(
                "autonomy.verify_repair_max_repair_depth must be < autonomy.verify_repair_max_attempts"
            );
        }
        Ok(())
    }
}

#[must_use]
fn rollout_stage_to_autonomy(stage: AutonomyRolloutStage) -> AutonomyLevel {
    match stage {
        AutonomyRolloutStage::ReadOnly => AutonomyLevel::ReadOnly,
        AutonomyRolloutStage::Supervised => AutonomyLevel::Supervised,
        AutonomyRolloutStage::Full => AutonomyLevel::Full,
    }
}

#[must_use]
fn min_autonomy(global: AutonomyLevel, channel: AutonomyLevel) -> AutonomyLevel {
    match (global, channel) {
        (AutonomyLevel::ReadOnly, _) | (_, AutonomyLevel::ReadOnly) => AutonomyLevel::ReadOnly,
        (AutonomyLevel::Supervised, _) | (_, AutonomyLevel::Supervised) => {
            AutonomyLevel::Supervised
        }
        (AutonomyLevel::Full, AutonomyLevel::Full) => AutonomyLevel::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(lhs: f64, rhs: f64) {
        assert!((lhs - rhs).abs() < 1e-9, "lhs={lhs} rhs={rhs}");
    }

    #[test]
    fn temperature_band_validate_accepts_valid_band() {
        let band = TemperatureBand { min: 0.1, max: 1.8 };
        assert!(band.validate("test").is_ok());
    }

    #[test]
    fn temperature_band_validate_rejects_min_greater_than_max() {
        let band = TemperatureBand { min: 1.0, max: 0.5 };
        assert!(band.validate("test").is_err());
    }

    #[test]
    fn temperature_band_validate_rejects_nan_values() {
        let min_nan = TemperatureBand {
            min: f64::NAN,
            max: 1.0,
        };
        let max_nan = TemperatureBand {
            min: 0.5,
            max: f64::NAN,
        };

        assert!(min_nan.validate("test").is_err());
        assert!(max_nan.validate("test").is_err());
    }

    #[test]
    fn temperature_band_validate_rejects_values_outside_range() {
        let below_range = TemperatureBand {
            min: -0.1,
            max: 0.5,
        };
        let above_range = TemperatureBand { min: 0.5, max: 2.1 };

        assert!(below_range.validate("test").is_err());
        assert!(above_range.validate("test").is_err());
    }

    #[test]
    fn temperature_band_validate_accepts_boundary_values() {
        let boundary = TemperatureBand { min: 0.0, max: 2.0 };
        assert!(boundary.validate("test").is_ok());
    }

    #[test]
    fn selected_temperature_band_matches_autonomy_level() {
        let bands = TemperatureBands {
            read_only: TemperatureBand { min: 0.0, max: 0.1 },
            supervised: TemperatureBand { min: 0.2, max: 0.3 },
            full: TemperatureBand { min: 0.4, max: 0.9 },
        };

        let read_only_cfg = AutonomyConfig {
            level: AutonomyLevel::ReadOnly,
            temperature_bands: bands.clone(),
            ..AutonomyConfig::default()
        };
        let supervised_cfg = AutonomyConfig {
            level: AutonomyLevel::Supervised,
            temperature_bands: bands.clone(),
            ..AutonomyConfig::default()
        };
        let full_cfg = AutonomyConfig {
            level: AutonomyLevel::Full,
            temperature_bands: bands,
            ..AutonomyConfig::default()
        };

        let read_only_band = read_only_cfg.selected_temp_band();
        let supervised_band = supervised_cfg.selected_temp_band();
        let full_band = full_cfg.selected_temp_band();

        assert_close(read_only_band.min, 0.0);
        assert_close(read_only_band.max, 0.1);
        assert_close(supervised_band.min, 0.2);
        assert_close(supervised_band.max, 0.3);
        assert_close(full_band.min, 0.4);
        assert_close(full_band.max, 0.9);
    }

    #[test]
    fn effective_autonomy_level_rollout_disabled_returns_configured_level() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            rollout: RolloutConfig {
                enabled: false,
                stage: Some(AutonomyRolloutStage::ReadOnly),
                ..RolloutConfig::default()
            },
            ..AutonomyConfig::default()
        };

        assert_eq!(config.effective_autonomy_lvl(), AutonomyLevel::Full);
    }

    #[test]
    fn effective_autonomy_level_rollout_enabled_read_only_overrides_full() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            rollout: RolloutConfig {
                enabled: true,
                stage: Some(AutonomyRolloutStage::ReadOnly),
                ..RolloutConfig::default()
            },
            ..AutonomyConfig::default()
        };

        assert_eq!(config.effective_autonomy_lvl(), AutonomyLevel::ReadOnly);
    }

    #[test]
    fn effective_autonomy_level_rollout_enabled_supervised_caps_full() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            rollout: RolloutConfig {
                enabled: true,
                stage: Some(AutonomyRolloutStage::Supervised),
                ..RolloutConfig::default()
            },
            ..AutonomyConfig::default()
        };

        assert_eq!(config.effective_autonomy_lvl(), AutonomyLevel::Supervised);
    }

    #[test]
    fn effective_autonomy_level_rollout_enabled_without_stage_returns_configured_level() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Supervised,
            rollout: RolloutConfig {
                enabled: true,
                stage: None,
                ..RolloutConfig::default()
            },
            ..AutonomyConfig::default()
        };

        assert_eq!(config.effective_autonomy_lvl(), AutonomyLevel::Supervised);
    }

    #[test]
    fn effective_autonomy_level_rollout_enabled_full_keeps_full_when_config_full() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            rollout: RolloutConfig {
                enabled: true,
                stage: Some(AutonomyRolloutStage::Full),
                ..RolloutConfig::default()
            },
            ..AutonomyConfig::default()
        };

        assert_eq!(config.effective_autonomy_lvl(), AutonomyLevel::Full);
    }

    #[test]
    fn effective_autonomy_level_rollout_cannot_escalate_supervised_to_full() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Supervised,
            rollout: RolloutConfig {
                enabled: true,
                stage: Some(AutonomyRolloutStage::Full),
                ..RolloutConfig::default()
            },
            ..AutonomyConfig::default()
        };

        assert_eq!(config.effective_autonomy_lvl(), AutonomyLevel::Supervised);
    }

    #[test]
    fn selected_temperature_band_uses_effective_autonomy_level() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            rollout: RolloutConfig {
                enabled: true,
                stage: Some(AutonomyRolloutStage::ReadOnly),
                ..RolloutConfig::default()
            },
            temperature_bands: TemperatureBands {
                read_only: TemperatureBand { min: 0.0, max: 0.1 },
                supervised: TemperatureBand { min: 0.2, max: 0.7 },
                full: TemperatureBand { min: 0.8, max: 1.1 },
            },
            ..AutonomyConfig::default()
        };

        assert_close(config.selected_temp_band().max, 0.1);
    }

    #[test]
    fn clamp_temperature_applies_band_limits() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Supervised,
            temperature_bands: TemperatureBands {
                read_only: TemperatureBand { min: 0.0, max: 0.2 },
                supervised: TemperatureBand { min: 0.2, max: 0.7 },
                full: TemperatureBand { min: 0.2, max: 1.0 },
            },
            ..AutonomyConfig::default()
        };

        assert_close(config.clamp_temperature(0.5), 0.5);
        assert_close(config.clamp_temperature(0.2), 0.2);
        assert_close(config.clamp_temperature(0.7), 0.7);
        assert_close(config.clamp_temperature(0.1), 0.2);
        assert_close(config.clamp_temperature(0.9), 0.7);
    }

    #[test]
    fn validate_temperature_bands_accepts_valid_configuration() {
        let config = AutonomyConfig {
            temperature_bands: TemperatureBands {
                read_only: TemperatureBand { min: 0.0, max: 0.1 },
                supervised: TemperatureBand { min: 0.1, max: 0.7 },
                full: TemperatureBand { min: 0.1, max: 1.0 },
            },
            ..AutonomyConfig::default()
        };

        assert!(config.validate_temperature_bands().is_ok());
    }

    #[test]
    fn validate_temperature_bands_rejects_invalid_band() {
        let config = AutonomyConfig {
            temperature_bands: TemperatureBands {
                read_only: TemperatureBand { min: 0.0, max: 0.1 },
                supervised: TemperatureBand { min: 0.9, max: 0.7 },
                full: TemperatureBand { min: 0.1, max: 1.0 },
            },
            ..AutonomyConfig::default()
        };

        assert!(config.validate_temperature_bands().is_err());
    }

    #[test]
    fn validate_verify_repair_caps_accepts_valid_caps() {
        let config = AutonomyConfig {
            verify_repair_max_attempts: 3,
            verify_repair_max_repair_depth: 2,
            ..AutonomyConfig::default()
        };

        assert!(config.validate_verify_repair_caps().is_ok());
    }

    #[test]
    fn validate_verify_repair_caps_rejects_zero_attempts() {
        let config = AutonomyConfig {
            verify_repair_max_attempts: 0,
            verify_repair_max_repair_depth: 0,
            ..AutonomyConfig::default()
        };

        assert!(config.validate_verify_repair_caps().is_err());
    }

    #[test]
    fn validate_verify_repair_caps_rejects_depth_greater_than_or_equal_attempts() {
        let config = AutonomyConfig {
            verify_repair_max_attempts: 3,
            verify_repair_max_repair_depth: 3,
            ..AutonomyConfig::default()
        };

        assert!(config.validate_verify_repair_caps().is_err());
    }

    #[test]
    fn default_config_is_valid_and_has_reasonable_controls() {
        let config = AutonomyConfig::default();

        assert!(config.validate_temperature_bands().is_ok());
        assert!(config.validate_verify_repair_caps().is_ok());
        assert!(config.workspace_only);
        assert!(config.allowed_commands.contains(&"git".to_string()));
        assert!(config.allowed_commands.contains(&"cargo".to_string()));
        assert!(config.forbidden_paths.contains(&"/etc".to_string()));
        assert!(config.max_actions_per_hour > 0);
        assert!(config.max_actions_per_entity_per_hour > 0);
        assert!(config.max_cost_per_day_cents > 0);
    }

    #[test]
    fn autonomy_config_toml_round_trip_preserves_values() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            external_action_execution: ExternalActionExecution::Enabled,
            workspace_only: false,
            allowed_commands: vec!["git".into(), "cargo".into(), "just".into()],
            forbidden_paths: vec!["/etc".into(), "~/.ssh".into()],
            max_actions_per_hour: 42,
            max_actions_per_entity_per_hour: 11,
            max_actions_per_conversation_per_hour: 55,
            max_actions_per_workspace_per_hour: 180,
            burst_max_per_entity: 8,
            burst_window_secs: 45,
            max_cost_per_day_cents: 2_500,
            verify_repair_max_attempts: 5,
            verify_repair_max_repair_depth: 2,
            max_tool_loop_iterations: 12,
            temperature_bands: TemperatureBands {
                read_only: TemperatureBand { min: 0.0, max: 0.1 },
                supervised: TemperatureBand { min: 0.2, max: 0.6 },
                full: TemperatureBand { min: 0.3, max: 1.2 },
            },
            ..AutonomyConfig::default()
        };

        let serialized = toml::to_string(&config).unwrap();
        let deserialized: AutonomyConfig = toml::from_str(&serialized).unwrap();

        assert!(matches!(deserialized.level, AutonomyLevel::Full));
        assert_eq!(
            deserialized.external_action_execution,
            ExternalActionExecution::Enabled
        );
        assert!(!deserialized.workspace_only);
        assert_eq!(deserialized.allowed_commands, config.allowed_commands);
        assert_eq!(deserialized.forbidden_paths, config.forbidden_paths);
        assert_eq!(deserialized.max_actions_per_hour, 42);
        assert_eq!(deserialized.max_actions_per_entity_per_hour, 11);
        assert_eq!(deserialized.max_actions_per_conversation_per_hour, 55);
        assert_eq!(deserialized.max_actions_per_workspace_per_hour, 180);
        assert_eq!(deserialized.burst_max_per_entity, 8);
        assert_eq!(deserialized.burst_window_secs, 45);
        assert_eq!(deserialized.max_cost_per_day_cents, 2_500);
        assert_eq!(deserialized.verify_repair_max_attempts, 5);
        assert_eq!(deserialized.verify_repair_max_repair_depth, 2);
        assert_eq!(deserialized.max_tool_loop_iterations, 12);
        assert_close(deserialized.temperature_bands.read_only.min, 0.0);
        assert_close(deserialized.temperature_bands.read_only.max, 0.1);
        assert_close(deserialized.temperature_bands.supervised.min, 0.2);
        assert_close(deserialized.temperature_bands.supervised.max, 0.6);
        assert_close(deserialized.temperature_bands.full.min, 0.3);
        assert_close(deserialized.temperature_bands.full.max, 1.2);
    }
}
