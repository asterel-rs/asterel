//! Typed extension contract for skills, agents, hooks, and MCP packages.
//!
//! `extension.toml` is the machine-readable entrypoint. Markdown bodies
//! remain external files so humans can author prompts and instructions
//! without losing a typed runtime contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::schema::{McpConfig, McpServerConfig, McpTransport};
use crate::plugins::skills::SkillTool;
use crate::security::{
    RootBoundPathKind, canonicalize_path_within_root, resolve_relative_path_within_root,
};

const WORKSPACE_EXTENSIONS_ENV: &str = "ASTEREL_ENABLE_WORKSPACE_EXTENSIONS";
const WORKSPACE_AGENT_EXTENSIONS_ENV: &str = "ASTEREL_ENABLE_WORKSPACE_AGENT_EXTENSIONS";
const WORKSPACE_MCP_EXTENSIONS_ENV: &str = "ASTEREL_ENABLE_WORKSPACE_MCP_EXTENSIONS";

/// Supported extension kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Skill,
    Agent,
    Hook,
    Mcp,
}

/// Typed capability identifier declared by an extension manifest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExtensionCapabilityId(pub String);

impl ExtensionCapabilityId {
    /// Borrow the underlying identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Typed permission identifier declared by an extension manifest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExtensionPermissionId(pub String);

impl ExtensionPermissionId {
    /// Borrow the underlying identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Shared metadata for all extension kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionMetadata {
    /// Stable extension identifier.
    pub id: String,
    /// Extension family.
    pub kind: ExtensionKind,
    /// Human-readable description.
    pub description: String,
    /// Manifest version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Optional author attribution.
    #[serde(default)]
    pub author: Option<String>,
    /// Discovery tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Declared capabilities for runtime enforcement.
    #[serde(default)]
    pub capabilities: Vec<ExtensionCapabilityId>,
    /// Declared permissions for runtime enforcement.
    #[serde(default)]
    pub permissions: Vec<ExtensionPermissionId>,
}

/// External requirements that must be satisfied before loading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionRequirements {
    /// Commands that must exist in `PATH`.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Environment variables that must be present.
    #[serde(default)]
    pub env: Vec<String>,
}

/// Skill-specific runtime contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillExtensionSpec {
    /// Tool definitions exported by the skill.
    #[serde(default)]
    pub tools: Vec<SkillTool>,
    /// Inline prompts embedded directly in the manifest.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Markdown prompt bodies resolved relative to `extension.toml`.
    #[serde(default)]
    pub prompt_bodies: Vec<String>,
}

/// Agent-specific runtime contract.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentExtensionSpec {
    /// Optional role label for the agent.
    #[serde(default)]
    pub role: Option<String>,
    /// Inline prompts embedded directly in the manifest.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Markdown prompt bodies resolved relative to `extension.toml`.
    #[serde(default)]
    pub prompt_bodies: Vec<String>,
    /// Optional model override for this agent profile.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional temperature override for this agent profile.
    #[serde(default)]
    pub temperature: Option<f64>,
}

/// Hook-specific runtime contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookExtensionSpec {
    /// Optional hook event selector.
    #[serde(default)]
    pub event: Option<String>,
    /// Markdown prompt bodies resolved relative to `extension.toml`.
    #[serde(default)]
    pub prompt_bodies: Vec<String>,
}

/// MCP-specific runtime contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpExtensionSpec {
    /// Optional transport label such as `stdio` or `http`.
    #[serde(default)]
    pub transport: Option<String>,
    /// Optional entry command for local MCP servers.
    #[serde(default)]
    pub command: Option<String>,
    /// Optional command arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional command environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional URL for HTTP MCP servers.
    #[serde(default)]
    pub url: Option<String>,
    /// Optional enable flag.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Optional per-call timeout override in seconds.
    #[serde(default)]
    pub max_call_seconds: Option<u64>,
}

