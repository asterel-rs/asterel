use std::fmt;

use serde::{Deserialize, Serialize};

/// A single capability that a tool may require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Access to the local filesystem (read/write).
    Filesystem,
    /// Access to network resources (HTTP, sockets, etc.).
    Network,
    /// Ability to spawn shell processes.
    Shell,
    /// Ability to write to the memory subsystem.
    MemoryWrite,
    /// Ability to perform external actions (composio, MCP, games).
    ExternalAction,
    /// Read-only access to cognitive/persona internal state.
    CognitiveRead,
    /// Write access to cognitive state (strategy, uncertainty, narrative).
    CognitiveWrite,
    /// Unrestricted access — implies all other capabilities.
    Unrestricted,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Filesystem => write!(f, "filesystem"),
            Self::Network => write!(f, "network"),
            Self::Shell => write!(f, "shell"),
            Self::MemoryWrite => write!(f, "memory_write"),
            Self::ExternalAction => write!(f, "external_action"),
            Self::CognitiveRead => write!(f, "cognitive_read"),
            Self::CognitiveWrite => write!(f, "cognitive_write"),
            Self::Unrestricted => write!(f, "unrestricted"),
        }
    }
}

/// Phase of an external action lifecycle (§6.4.E).
///
/// External actions should not collapse success into a single boolean.
/// These phases let the system distinguish between "the tool ran" and
/// "the user's goal was achieved."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPhase {
    /// Tool function was called and returned a result.
    Executed,
    /// Expected state change was observed (e.g. file created, message sent).
    StateChanged,
    /// User-visible outcome was confirmed (e.g. user acknowledged).
    OutcomeConfirmed,
    /// Follow-up action is required based on the result.
    FollowUpRequired,
}

/// Effect classification for a tool execution (WP-G3).
///
/// Finer-grained than `Capability` — describes *what happens* when the tool
/// runs, not what resources it needs. Used by the policy engine and security
/// middleware for risk-based decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    /// Only reads data, no side effects.
    ReadOnly,
    /// Mutates local state (files, memory, config).
    LocalMutation,
    /// Potentially irreversible local operation (delete, shell rm, drop).
    Destructive,
    /// Communicates with external services (HTTP, MCP, email).
    RemoteAction,
}

impl ToolEffect {
    /// Classify a tool by name using built-in heuristics.
    ///
    /// Tools not recognized default to `LocalMutation` (conservative).
    #[must_use]
    pub fn classify(tool_name: &str) -> Self {
        match tool_name {
            "file_read"
            | "memory_recall"
            | "memory_lookup"
            | "memory_search"
            | "introspect_affect"
            | "introspect_persona"
            | "introspect_relationship"
            | "introspect_self_model"
            | "introspect_principles"
            | "introspect_experience"
            | "evaluate_consistency" => Self::ReadOnly,

            "shell" => Self::Destructive,

            "web_fetch"
            | "web_search"
            | "web_scrape"
            | "duckduckgo_search"
            | "browser"
            | "browser_open"
            | "composio"
            | "email_send"
            | "channel_create_thread"
            | "channel_add_reaction"
            | "channel_send_rich"
            | "channel_get_history"
            | "channel_send_embed" => Self::RemoteAction,

            name if name.starts_with("mcp_") => Self::RemoteAction,

            // Everything else (file_write, file_edit, memory_store, etc.) defaults
            // to LocalMutation — the conservative middle ground.
            _ => Self::LocalMutation,
        }
    }

    /// Whether this effect is potentially dangerous and warrants extra scrutiny.
    #[must_use]
    pub fn is_high_risk(self) -> bool {
        matches!(self, Self::Destructive | Self::RemoteAction)
    }
}

impl fmt::Display for ToolEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => write!(f, "read_only"),
            Self::LocalMutation => write!(f, "local_mutation"),
            Self::Destructive => write!(f, "destructive"),
            Self::RemoteAction => write!(f, "remote_action"),
        }
    }
}

/// Description of a tool for the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Unique tool name for LLM function-calling dispatch.
    pub name: String,
    /// Human-readable description of the tool's purpose.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub parameters: serde_json::Value,
    /// Capabilities this tool requires to execute.
    #[serde(default)]
    pub required_capabilities: Vec<Capability>,
    /// Effect classification for policy decisions.
    #[serde(default = "default_tool_effect")]
    pub effect: ToolEffect,
}

fn default_tool_effect() -> ToolEffect {
    ToolEffect::LocalMutation
}

impl ToolSpec {
    /// Create a tool spec with auto-classified effect.
    #[must_use]
    pub fn with_auto_effect(
        name: String,
        description: String,
        parameters: serde_json::Value,
        required_capabilities: Vec<Capability>,
    ) -> Self {
        let effect = ToolEffect::classify(&name);
        Self {
            name,
            description,
            parameters,
            required_capabilities,
            effect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_read_only_tools() {
        assert_eq!(ToolEffect::classify("file_read"), ToolEffect::ReadOnly);
        assert_eq!(ToolEffect::classify("memory_recall"), ToolEffect::ReadOnly);
        assert_eq!(
            ToolEffect::classify("introspect_affect"),
            ToolEffect::ReadOnly
        );
        for read_only_introspection_tool in [
            "introspect_self_model",
            "introspect_principles",
            "introspect_experience",
            "evaluate_consistency",
        ] {
            assert_eq!(
                ToolEffect::classify(read_only_introspection_tool),
                ToolEffect::ReadOnly
            );
        }
    }

    #[test]
    fn classify_destructive_tools() {
        assert_eq!(ToolEffect::classify("shell"), ToolEffect::Destructive);
    }

    #[test]
    fn classify_remote_action_tools() {
        assert_eq!(ToolEffect::classify("web_fetch"), ToolEffect::RemoteAction);
        assert_eq!(ToolEffect::classify("composio"), ToolEffect::RemoteAction);
        assert_eq!(ToolEffect::classify("mcp_github"), ToolEffect::RemoteAction);
        assert_eq!(
            ToolEffect::classify("mcp_anything"),
            ToolEffect::RemoteAction
        );
        for channel_tool in [
            "channel_create_thread",
            "channel_add_reaction",
            "channel_send_rich",
            "channel_get_history",
            "channel_send_embed",
        ] {
            assert_eq!(ToolEffect::classify(channel_tool), ToolEffect::RemoteAction);
        }
    }

    #[test]
    fn classify_defaults_to_local_mutation() {
        assert_eq!(
            ToolEffect::classify("file_write"),
            ToolEffect::LocalMutation
        );
        assert_eq!(
            ToolEffect::classify("unknown_tool"),
            ToolEffect::LocalMutation
        );
    }

    #[test]
    fn is_high_risk_flags_destructive_and_remote() {
        assert!(ToolEffect::Destructive.is_high_risk());
        assert!(ToolEffect::RemoteAction.is_high_risk());
        assert!(!ToolEffect::ReadOnly.is_high_risk());
        assert!(!ToolEffect::LocalMutation.is_high_risk());
    }
}
