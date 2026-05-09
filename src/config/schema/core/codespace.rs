//! Sandboxed codespace development environment configuration.
//! Controls project limits, allowed languages, promotion gates,
//! and test-timeout settings.

use serde::{Deserialize, Serialize};

/// Gate strictness for promoting codespace projects to skills.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromoteGateLevel {
    /// Layer 1 only (code patterns). Provenance overridden.
    Minimal,
    /// Layers 1-2 (code + metadata). Provenance overridden.
    #[default]
    Standard,
    /// All 4 layers, no provenance override.
    Strict,
}

/// Configuration for the sandboxed codespace development environment.
///
/// When enabled, the agent can create projects, write code, run tests,
/// and promote successful projects to skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodespaceConfig {
    /// Whether the codespace tool is available. Default: false.
    #[serde(default)]
    pub enabled: bool,

    /// Subdirectory name under workspace for codespace projects.
    #[serde(default = "default_root_dir")]
    pub root_dir: String,

    /// Languages the agent is allowed to use in codespace projects.
    #[serde(default = "default_allowed_languages")]
    pub allowed_languages: Vec<String>,

    /// Maximum total size of a single project in megabytes.
    #[serde(default = "default_max_project_size_mb")]
    pub max_project_size_mb: u64,

    /// Maximum number of concurrent projects in the codespace.
    #[serde(default = "default_max_projects")]
    pub max_projects: usize,

    /// Automatically promote projects after tests pass.
    #[serde(default)]
    pub auto_promote: bool,

    /// Gate strictness for promotion evaluation.
    #[serde(default)]
    pub promote_gate_level: PromoteGateLevel,

    /// Timeout in seconds for test and exec commands.
    #[serde(default = "default_test_timeout_secs")]
    pub test_timeout_secs: u64,
}

impl Default for CodespaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            root_dir: default_root_dir(),
            allowed_languages: default_allowed_languages(),
            max_project_size_mb: default_max_project_size_mb(),
            max_projects: default_max_projects(),
            auto_promote: false,
            promote_gate_level: PromoteGateLevel::default(),
            test_timeout_secs: default_test_timeout_secs(),
        }
    }
}

fn default_root_dir() -> String {
    "codespace".into()
}

fn default_allowed_languages() -> Vec<String> {
    vec![
        "python".into(),
        "bash".into(),
        "rust".into(),
        "javascript".into(),
    ]
}

fn default_max_project_size_mb() -> u64 {
    256
}

fn default_max_projects() -> usize {
    20
}

fn default_test_timeout_secs() -> u64 {
    120
}

#[cfg(test)]
mod tests {
    use crate::config::schema::core::codespace::{CodespaceConfig, PromoteGateLevel};

    #[test]
    fn default_config_is_disabled() {
        let cfg = CodespaceConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.root_dir, "codespace");
        assert_eq!(cfg.max_projects, 20);
        assert_eq!(cfg.max_project_size_mb, 256);
        assert_eq!(cfg.test_timeout_secs, 120);
        assert_eq!(cfg.promote_gate_level, PromoteGateLevel::Standard);
        assert!(!cfg.auto_promote);
    }

    #[test]
    fn default_languages_include_expected() {
        let cfg = CodespaceConfig::default();
        assert!(cfg.allowed_languages.contains(&"python".to_string()));
        assert!(cfg.allowed_languages.contains(&"bash".to_string()));
        assert!(cfg.allowed_languages.contains(&"rust".to_string()));
        assert!(cfg.allowed_languages.contains(&"javascript".to_string()));
    }

    #[test]
    fn roundtrip_serde() {
        let cfg = CodespaceConfig::default();
        let toml_str = toml::to_string(&cfg).expect("serialize");
        let decoded: CodespaceConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(decoded.root_dir, cfg.root_dir);
        assert_eq!(decoded.max_projects, cfg.max_projects);
    }

    #[test]
    fn gate_level_serde_roundtrip() {
        let val = PromoteGateLevel::Strict;
        let json = serde_json::to_string(&val).expect("serialize");
        assert_eq!(json, "\"strict\"");
        let decoded: PromoteGateLevel = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, PromoteGateLevel::Strict);
    }
}