/// Top-level typed manifest parsed from `extension.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionManifest {
    /// Shared extension metadata.
    pub extension: ExtensionMetadata,
    /// Commands/env required before activation.
    #[serde(default)]
    pub requirements: ExtensionRequirements,
    /// Skill-specific configuration when `kind = "skill"`.
    #[serde(default)]
    pub skill: Option<SkillExtensionSpec>,
    /// Agent-specific configuration when `kind = "agent"`.
    #[serde(default)]
    pub agent: Option<AgentExtensionSpec>,
    /// Hook-specific configuration when `kind = "hook"`.
    #[serde(default)]
    pub hook: Option<HookExtensionSpec>,
    /// MCP-specific configuration when `kind = "mcp"`.
    #[serde(default)]
    pub mcp: Option<McpExtensionSpec>,
}

impl ExtensionManifest {
    /// Return prompt-body paths for the active extension kind.
    #[must_use]
    pub fn prompt_body_paths(&self) -> Vec<PathBuf> {
        let raw_paths = match self.extension.kind {
            ExtensionKind::Skill => self
                .skill
                .as_ref()
                .map_or_else(Vec::new, |spec| spec.prompt_bodies.clone()),
            ExtensionKind::Agent => self
                .agent
                .as_ref()
                .map_or_else(Vec::new, |spec| spec.prompt_bodies.clone()),
            ExtensionKind::Hook => self
                .hook
                .as_ref()
                .map_or_else(Vec::new, |spec| spec.prompt_bodies.clone()),
            ExtensionKind::Mcp => Vec::new(),
        };

        raw_paths
            .into_iter()
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect()
    }
}

/// Loaded markdown body referenced by the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionBody {
    /// Relative path declared in the manifest.
    pub relative_path: PathBuf,
    /// Absolute resolved path on disk.
    pub absolute_path: PathBuf,
    /// File contents.
    pub content: String,
}

/// Parsed extension manifest without loading prompt bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionManifestSpec {
    /// Path to the parsed `extension.toml`.
    pub manifest_path: PathBuf,
    /// Parsed manifest document.
    pub manifest: ExtensionManifest,
}

/// Runtime-ready extension contract plus resolved markdown bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionRuntimeSpec {
    /// Path to the parsed `extension.toml`.
    pub manifest_path: PathBuf,
    /// Parsed manifest document.
    pub manifest: ExtensionManifest,
    /// Resolved markdown bodies.
    pub bodies: Vec<ExtensionBody>,
}

/// Runtime-ready agent extension derived from `extension.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentExtensionRuntime {
    /// Stable extension identifier.
    pub id: String,
    /// Optional role alias this profile is intended for.
    pub role: Option<String>,
    /// Resolved system prompt text from inline prompts and Markdown bodies.
    pub system_prompt: String,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional temperature override.
    pub temperature: Option<f64>,
    /// Path to the source manifest.
    pub manifest_path: PathBuf,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// Load an extension manifest and its referenced markdown bodies.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read, parsed, or when any
/// declared markdown body cannot be loaded relative to the manifest path.
pub fn load_extension_runtime(path: &Path) -> Result<ExtensionRuntimeSpec> {
    let ExtensionManifestSpec {
        manifest_path,
        manifest,
    } = load_extension_manifest(path)?;
    let mut bodies = Vec::new();

    for relative_path in manifest.prompt_body_paths() {
        let absolute_path = resolve_extension_body_path(&manifest_path, &relative_path)?;
        let body_content = std::fs::read_to_string(&absolute_path).with_context(|| {
            format!(
                "read extension body '{}' declared by '{}'",
                absolute_path.display(),
                manifest_path.display()
            )
        })?;
        bodies.push(ExtensionBody {
            relative_path,
            absolute_path,
            content: body_content,
        });
    }

    Ok(ExtensionRuntimeSpec {
        manifest_path,
        manifest,
        bodies,
    })
}

