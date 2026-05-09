//! Per-tool enable/disable flags for shell, file I/O, memory store,
//! memory recall, memory lookup, memory correct, memory forget, and memory
//! governance tools.

use serde::{Deserialize, Serialize};

use super::default_true;

fn default_tool_enabled() -> ToolEntry {
    ToolEntry { enabled: true }
}

fn default_tool_disabled() -> ToolEntry {
    ToolEntry { enabled: false }
}

fn default_loop_history_size() -> usize {
    8
}

fn default_loop_warning_threshold() -> u32 {
    2
}

fn default_loop_critical_threshold() -> u32 {
    4
}

/// Enable/disable flag for a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    /// Whether this tool is enabled. Default: true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Loop-pattern detection guardrails for provider/tool iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // Config surface intentionally exposes independent toggles.
pub struct LoopDetectionConfig {
    /// Enables pattern detection in the tool loop.
    #[serde(default)]
    pub enabled: bool,
    /// Number of recent turns retained for pattern checks.
    #[serde(default = "default_loop_history_size")]
    pub history_size: usize,
    /// Warning threshold for repeated suspicious patterns.
    #[serde(default = "default_loop_warning_threshold")]
    pub warning_threshold: u32,
    /// Hard-stop threshold for repeated suspicious patterns.
    #[serde(default = "default_loop_critical_threshold")]
    pub critical_threshold: u32,
    /// Detect immediate exact response/tool repetition.
    #[serde(default = "default_true")]
    pub repeat: bool,
    /// Detect alternating A/B/A/B loop patterns.
    #[serde(default = "default_true")]
    pub ping_pong: bool,
    /// Detect repeated tool-use turns with no visible progress.
    #[serde(default = "default_true")]
    pub no_progress: bool,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            history_size: default_loop_history_size(),
            warning_threshold: default_loop_warning_threshold(),
            critical_threshold: default_loop_critical_threshold(),
            repeat: true,
            ping_pong: true,
            no_progress: true,
        }
    }
}

