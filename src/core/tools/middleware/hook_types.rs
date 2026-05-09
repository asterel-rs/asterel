//! Data types for the external hook execution system (WP-G2).
//!
//! These types define the stdin/stdout contract between `Asterel` and
//! external hook subprocesses.  See `middleware::hooks` for the execution
//! logic and exit-code semantics.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, SessionId};

/// Lifecycle event that triggers a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before a tool is executed.
    PreToolUse,
    /// After a tool completes successfully.
    PostToolUse,
    /// After a tool execution fails.
    PostToolUseFailure,
}

/// JSON payload written to a hook subprocess's stdin.
///
/// Serialised with `serde_json` before being piped to the hook.  The hook
/// can use any fields for policy decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPayload {
    /// Which lifecycle event triggered this hook.
    pub event: HookEvent,
    /// Canonical name of the tool being executed (e.g. `"shell"`, `"file_write"`).
    pub tool_name: String,
    /// Tool arguments as a JSON value (`null` for `post_tool_use` hooks where
    /// the args are no longer relevant).
    pub args: serde_json::Value,
    /// Current session ID for correlation.
    #[serde(default = "default_session_id")]
    pub session_id: SessionId,
    /// Entity that triggered this tool call.
    #[serde(default = "default_entity_id")]
    pub entity_id: EntityId,
}

/// JSON response read from a hook subprocess's stdout (exit code `0`).
///
/// All fields are optional.  An empty JSON object (`{}`) or blank stdout
/// means the hook ran successfully but does not override anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResponse {
    /// Decision override.  `None` means "no opinion — defer to the next hook
    /// or the default policy".
    #[serde(default)]
    pub decision: Option<HookDecision>,
    /// Optional system-level message to inject into the session context.
    #[serde(default)]
    pub system_message: Option<String>,
    /// Optional replacement for the tool's input arguments.  When set, the
    /// hook middleware substitutes these args before the tool executes.
    #[serde(default)]
    pub updated_input: Option<serde_json::Value>,
}

/// A hook's decision about whether to allow, deny, or ask for approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookDecision {
    /// Allow the tool call.
    Allow,
    /// Block the tool call.
    Deny,
    /// Force interactive approval.
    Ask,
}

/// Configuration for a single external hook.
///
/// Stored in `hooks.toml` and loaded at startup via [`HookConfigSet::from_toml`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Shell command to execute (run via `sh -c`).
    pub command: String,
    /// Lifecycle events that trigger this hook.
    pub events: Vec<HookEvent>,
    /// Maximum seconds to wait for the hook before treating it as an error.
    /// Defaults to 10 seconds.
    #[serde(default = "default_hook_timeout")]
    pub timeout_secs: u64,
    /// Whether this hook is active.  Disabled hooks are skipped without
    /// executing.  Defaults to `true`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_session_id() -> SessionId {
    SessionId::new("")
}

fn default_entity_id() -> EntityId {
    EntityId::new("")
}

fn default_hook_timeout() -> u64 {
    10
}

fn default_enabled() -> bool {
    true
}

/// Ordered collection of [`HookConfig`] entries.
///
/// Loaded from `hooks.toml` via [`HookConfigSet::from_toml`].  The hooks
/// in `hooks` are evaluated in declaration order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfigSet {
    /// Ordered list of hook configurations.
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
}

impl HookConfigSet {
    /// Load from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML is malformed.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }
}
