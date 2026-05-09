//! Root `Config` struct that aggregates every configuration section,
//! plus validation, onboarding detection, and default constants.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::super::{
    AutonomyConfig, ChannelsConfig, CommandsConfig, GatewayConfig, InferenceConfig, McpConfig,
    MemoryConfig, NetworkConfig, ObservabilityConfig, SecurityConfig, SessionRoutingConfig,
    TasteConfig, ToolsConfig, TunnelConfig,
};
use super::codespace::CodespaceConfig;
use super::identity::{
    BrowserConfig, ComposioConfig, HeartbeatConfig, IdentityConfig, ReliabilityConfig,
    RuntimeConfig, SecretsConfig,
};
use super::models::{ModelListEntry, SkillsRuntimeConfig};
use super::persona::PersonaConfig;
use crate::contracts::media::MediaConfig;

/// Default provider when none is configured.
pub const DEFAULT_PROVIDER: &str = "openrouter";
/// Default model when none is configured.
pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.6";

pub(super) use crate::config::schema::default_true;

/// Root configuration struct aggregating all sections.
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    /// Workspace directory - computed from home, not serialized.
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    /// Path to config.toml - computed from home, not serialized.
    #[serde(skip)]
    pub config_path: PathBuf,
    /// LLM provider API key.
    pub api_key: Option<String>,
    /// Default provider name (e.g. `"openrouter"`, `"anthropic"`).
    pub default_provider: Option<String>,
    /// Default model name or registry alias.
    pub default_model: Option<String>,
    /// Default LLM temperature (0.0-2.0).
    pub default_temperature: f64,

    /// Model registry aliases for named model shortcuts.
    #[serde(default)]
    pub model_list: Vec<ModelListEntry>,

    /// Skills runtime discovery and loading configuration.
    #[serde(default)]
    pub skills: SkillsRuntimeConfig,

    /// Observability backend configuration.
    #[serde(default)]
    pub observability: ObservabilityConfig,

    /// Autonomy level, rate limits, and safety controls.
    #[serde(default)]
    pub autonomy: AutonomyConfig,

    /// Runtime environment configuration.
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// Retry, backoff, and scheduler reliability settings.
    #[serde(default)]
    pub reliability: ReliabilityConfig,

    /// Heartbeat (periodic self-check) configuration.
    #[serde(default)]
    pub heartbeat: HeartbeatConfig,

    /// I/O channel adapter configuration.
    #[serde(default)]
    pub channels_config: ChannelsConfig,

    /// Runtime command authorization policy.
    #[serde(default)]
    pub commands: CommandsConfig,

    /// Memory subsystem configuration.
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Outbound network settings.
    #[serde(default)]
    pub network: NetworkConfig,

    /// Media (STT/TTS) configuration.
    #[serde(default)]
    pub media: MediaConfig,

    /// Tunnel configuration for public gateway exposure.
    #[serde(default)]
    pub tunnel: TunnelConfig,

    /// HTTP gateway configuration.
    #[serde(default)]
    pub gateway: GatewayConfig,

    /// Composio integration configuration.
    #[serde(default)]
    pub composio: ComposioConfig,

    /// Secrets encryption configuration.
    #[serde(default)]
    pub secrets: SecretsConfig,

    /// Browser automation tool configuration.
    #[serde(default)]
    pub browser: BrowserConfig,

    /// Persona subsystem configuration.
    #[serde(default)]
    pub persona: PersonaConfig,

    /// Identity document configuration.
    #[serde(default)]
    pub identity: IdentityConfig,

    /// Per-tool enable/disable configuration.
    #[serde(default)]
    pub tools: ToolsConfig,

    /// Session routing and reset behavior controls.
    #[serde(default)]
    pub session: SessionRoutingConfig,

    /// MCP server configuration.
    #[serde(default)]
    pub mcp: McpConfig,

    /// Taste evaluation configuration.
    #[serde(default)]
    pub taste: TasteConfig,

    /// Inference-level configuration.
    #[serde(default)]
    pub inference: InferenceConfig,

    /// Security subsystem configuration.
    #[serde(default)]
    pub security: SecurityConfig,

    /// Sandboxed codespace configuration.
    #[serde(default)]
    pub codespace: CodespaceConfig,

    /// UI locale (ISO 639-1 code). Default: `"en"`.
    #[serde(default = "default_locale")]
    pub locale: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("workspace_dir", &self.workspace_dir)
            .field("config_path", &self.config_path)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("default_provider", &self.default_provider)
            .field("default_model", &self.default_model)
            .field("default_temperature", &self.default_temperature)
            .field("model_list", &self.model_list)
            .field("skills", &self.skills)
            .field("observability", &self.observability)
            .field("autonomy", &self.autonomy)
            .field("runtime", &self.runtime)
            .field("reliability", &self.reliability)
            .field("heartbeat", &self.heartbeat)
            .field("channels_config", &self.channels_config)
            .field("commands", &self.commands)
            .field("memory", &self.memory)
            .field("network", &self.network)
            .field("media", &self.media)
            .field("tunnel", &self.tunnel)
            .field("gateway", &self.gateway)
            .field("composio", &self.composio)
            .field("secrets", &self.secrets)
            .field("browser", &self.browser)
            .field("persona", &self.persona)
            .field("identity", &self.identity)
            .field("tools", &self.tools)
            .field("session", &self.session)
            .field("mcp", &self.mcp)
            .field("taste", &self.taste)
            .field("codespace", &self.codespace)
            .field("locale", &self.locale)
            .field("inference", &self.inference)
            .field("security", &self.security)
            .finish()
    }
}