/// Resolve a prompt body path declared by an extension manifest.
///
/// # Errors
///
/// Returns an error if the declared path is absolute, traverses upward, escapes
/// the manifest directory via symlink/canonicalization, or is not a file.
pub fn resolve_extension_body_path(manifest_path: &Path, declared_path: &Path) -> Result<PathBuf> {
    if declared_path.is_absolute() {
        anyhow::bail!(
            "extension manifest '{}' references absolute prompt body '{}'",
            manifest_path.display(),
            declared_path.display()
        );
    }

    if declared_path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        anyhow::bail!(
            "extension manifest '{}' references prompt body outside its root: '{}'",
            manifest_path.display(),
            declared_path.display()
        );
    }

    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    match resolve_relative_path_within_root(base_dir, declared_path, RootBoundPathKind::File) {
        Ok(path) => Ok(path),
        Err(error) => {
            let message = error.to_string();
            if message.contains("outside allowed root") {
                anyhow::bail!(
                    "extension manifest '{}' references prompt body outside its root: '{}'",
                    manifest_path.display(),
                    declared_path.display()
                );
            }
            if message.contains("not a file") || message.contains("canonicalize path") {
                anyhow::bail!(
                    "extension manifest '{}' references missing prompt body '{}'",
                    manifest_path.display(),
                    declared_path.display()
                );
            }
            Err(error).with_context(|| {
                format!(
                    "resolve extension body '{}' declared by '{}'",
                    declared_path.display(),
                    manifest_path.display()
                )
            })
        }
    }
}

/// Load and parse an extension manifest without resolving prompt bodies.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read or parsed.
fn resolve_extension_manifest_path(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect extension manifest '{}'", path.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!(
            "extension manifest '{}' must not be a symlink",
            path.display()
        );
    }
    if !metadata.is_file() {
        anyhow::bail!("extension manifest '{}' is not a file", path.display());
    }

    let root = path.parent().unwrap_or_else(|| Path::new("."));
    canonicalize_path_within_root(path, root, RootBoundPathKind::File)
        .with_context(|| format!("canonicalize extension manifest '{}'", path.display()))
}

/// Load and parse an extension manifest without resolving prompt bodies.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read or parsed.
pub fn load_extension_manifest(path: &Path) -> Result<ExtensionManifestSpec> {
    let manifest_path = resolve_extension_manifest_path(path)?;
    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("read extension manifest '{}'", manifest_path.display()))?;
    let manifest: ExtensionManifest =
        toml::from_str(&content).with_context(|| format!("parse '{}'", manifest_path.display()))?;
    Ok(ExtensionManifestSpec {
        manifest_path,
        manifest,
    })
}

/// Root directory for file-driven extensions in the workspace.
#[must_use]
pub fn extensions_root_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("extensions")
}

/// Directory containing agent extension manifests.
#[must_use]
pub fn agent_extensions_dir(workspace_dir: &Path) -> PathBuf {
    extensions_root_dir(workspace_dir).join("agents")
}

/// Directory containing MCP extension manifests.
#[must_use]
pub fn mcp_extensions_dir(workspace_dir: &Path) -> PathBuf {
    extensions_root_dir(workspace_dir).join("mcp")
}

/// Load agent extension runtimes from the workspace extension directory.
#[must_use]
pub fn load_agent_extensions_from_workspace(workspace_dir: &Path) -> Vec<AgentExtensionRuntime> {
    if !workspace_extension_loading_enabled(ExtensionKind::Agent) {
        return Vec::new();
    }
    load_workspace_extension_manifests(&agent_extensions_dir(workspace_dir), ExtensionKind::Agent)
        .into_iter()
        .filter_map(agent_runtime_from_extension)
        .collect()
}

/// Load MCP server configs declared via workspace extension manifests.
#[must_use]
pub fn load_mcp_server_configs_from_workspace(workspace_dir: &Path) -> Vec<McpServerConfig> {
    if !workspace_extension_loading_enabled(ExtensionKind::Mcp) {
        return Vec::new();
    }
    load_workspace_extension_manifests(&mcp_extensions_dir(workspace_dir), ExtensionKind::Mcp)
        .into_iter()
        .filter_map(mcp_server_from_extension)
        .collect()
}

