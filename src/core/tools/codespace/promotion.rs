//! Codespace project promotion — generates a local skill manifest from a project.
//!
//! # What it does
//!
//! `promote_project` writes two files to `<workspace>/skills/<skill-id>/`:
//!
//! * `extension.toml` — TOML manifest with the extension `id`, `kind`,
//!   `description`, `version`, and `tags` (always includes `"codespace"` and
//!   the project language).
//! * `SKILL.md` — Markdown skill body rendered from the project name,
//!   description, language, and a note that the skill was promoted from a
//!   codespace project.
//!
//! # Skill ID sanitization
//!
//! Raw tool names are normalized to lowercase kebab-case by
//! `sanitize_skill_id`: alphanumeric characters are kept, runs of
//! `[-_ .]` are collapsed to a single `-`, and leading/trailing dashes are
//! stripped. Purely symbolic names (no alphanumeric content) are rejected.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tokio::fs;

use super::types::CodespaceProject;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromotionResult {
    pub output_dir: PathBuf,
    pub summary: String,
}

#[derive(Debug, Serialize)]
struct ExtensionManifest<'a> {
    extension: ExtensionSection<'a>,
    skill: SkillSection<'a>,
}

#[derive(Debug, Serialize)]
struct ExtensionSection<'a> {
    id: &'a str,
    kind: &'a str,
    description: &'a str,
    version: &'a str,
    tags: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
struct SkillSection<'a> {
    prompt_bodies: Vec<&'a str>,
}

pub(crate) async fn promote_project(
    project: &CodespaceProject,
    tool_name: &str,
    tool_description: &str,
    skills_dir: &Path,
) -> Result<PromotionResult> {
    let skill_id = sanitize_skill_id(tool_name)?;
    let output_dir = skills_dir.join(&skill_id);
    fs::create_dir_all(&output_dir)
        .await
        .with_context(|| format!("Failed to create skill dir: {}", output_dir.display()))?;

    let manifest = ExtensionManifest {
        extension: ExtensionSection {
            id: &skill_id,
            kind: "skill",
            description: tool_description,
            version: "0.1.0",
            tags: vec!["codespace", project.language.as_str()],
        },
        skill: SkillSection {
            prompt_bodies: vec!["SKILL.md"],
        },
    };

    let manifest_content =
        toml::to_string_pretty(&manifest).context("Failed to serialize local skill manifest")?;
    let skill_content = render_skill_body(project, tool_name, tool_description);

    fs::write(output_dir.join("extension.toml"), manifest_content)
        .await
        .context("Failed to write extension.toml")?;
    fs::write(output_dir.join("SKILL.md"), skill_content)
        .await
        .context("Failed to write SKILL.md")?;

    Ok(PromotionResult {
        output_dir,
        summary: "Local skill manifest generated".to_string(),
    })
}

fn sanitize_skill_id(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("Skill name cannot be empty");
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut previous_was_dash = false;
    for ch in trimmed.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            previous_was_dash = false;
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_' | ' ' | '.') {
            if previous_was_dash {
                None
            } else {
                previous_was_dash = true;
                Some('-')
            }
        } else {
            None
        };

        if let Some(ch) = mapped {
            normalized.push(ch);
        }
    }

    let normalized = normalized.trim_matches('-').to_string();
    if normalized.is_empty() {
        bail!("Skill name must contain ASCII letters or numbers");
    }

    Ok(normalized)
}

fn render_skill_body(
    project: &CodespaceProject,
    tool_name: &str,
    tool_description: &str,
) -> String {
    format!(
        "# {tool_name}\n\n{tool_description}\n\n## Origin\n- Project: {}\n- Language: {}\n\n## Notes\n- Promoted from a local codespace project.\n- Review the project source before relying on the skill in other workspaces.\n",
        project.name, project.language
    )
}

#[cfg(test)]
mod tests {
    use super::sanitize_skill_id;

    #[test]
    fn sanitize_skill_id_normalizes_local_names() {
        assert_eq!(sanitize_skill_id("My Tool").unwrap(), "my-tool");
        assert_eq!(
            sanitize_skill_id("local_skill.v2").unwrap(),
            "local-skill-v2"
        );
    }

    #[test]
    fn sanitize_skill_id_rejects_empty_names() {
        assert!(sanitize_skill_id("   ").is_err());
        assert!(sanitize_skill_id("!!!").is_err());
    }
}