fn default_locale() -> String {
    "en".into()
}

impl Default for Config {
    fn default() -> Self {
        let asterel_dir = crate::utils::dirs::asterel_home_dir_or_local();

        Self {
            workspace_dir: asterel_dir.join("workspace"),
            config_path: asterel_dir.join("config.toml"),
            api_key: None,
            default_provider: Some(DEFAULT_PROVIDER.to_string()),
            default_model: Some(DEFAULT_MODEL.to_string()),
            default_temperature: 0.7,
            model_list: Vec::new(),
            skills: SkillsRuntimeConfig::default(),
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            runtime: RuntimeConfig::default(),
            reliability: ReliabilityConfig::default(),
            heartbeat: HeartbeatConfig::default(),
            channels_config: ChannelsConfig::default(),
            commands: CommandsConfig::default(),
            memory: MemoryConfig::default(),
            network: NetworkConfig::default(),
            media: MediaConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            persona: PersonaConfig::default(),
            identity: IdentityConfig::default(),
            tools: ToolsConfig::default(),
            session: SessionRoutingConfig::default(),
            mcp: McpConfig::default(),
            taste: TasteConfig::default(),
            inference: InferenceConfig::default(),
            security: SecurityConfig::default(),
            codespace: CodespaceConfig::default(),
            locale: default_locale(),
        }
    }
}

impl Config {
    /// # Errors
    ///
    /// Returns an error when autonomy temperature bands are invalid.
    pub fn validate_temperature_bands(&self) -> Result<()> {
        self.autonomy.validate_temperature_bands()
    }

    /// # Errors
    ///
    /// Returns an error when autonomy, reliability window settings, or model
    /// list registry controls are invalid.
    pub fn validate_autonomy_controls(&self) -> Result<()> {
        self.validate_temperature_bands()?;
        if !(0.0..=2.0).contains(&self.default_temperature) {
            anyhow::bail!(
                "default_temperature must be in [0.0, 2.0], got {}",
                self.default_temperature
            );
        }
        self.autonomy.validate_verify_repair_caps()?;
        self.validate_reliability_controls()?;
        self.validate_model_list_registry()?;
        self.validate_loop_detection_controls()?;
        self.validate_session_controls()?;
        self.persona.validate()?;
        self.taste.validate()?;
        self.network.validate()?;
        self.security.validate()?;
        self.warn_channel_tool_allowlist_posture();
        Ok(())
    }