/// Merge config-defined MCP servers with workspace extension manifests.
///
/// Explicit config entries take precedence over file-driven extension entries
/// with the same server name.
#[must_use]
pub fn merge_mcp_config_with_workspace_extensions(
    config: &McpConfig,
    workspace_dir: &Path,
) -> McpConfig {
    let explicit_names = config
        .servers
        .iter()
        .map(|server| server.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut merged_servers = load_mcp_server_configs_from_workspace(workspace_dir)
        .into_iter()
        .filter(|server| !explicit_names.contains(server.name.as_str()))
        .collect::<Vec<_>>();
    merged_servers.extend(config.servers.clone());

    McpConfig {
        enabled: config.enabled,
        import_json: config.import_json.clone(),
        servers: merged_servers,
    }
}

fn load_workspace_extension_manifests(
    root: &Path,
    expected_kind: ExtensionKind,
) -> Vec<ExtensionRuntimeSpec> {
    if !root.exists() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        tracing::warn!(path = %root.display(), "failed to read extension root");
        return Vec::new();
    };

    let mut runtimes = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let manifest_path = entry.path().join("extension.toml");
        if !manifest_path.exists() {
            continue;
        }

        match load_extension_runtime(&manifest_path) {
            Ok(runtime) if runtime.manifest.extension.kind == expected_kind => {
                runtimes.push(runtime);
            }
            Ok(runtime) => {
                tracing::warn!(
                    path = %manifest_path.display(),
                    expected = %kind_label(expected_kind),
                    actual = %kind_label(runtime.manifest.extension.kind),
                    "skipping extension with unexpected kind"
                );
            }
            Err(error) => {
                tracing::warn!(
                    path = %manifest_path.display(),
                    error = %error,
                    "failed to load extension manifest"
                );
            }
        }
    }

    runtimes
}

fn agent_runtime_from_extension(runtime: ExtensionRuntimeSpec) -> Option<AgentExtensionRuntime> {
    let agent = runtime.manifest.agent.as_ref()?;
    let mut sections = agent.prompts.clone();
    sections.extend(runtime.bodies.into_iter().map(|body| body.content));
    let mut system_prompt = String::new();
    for section in sections {
        let trimmed = section.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !system_prompt.is_empty() {
            system_prompt.push_str("\n\n");
        }
        system_prompt.push_str(trimmed);
    }

    Some(AgentExtensionRuntime {
        id: runtime.manifest.extension.id,
        role: agent.role.clone(),
        system_prompt,
        model: agent.model.clone(),
        temperature: agent.temperature,
        manifest_path: runtime.manifest_path,
    })
}

fn mcp_server_from_extension(runtime: ExtensionRuntimeSpec) -> Option<McpServerConfig> {
    let manifest = runtime.manifest;
    let mcp = manifest.mcp?;
    let transport = match mcp.transport.as_deref().map(str::trim) {
        Some("http") => {
            let url = mcp.url?.trim().to_string();
            if url.is_empty() {
                return None;
            }
            McpTransport::Http { url }
        }
        Some("stdio") | None => {
            let command = mcp.command?.trim().to_string();
            if command.is_empty() {
                return None;
            }
            McpTransport::Stdio {
                command,
                args: mcp.args,
                env: mcp.env,
            }
        }
        Some(other) => {
            tracing::warn!(
                manifest = %runtime.manifest_path.display(),
                transport = other,
                "unsupported MCP extension transport"
            );
            return None;
        }
    };

    Some(McpServerConfig {
        name: manifest.extension.id,
        transport,
        enabled: mcp.enabled.unwrap_or(true),
        max_call_seconds: mcp.max_call_seconds.unwrap_or(30),
    })
}

fn kind_label(kind: ExtensionKind) -> &'static str {
    match kind {
        ExtensionKind::Skill => "skill",
        ExtensionKind::Agent => "agent",
        ExtensionKind::Hook => "hook",
        ExtensionKind::Mcp => "mcp",
    }
}

