//! Identity, composio, secrets, browser, runtime, reliability,
//! and heartbeat configuration structs.

use serde::{Deserialize, Serialize};

use super::types::default_true;
use crate::contracts::ids::EntityId;

/// Identity document configuration (AIEOS / persona file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityConfig {
    /// Identity document format. Default: `"markdown"`.
    #[serde(default = "default_identity_format")]
    pub format: String,
    /// Optional stable person ID for cross-session identity.
    #[serde(default)]
    pub person_id: Option<String>,
    /// File path to an external AIEOS identity document.
    #[serde(default)]
    pub aieos_path: Option<String>,
    /// Inline AIEOS identity document content.
    #[serde(default)]
    pub aieos_inline: Option<String>,
}

fn default_identity_format() -> String {
    "markdown".into()
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            format: default_identity_format(),
            person_id: None,
            aieos_path: None,
            aieos_inline: None,
        }
    }
}

/// Composio integration configuration for third-party tool auth.
#[derive(Clone, Serialize, Deserialize)]
pub struct ComposioConfig {
    /// Whether Composio integration is enabled. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Composio API key.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Composio entity ID. Default: `"default"`.
    #[serde(default = "default_entity_id")]
    pub entity_id: EntityId,
}

impl std::fmt::Debug for ComposioConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposioConfig")
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("entity_id", &self.entity_id)
            .finish()
    }
}

fn default_entity_id() -> EntityId {
    EntityId::new("default")
}

impl Default for ComposioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            entity_id: default_entity_id(),
        }
    }
}

/// Secrets encryption configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsConfig {
    /// Whether to encrypt secrets at rest in config. Default: true.
    #[serde(default = "default_true")]
    pub encrypt: bool,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self { encrypt: true }
    }
}

/// Browser automation tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserConfig {
    /// Whether the browser tool is enabled. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed domains for browser navigation.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Persistent browser session name.
    #[serde(default)]
    pub session_name: Option<String>,
}

/// Runtime execution environment kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum RuntimeKind {
    /// Auto-detect based on platform and Docker availability.
    Auto,
    /// Native OS process execution (default).
    #[default]
    Native,
    /// Docker container isolation.
    Docker,
    /// WebAssembly sandbox.
    Wasm,
}

/// Sandbox workspace-only enforcement strategy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum SandboxSelectorMode {
    /// Use the configured `workspace_only` value as-is (default).
    #[default]
    Fixed,
    /// Automatically relax `workspace_only` when running in Docker.
    Auto,
}

/// Runtime environment configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Execution environment kind. Default: native.
    #[serde(default)]
    pub kind: RuntimeKind,
    /// Enable Docker runtime support. Default: false.
    #[serde(default)]
    pub enable_docker_runtime: bool,
    /// Hot-reload settings changes at runtime. Default: true.
    #[serde(default = "default_true")]
    pub enable_live_settings_reload: bool,
    /// Sandbox workspace-only enforcement mode. Default: fixed.
    #[serde(default)]
    pub sandbox_selector: SandboxSelectorMode,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            kind: RuntimeKind::default(),
            enable_docker_runtime: false,
            enable_live_settings_reload: true,
            sandbox_selector: SandboxSelectorMode::default(),
        }
    }
}

impl RuntimeConfig {
    /// Resolves `RuntimeKind::Auto` to a concrete kind.
    #[must_use]
    pub fn resolved_runtime_kind(&self) -> RuntimeKind {
        if self.kind != RuntimeKind::Auto {
            return self.kind;
        }
        if cfg!(target_arch = "wasm32") {
            return RuntimeKind::Wasm;
        }
        if self.enable_docker_runtime {
            return RuntimeKind::Docker;
        }
        RuntimeKind::Native
    }

    /// Resolves `workspace_only` considering the sandbox selector mode.
    #[must_use]
    pub fn resolved_workspace_only(&self, configured_workspace_only: bool) -> bool {
        match self.sandbox_selector {
            SandboxSelectorMode::Fixed => configured_workspace_only,
            SandboxSelectorMode::Auto => {
                !matches!(self.resolved_runtime_kind(), RuntimeKind::Docker)
            }
        }
    }
}

/// Retry, backoff, and scheduler reliability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliabilityConfig {
    /// Provider API call retry count. Default: 2.
    #[serde(default = "default_provider_retries")]
    pub provider_retries: u32,
    /// Initial backoff between provider retries (ms). Default: 500.
    #[serde(default = "default_provider_backoff_ms")]
    pub provider_backoff_ms: u64,
    /// Fallback provider names tried in order on failure.
    #[serde(default)]
    pub fallback_providers: Vec<String>,
    /// Channel reconnect initial backoff (seconds). Default: 2.
    #[serde(default = "default_channel_backoff_secs")]
    pub channel_initial_backoff_secs: u64,
    /// Channel reconnect maximum backoff (seconds). Default: 60.
    #[serde(default = "default_channel_backoff_max_secs")]
    pub channel_max_backoff_secs: u64,
    /// Cron scheduler poll interval (seconds). Default: 15.
    #[serde(default = "default_scheduler_poll_secs")]
    pub scheduler_poll_secs: u64,
    /// Scheduler task retry count. Default: 2.
    #[serde(default = "default_scheduler_retries")]
    pub scheduler_retries: u32,
    /// Scheduler circuit breaker failure budget. Default: 0.
    #[serde(default = "default_scheduler_failure_budget")]
    pub scheduler_failure_budget: u32,
    /// Breaker cooldown after budget exhaustion (seconds). Default: 0.
    #[serde(default = "default_scheduler_breaker_cooldown_secs")]
    pub scheduler_breaker_cooldown_secs: u64,
    /// Scheduler active window start (HH:MM UTC). Must pair with end.
    #[serde(default)]
    pub scheduler_active_hours_start_utc: Option<String>,
    /// Scheduler active window end (HH:MM UTC). Must pair with start.
    #[serde(default)]
    pub scheduler_active_hours_end_utc: Option<String>,
}

fn default_provider_retries() -> u32 {
    2
}

fn default_provider_backoff_ms() -> u64 {
    500
}

fn default_channel_backoff_secs() -> u64 {
    2
}

fn default_channel_backoff_max_secs() -> u64 {
    60
}

fn default_scheduler_poll_secs() -> u64 {
    15
}

fn default_scheduler_retries() -> u32 {
    2
}

fn default_scheduler_failure_budget() -> u32 {
    0
}

fn default_scheduler_breaker_cooldown_secs() -> u64 {
    0
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            provider_retries: default_provider_retries(),
            provider_backoff_ms: default_provider_backoff_ms(),
            fallback_providers: Vec::new(),
            channel_initial_backoff_secs: default_channel_backoff_secs(),
            channel_max_backoff_secs: default_channel_backoff_max_secs(),
            scheduler_poll_secs: default_scheduler_poll_secs(),
            scheduler_retries: default_scheduler_retries(),
            scheduler_failure_budget: default_scheduler_failure_budget(),
            scheduler_breaker_cooldown_secs: default_scheduler_breaker_cooldown_secs(),
            scheduler_active_hours_start_utc: None,
            scheduler_active_hours_end_utc: None,
        }
    }
}

/// Heartbeat (periodic self-check) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Whether heartbeat is enabled. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Interval between heartbeats in minutes. Default: 30.
    #[serde(default = "default_heartbeat_interval_minutes")]
    pub interval_minutes: u32,
}

fn default_heartbeat_interval_minutes() -> u32 {
    30
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 30,
        }
    }
}