    fn warn_channel_tool_allowlist_posture(&self) {
        use crate::contracts::security::AutonomyLevel;

        fn min_autonomy(global: AutonomyLevel, channel: AutonomyLevel) -> AutonomyLevel {
            match (global, channel) {
                (AutonomyLevel::ReadOnly, _) | (_, AutonomyLevel::ReadOnly) => {
                    AutonomyLevel::ReadOnly
                }
                (AutonomyLevel::Supervised, _) | (_, AutonomyLevel::Supervised) => {
                    AutonomyLevel::Supervised
                }
                _ => AutonomyLevel::Full,
            }
        }

        let global = self.autonomy.effective_autonomy_lvl();
        let warn_if_unbounded = |channel_name: &str,
                                 autonomy_override: Option<AutonomyLevel>,
                                 has_tool_allowlist: bool| {
            let channel = autonomy_override.unwrap_or(global);
            let effective = min_autonomy(global, channel);
            if effective != AutonomyLevel::ReadOnly && !has_tool_allowlist {
                tracing::warn!(
                    channel = channel_name,
                    effective_autonomy = ?effective,
                    "channel tool_allowlist is not set; all tools are currently permitted"
                );
            }
        };

        if let Some(cfg) = &self.channels_config.telegram {
            warn_if_unbounded(
                "telegram",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.discord {
            warn_if_unbounded(
                "discord",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.slack {
            warn_if_unbounded(
                "slack",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.webhook {
            warn_if_unbounded(
                "webhook",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.imessage {
            warn_if_unbounded(
                "imessage",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.matrix {
            warn_if_unbounded(
                "matrix",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.whatsapp {
            warn_if_unbounded(
                "whatsapp",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.email {
            warn_if_unbounded(
                "email",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
        if let Some(cfg) = &self.channels_config.irc {
            warn_if_unbounded(
                "irc",
                cfg.security.autonomy_level,
                cfg.security.tool_allowlist.is_some(),
            );
        }
    }

    fn validate_loop_detection_controls(&self) -> Result<()> {
        let cfg = &self.tools.loop_detection;
        if !cfg.enabled {
            return Ok(());
        }

        if cfg.history_size < 2 {
            anyhow::bail!("tools.loop_detection.history_size must be >= 2 when enabled");
        }
        if cfg.warning_threshold == 0 {
            anyhow::bail!("tools.loop_detection.warning_threshold must be >= 1 when enabled");
        }
        if cfg.critical_threshold == 0 {
            anyhow::bail!("tools.loop_detection.critical_threshold must be >= 1 when enabled");
        }
        if cfg.warning_threshold > cfg.critical_threshold {
            anyhow::bail!("tools.loop_detection.warning_threshold must be <= critical_threshold");
        }
        Ok(())
    }

    fn validate_session_controls(&self) -> Result<()> {
        if self.session.parent_fork_max_tokens < 1_024 {
            anyhow::bail!("session.parent_fork_max_tokens must be >= 1024");
        }
        Ok(())
    }

    fn validate_reliability_controls(&self) -> Result<()> {
        let start = self.reliability.scheduler_active_hours_start_utc.as_deref();
        let end = self.reliability.scheduler_active_hours_end_utc.as_deref();

        match (start, end) {
            (None, None) => Ok(()),
            (Some(_), None) | (None, Some(_)) => anyhow::bail!(
                "reliability.scheduler_active_hours_start_utc and reliability.scheduler_active_hours_end_utc must be set together"
            ),
            (Some(start), Some(end)) => {
                parse_hhmm_utc(start, "reliability.scheduler_active_hours_start_utc")?;
                parse_hhmm_utc(end, "reliability.scheduler_active_hours_end_utc")?;
                Ok(())
            }
        }
    }

    /// Returns `true` when the config appears to be a fresh default that has
    /// never been through onboarding (no API key and no env var overrides).
    ///
    /// Note: `default_provider` is not checked because `Config::default()`
    /// always sets it to `Some("openrouter")`, so its `is_none()` branch
    /// would never be true in practice.
    #[must_use]
    pub fn needs_onboarding(&self) -> bool {
        // If env var provides an API key, user has configured externally
        if std::env::var("ASTEREL_API_KEY").is_ok() {
            return false;
        }
        self.api_key.is_none()
    }

    /// Path to the daemon state JSON file, derived from the config file location.
    #[must_use]
    pub fn daemon_state_path(&self) -> std::path::PathBuf {
        self.config_path
            .parent()
            .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from)
            .join("daemon_state.json")
    }
}

fn parse_hhmm_utc(value: &str, field_name: &str) -> Result<()> {
    let (hour_raw, minute_raw) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("{field_name} must use HH:MM format"))?;

    let hour = hour_raw
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("{field_name} hour must be 0..=23"))?;
    let minute = minute_raw
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("{field_name} minute must be 0..=59"))?;

    if hour > 23 || minute > 59 {
        anyhow::bail!("{field_name} must be within 00:00..23:59");
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