fn workspace_extension_loading_enabled(kind: ExtensionKind) -> bool {
    env_flag_enabled(WORKSPACE_EXTENSIONS_ENV)
        || match kind {
            ExtensionKind::Agent => env_flag_enabled(WORKSPACE_AGENT_EXTENSIONS_ENV),
            ExtensionKind::Mcp => env_flag_enabled(WORKSPACE_MCP_EXTENSIONS_ENV),
            ExtensionKind::Skill | ExtensionKind::Hook => false,
        }
}

fn env_flag_enabled(key: &str) -> bool {
    std::env::var(key).ok().is_some_and(|value| {
        matches!(
            value.trim(),
            "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
        )
    })
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: Tests hold `ENV_LOCK`, so environment mutation is serialized.
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                // SAFETY: Tests hold `ENV_LOCK`, so environment mutation is serialized.
                unsafe { std::env::set_var(self.key, previous) };
            } else {
                // SAFETY: Tests hold `ENV_LOCK`, so environment mutation is serialized.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    #[test]
    fn load_extension_runtime_reads_prompt_bodies() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("SKILL.md"), "# Demo\nBody\n").expect("write body");
        std::fs::write(
            dir.path().join("extension.toml"),
            r#"
[extension]
id = "demo"
kind = "skill"
description = "demo skill"

[skill]
prompt_bodies = ["SKILL.md"]
"#,
        )
        .expect("write manifest");

        let runtime =
            load_extension_runtime(&dir.path().join("extension.toml")).expect("load runtime");

        assert_eq!(runtime.manifest.extension.id, "demo");
        assert_eq!(runtime.manifest.extension.kind, ExtensionKind::Skill);
        assert_eq!(runtime.bodies.len(), 1);
        assert!(runtime.bodies[0].content.contains("Body"));
    }

    #[test]
    fn load_extension_runtime_defaults_manifest_version() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(
            dir.path().join("extension.toml"),
            r#"
[extension]
id = "demo"
kind = "hook"
description = "demo hook"
"#,
        )
        .expect("write manifest");

        let runtime =
            load_extension_runtime(&dir.path().join("extension.toml")).expect("load runtime");

        assert_eq!(runtime.manifest.extension.version, "0.1.0");
    }

    #[test]
    fn load_extension_manifest_does_not_require_prompt_bodies() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::write(
            dir.path().join("extension.toml"),
            r#"
[extension]
id = "demo"
kind = "skill"
description = "demo skill"

[skill]
prompt_bodies = ["missing.md"]
"#,
        )
        .expect("write manifest");

        let manifest =
            load_extension_manifest(&dir.path().join("extension.toml")).expect("load manifest");

        assert_eq!(manifest.manifest.extension.id, "demo");
        assert_eq!(
            manifest.manifest.prompt_body_paths(),
            vec![PathBuf::from("missing.md")]
        );
    }

    #[test]
    fn load_agent_extensions_from_workspace_reads_prompt_and_overrides() {
        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let dir = TempDir::new().expect("tempdir");
        let _enabled_guard = EnvVarGuard::set(WORKSPACE_AGENT_EXTENSIONS_ENV, "1");
        let agent_dir = agent_extensions_dir(dir.path()).join("planner");
        std::fs::create_dir_all(&agent_dir).expect("create agent dir");
        std::fs::write(agent_dir.join("AGENT.md"), "Follow the planner contract.")
            .expect("write agent body");
        std::fs::write(
            agent_dir.join("extension.toml"),
            r#"
[extension]
id = "planner"
kind = "agent"
description = "planner agent"

[agent]
role = "planner"
model = "planner-model"
temperature = 0.3
prompt_bodies = ["AGENT.md"]
"#,
        )
        .expect("write manifest");

        let loaded = load_agent_extensions_from_workspace(dir.path());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "planner");
        assert_eq!(loaded[0].role.as_deref(), Some("planner"));
        assert_eq!(loaded[0].model.as_deref(), Some("planner-model"));
        assert_eq!(loaded[0].temperature, Some(0.3));
        assert!(loaded[0].system_prompt.contains("planner contract"));
    }

    #[test]
    fn load_mcp_server_configs_from_workspace_reads_stdio_server() {
        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let dir = TempDir::new().expect("tempdir");
        let _enabled_guard = EnvVarGuard::set(WORKSPACE_MCP_EXTENSIONS_ENV, "1");
        let mcp_dir = mcp_extensions_dir(dir.path()).join("filesystem");
        std::fs::create_dir_all(&mcp_dir).expect("create mcp dir");
        std::fs::write(
            mcp_dir.join("extension.toml"),
            r#"
[extension]
id = "filesystem"
kind = "mcp"
description = "filesystem mcp"

[mcp]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem"]
max_call_seconds = 45
"#,
        )
        .expect("write manifest");

        let loaded = load_mcp_server_configs_from_workspace(dir.path());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "filesystem");
        assert_eq!(loaded[0].max_call_seconds, 45);
        match &loaded[0].transport {
            McpTransport::Stdio { command, args, .. } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            McpTransport::Http { .. } => panic!("expected stdio transport, got http transport"),
        }
    }

    #[test]
    fn merge_mcp_config_prefers_explicit_config_over_workspace_extensions() {
        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let dir = TempDir::new().expect("tempdir");
        let _enabled_guard = EnvVarGuard::set(WORKSPACE_MCP_EXTENSIONS_ENV, "1");
        let mcp_dir = mcp_extensions_dir(dir.path()).join("filesystem");
        std::fs::create_dir_all(&mcp_dir).expect("create mcp dir");
        std::fs::write(
            mcp_dir.join("extension.toml"),
            r#"
[extension]
id = "filesystem"
kind = "mcp"
description = "filesystem mcp"

[mcp]
transport = "stdio"
command = "npx"
"#,
        )
        .expect("write manifest");

        let config = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![McpServerConfig {
                name: "filesystem".to_string(),
                transport: McpTransport::Stdio {
                    command: "custom-binary".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                },
                enabled: true,
                max_call_seconds: 30,
            }],
        };

        let merged = merge_mcp_config_with_workspace_extensions(&config, dir.path());

        assert_eq!(merged.servers.len(), 1);
        match &merged.servers[0].transport {
            McpTransport::Stdio { command, .. } => assert_eq!(command, "custom-binary"),
            McpTransport::Http { .. } => panic!("expected stdio transport, got http transport"),
        }
    }

    #[test]
    fn load_extension_runtime_rejects_parent_dir_prompt_body_escape() {
        let dir = TempDir::new().expect("tempdir");
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "# Outside").expect("write outside body");
        let extension_dir = dir.path().join("skill");
        std::fs::create_dir_all(&extension_dir).expect("create extension dir");
        std::fs::write(
            extension_dir.join("extension.toml"),
            r#"
[extension]
id = "escape"
kind = "skill"
description = "escape skill"

[skill]
prompt_bodies = ["../outside.md"]
"#,
        )
        .expect("write manifest");

        let error = load_extension_runtime(&extension_dir.join("extension.toml"))
            .expect_err("parent-dir prompt body should be rejected");

        assert!(error.to_string().contains("outside its root"));
    }

    #[test]
    fn load_extension_runtime_rejects_absolute_prompt_body_path() {
        let dir = TempDir::new().expect("tempdir");
        let absolute = dir.path().join("outside.md");
        std::fs::write(&absolute, "# Outside").expect("write outside body");
        std::fs::write(
            dir.path().join("extension.toml"),
            format!(
                r#"
[extension]
id = "absolute"
kind = "skill"
description = "absolute skill"

[skill]
prompt_bodies = ["{}"]
"#,
                absolute.display()
            ),
        )
        .expect("write manifest");

        let error = load_extension_runtime(&dir.path().join("extension.toml"))
            .expect_err("absolute prompt body should be rejected");

        assert!(error.to_string().contains("absolute prompt body"));
    }

    #[test]
    #[cfg(unix)]
    fn load_extension_runtime_rejects_symlinked_prompt_body_escape() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().expect("tempdir");
        let outside_dir = TempDir::new().expect("outside tempdir");
        let outside_file = outside_dir.path().join("outside.md");
        std::fs::write(&outside_file, "# Outside").expect("write outside file");
        symlink(&outside_file, dir.path().join("SKILL.md")).expect("create symlink");
        std::fs::write(
            dir.path().join("extension.toml"),
            r#"
[extension]
id = "symlink"
kind = "skill"
description = "symlink skill"

[skill]
prompt_bodies = ["SKILL.md"]
"#,
        )
        .expect("write manifest");

        let error = load_extension_runtime(&dir.path().join("extension.toml"))
            .expect_err("symlinked prompt body escape should be rejected");

        assert!(error.to_string().contains("outside its root"));
    }

    #[test]
    #[cfg(unix)]
    fn load_extension_runtime_rejects_symlinked_manifest() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().expect("tempdir");
        let outside_dir = TempDir::new().expect("outside tempdir");
        let outside_manifest = outside_dir.path().join("extension.toml");
        std::fs::write(
            &outside_manifest,
            r#"
[extension]
id = "external"
kind = "agent"

[agent]
prompts = ["outside"]
"#,
        )
        .expect("write outside manifest");
        symlink(&outside_manifest, dir.path().join("extension.toml")).expect("create symlink");

        let error = load_extension_runtime(&dir.path().join("extension.toml"))
            .expect_err("symlinked manifest should be rejected");

        assert!(error.to_string().contains("must not be a symlink"));
    }

    #[test]
    fn load_agent_extensions_from_workspace_is_disabled_by_default() {
        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let dir = TempDir::new().expect("tempdir");
        let agent_dir = agent_extensions_dir(dir.path()).join("planner");
        std::fs::create_dir_all(&agent_dir).expect("create agent dir");
        std::fs::write(agent_dir.join("AGENT.md"), "Follow the planner contract.")
            .expect("write agent body");
        std::fs::write(
            agent_dir.join("extension.toml"),
            r#"
[extension]
id = "planner"
kind = "agent"
description = "planner agent"

[agent]
role = "planner"
prompt_bodies = ["AGENT.md"]
"#,
        )
        .expect("write manifest");

        let loaded = load_agent_extensions_from_workspace(dir.path());
        assert!(loaded.is_empty());
    }

    #[test]
    fn merge_mcp_config_ignores_workspace_extensions_by_default() {
        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let dir = TempDir::new().expect("tempdir");
        let mcp_dir = mcp_extensions_dir(dir.path()).join("filesystem");
        std::fs::create_dir_all(&mcp_dir).expect("create mcp dir");
        std::fs::write(
            mcp_dir.join("extension.toml"),
            r#"
[extension]
id = "filesystem"
kind = "mcp"
description = "filesystem mcp"

[mcp]
transport = "stdio"
command = "npx"
"#,
        )
        .expect("write manifest");

        let config = McpConfig {
            enabled: true,
            import_json: None,
            servers: Vec::new(),
        };

        let merged = merge_mcp_config_with_workspace_extensions(&config, dir.path());
        assert!(merged.servers.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn workspace_loader_rejects_symlinked_manifest_files() {
        use std::os::unix::fs::symlink;

        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let _guard = EnvVarGuard::set(WORKSPACE_AGENT_EXTENSIONS_ENV, "1");

        let dir = TempDir::new().expect("tempdir");
        let agent_dir = agent_extensions_dir(dir.path()).join("planner");
        let outside_dir = TempDir::new().expect("outside tempdir");
        std::fs::create_dir_all(&agent_dir).expect("create agent dir");
        std::fs::write(
            outside_dir.path().join("extension.toml"),
            r#"
[extension]
id = "planner"
kind = "agent"

[agent]
prompts = ["external manifest"]
"#,
        )
        .expect("write outside manifest");
        symlink(
            outside_dir.path().join("extension.toml"),
            agent_dir.join("extension.toml"),
        )
        .expect("create manifest symlink");

        let loaded = load_agent_extensions_from_workspace(dir.path());
        assert!(loaded.is_empty());
    }
}