/// Per-tool enable/disable configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Shell command execution tool. Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub shell: ToolEntry,
    /// File read tool. Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub file_read: ToolEntry,
    /// File write tool. Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub file_write: ToolEntry,
    /// File delete tool (agent-owned files only). Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub file_delete: ToolEntry,
    /// Memory store tool. Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub memory_store: ToolEntry,
    /// Memory recall tool. Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub memory_recall: ToolEntry,
    /// Memory lookup tool (resolve single belief slot). Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub memory_lookup: ToolEntry,
    /// Memory correct tool (correct stored facts). Default: enabled.
    #[serde(default = "default_tool_enabled")]
    pub memory_correct: ToolEntry,
    /// Memory forget tool. Default: disabled.
    #[serde(default = "default_tool_disabled")]
    pub memory_forget: ToolEntry,
    /// Memory governance tool. Default: disabled.
    #[serde(default = "default_tool_disabled")]
    pub memory_governance: ToolEntry,
    /// Tool-loop pattern detection controls.
    #[serde(default)]
    pub loop_detection: LoopDetectionConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            shell: ToolEntry { enabled: true },
            file_read: ToolEntry { enabled: true },
            file_write: ToolEntry { enabled: true },
            file_delete: ToolEntry { enabled: true },
            memory_store: ToolEntry { enabled: true },
            memory_recall: ToolEntry { enabled: true },
            memory_lookup: ToolEntry { enabled: true },
            memory_correct: ToolEntry { enabled: true },
            memory_forget: ToolEntry { enabled: false },
            memory_governance: ToolEntry { enabled: false },
            loop_detection: LoopDetectionConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tools_config_has_correct_enabled_flags() {
        let cfg = ToolsConfig::default();
        assert!(cfg.shell.enabled);
        assert!(cfg.file_read.enabled);
        assert!(cfg.file_write.enabled);
        assert!(cfg.file_delete.enabled);
        assert!(cfg.memory_store.enabled);
        assert!(cfg.memory_recall.enabled);
        assert!(cfg.memory_lookup.enabled);
        assert!(cfg.memory_correct.enabled);
        assert!(!cfg.memory_forget.enabled);
        assert!(!cfg.memory_governance.enabled);
        assert!(!cfg.loop_detection.enabled);
        assert_eq!(cfg.loop_detection.history_size, 8);
        assert_eq!(cfg.loop_detection.warning_threshold, 2);
        assert_eq!(cfg.loop_detection.critical_threshold, 4);
        assert!(cfg.loop_detection.repeat);
        assert!(cfg.loop_detection.ping_pong);
        assert!(cfg.loop_detection.no_progress);
    }

    #[test]
    fn tools_config_deserialize_with_defaults() {
        let toml_str = "[tools]";
        let cfg: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.shell.enabled);
        assert!(cfg.file_read.enabled);
        assert!(cfg.file_write.enabled);
        assert!(cfg.file_delete.enabled);
        assert!(cfg.memory_store.enabled);
        assert!(cfg.memory_recall.enabled);
        assert!(cfg.memory_lookup.enabled);
        assert!(cfg.memory_correct.enabled);
        assert!(!cfg.memory_forget.enabled);
        assert!(!cfg.memory_governance.enabled);
        assert!(!cfg.loop_detection.enabled);
    }

    #[test]
    fn tools_config_deserialize_with_overrides() {
        let toml_str = r"
shell = { enabled = false }
memory_forget = { enabled = true }
";
        let cfg: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert!(!cfg.shell.enabled);
        assert!(cfg.file_read.enabled);
        assert!(cfg.memory_forget.enabled);
    }

    #[test]
    fn tools_config_all_disabled() {
        let cfg = ToolsConfig {
            shell: ToolEntry { enabled: false },
            file_read: ToolEntry { enabled: false },
            file_write: ToolEntry { enabled: false },
            file_delete: ToolEntry { enabled: false },
            memory_store: ToolEntry { enabled: false },
            memory_recall: ToolEntry { enabled: false },
            memory_lookup: ToolEntry { enabled: false },
            memory_correct: ToolEntry { enabled: false },
            memory_forget: ToolEntry { enabled: false },
            memory_governance: ToolEntry { enabled: false },
            loop_detection: LoopDetectionConfig {
                enabled: true,
                history_size: 4,
                warning_threshold: 1,
                critical_threshold: 2,
                repeat: true,
                ping_pong: true,
                no_progress: true,
            },
        };
        assert!(!cfg.shell.enabled);
        assert!(!cfg.file_read.enabled);
        assert!(!cfg.file_write.enabled);
        assert!(!cfg.file_delete.enabled);
        assert!(!cfg.memory_store.enabled);
        assert!(!cfg.memory_recall.enabled);
        assert!(!cfg.memory_lookup.enabled);
        assert!(!cfg.memory_correct.enabled);
        assert!(!cfg.memory_forget.enabled);
        assert!(!cfg.memory_governance.enabled);
        assert!(cfg.loop_detection.enabled);
    }

    #[test]
    fn loop_detection_deserializes_custom_values() {
        let cfg: ToolsConfig = toml::from_str(
            r"
[loop_detection]
enabled = true
history_size = 6
warning_threshold = 3
critical_threshold = 5
repeat = true
ping_pong = false
no_progress = true
",
        )
        .unwrap();

        assert!(cfg.loop_detection.enabled);
        assert_eq!(cfg.loop_detection.history_size, 6);
        assert_eq!(cfg.loop_detection.warning_threshold, 3);
        assert_eq!(cfg.loop_detection.critical_threshold, 5);
        assert!(cfg.loop_detection.repeat);
        assert!(!cfg.loop_detection.ping_pong);
        assert!(cfg.loop_detection.no_progress);
    }
}
