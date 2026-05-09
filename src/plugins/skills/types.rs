//! Type definitions for skills: `Skill` (manifest, tools, prompts),
//! `SkillMetadata` (catalog/ranking metadata), and `SkillTool`
//! (shell/HTTP/script tool descriptors).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A skill is a user-defined or community-built capability.
/// Skills live in `~/.asterel/workspace/skills/<name>/SKILL.md`
/// and can include tool definitions, prompts, and automation scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// Unique skill name (directory name or manifest `name` field).
    pub name: String,
    /// Human-readable description of what the skill does.
    pub description: String,
    /// Semver version string (e.g. "0.1.0").
    pub version: String,
    /// Skill author, if declared in the manifest.
    #[serde(default)]
    pub author: Option<String>,
    /// Freeform tags for categorization and discovery.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Tool definitions exposed by this skill.
    #[serde(default)]
    pub tools: Vec<SkillTool>,
    /// Raw prompt text injected into the agent system prompt.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Filesystem path where this skill was loaded from.
    #[serde(skip)]
    pub location: Option<PathBuf>,
}

/// Metadata-only view of a skill used for prompt catalogs and relevance
/// ranking without loading prompt body contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Unique skill name (directory name or manifest `name` field).
    pub name: String,
    /// Human-readable description of what the skill does.
    pub description: String,
    /// Semver version string (e.g. "0.1.0").
    pub version: String,
    /// Skill author, if declared in the manifest.
    #[serde(default)]
    pub author: Option<String>,
    /// Freeform tags for categorization and discovery.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Tool definitions exposed by this skill.
    #[serde(default)]
    pub tools: Vec<SkillTool>,
    /// Filesystem path where this skill was loaded from.
    #[serde(skip)]
    pub location: Option<PathBuf>,
}

/// Classification of a skill's capability boundary (WP-SKILL).
///
/// Instruction-only skills shape reasoning via prompts but cannot act.
/// Executable skills can run tools and affect the outside world, requiring
/// different trust, review, and promotion paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCapabilityClass {
    /// Prompt/instruction only — shapes reasoning, cannot act externally.
    InstructionOnly,
    /// Has executable tools — can affect the outside world.
    Executable,
}

impl Skill {
    /// Classify whether this skill is instruction-only or executable.
    #[must_use]
    pub fn capability_class(&self) -> SkillCapabilityClass {
        if self.tools.is_empty() {
            SkillCapabilityClass::InstructionOnly
        } else {
            SkillCapabilityClass::Executable
        }
    }
}

/// A tool defined by a skill (shell command, HTTP call, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillTool {
    /// Tool name used for invocation.
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// "shell", "http", "script"
    pub kind: String,
    /// The command/URL/script to execute
    pub command: String,
    /// Named arguments passed to the tool at invocation time.
    #[serde(default)]
    pub args: HashMap<String, String>,
}

/// Shared prompt-facing metadata for loaded skills.
pub trait SkillCatalogEntry {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn tags(&self) -> &[String];
    fn tools(&self) -> &[SkillTool];
    fn location(&self) -> Option<&Path>;
}

impl SkillCatalogEntry for Skill {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn tools(&self) -> &[SkillTool] {
        &self.tools
    }

    fn location(&self) -> Option<&Path> {
        self.location.as_deref()
    }
}

impl SkillCatalogEntry for SkillMetadata {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn tools(&self) -> &[SkillTool] {
        &self.tools
    }

    fn location(&self) -> Option<&Path> {
        self.location.as_deref()
    }
}
