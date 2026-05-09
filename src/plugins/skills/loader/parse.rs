//! Skill manifest parsing: loads `extension.toml` and open-skills
//! markdown files into `Skill` structs.

use std::io::{BufRead, BufReader, Cursor};
use std::path::Path;

use super::super::{Skill, SkillMetadata};
use crate::plugins::extensions::{
    ExtensionKind, ExtensionManifest, ExtensionManifestSpec, ExtensionRuntimeSpec,
    load_extension_manifest, load_extension_runtime, resolve_extension_body_path,
};
use anyhow::Result;

/// Parse an `extension.toml` manifest into a `Skill`.
///
/// # Errors
///
/// Returns an error when the file cannot be read, parsed, or when the
/// extension is not a skill manifest.
pub(super) fn load_extension_skill(path: &Path, enforce_requirements: bool) -> Result<Skill> {
    let runtime = load_extension_runtime(path)?;
    if runtime.manifest.extension.kind != ExtensionKind::Skill {
        anyhow::bail!(
            "extension manifest '{}' is '{}' not 'skill'",
            path.display(),
            manifest_kind_label(runtime.manifest.extension.kind)
        );
    }

    if enforce_requirements {
        enforce_declared_requirements(
            &runtime.manifest.requirements.commands,
            &runtime.manifest.requirements.env,
            path,
        )?;
    }

    Ok(skill_from_extension_runtime(runtime))
}

/// Parse an `extension.toml` manifest into metadata without loading prompt
/// bodies.
///
/// # Errors
///
/// Returns an error when the file cannot be read, parsed, when the extension
/// is not a skill manifest, or when required prompt bodies are missing.
pub(super) fn load_extension_skill_metadata(
    path: &Path,
    enforce_requirements: bool,
) -> Result<SkillMetadata> {
    let manifest = load_extension_manifest(path)?;
    if manifest.manifest.extension.kind != ExtensionKind::Skill {
        anyhow::bail!(
            "extension manifest '{}' is '{}' not 'skill'",
            path.display(),
            manifest_kind_label(manifest.manifest.extension.kind)
        );
    }

    if enforce_requirements {
        enforce_declared_requirements(
            &manifest.manifest.requirements.commands,
            &manifest.manifest.requirements.env,
            path,
        )?;
    }

    validate_prompt_body_paths(&manifest.manifest, &manifest.manifest_path)?;

    Ok(skill_metadata_from_extension_manifest(manifest))
}

fn enforce_declared_requirements(commands: &[String], env: &[String], path: &Path) -> Result<()> {
    for command in commands {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !is_command_available(trimmed) {
            anyhow::bail!(
                "skill '{}' skipped: required command '{}' not found in PATH",
                path.display(),
                trimmed
            );
        }
    }

    for env_key in env {
        let trimmed = env_key.trim();
        if trimmed.is_empty() {
            continue;
        }
        if std::env::var(trimmed).map_or(true, |value| value.trim().is_empty()) {
            anyhow::bail!(
                "skill '{}' skipped: required env var '{}' is not set",
                path.display(),
                trimmed
            );
        }
    }

    Ok(())
}

fn skill_from_extension_runtime(runtime: ExtensionRuntimeSpec) -> Skill {
    let ExtensionRuntimeSpec {
        manifest_path,
        manifest,
        bodies,
    } = runtime;
    let mut prompts = manifest
        .skill
        .as_ref()
        .map_or_else(Vec::new, |spec| spec.prompts.clone());
    if let Some(declarations) = render_extension_declarations(&manifest)
        && !declarations.is_empty()
    {
        prompts.push(declarations);
    }
    prompts.extend(bodies.into_iter().map(|body| body.content));

    Skill {
        name: manifest.extension.id,
        description: manifest.extension.description,
        version: manifest.extension.version,
        author: manifest.extension.author,
        tags: manifest.extension.tags,
        tools: manifest.skill.map_or_else(Vec::new, |spec| spec.tools),
        prompts,
        location: Some(manifest_path),
    }
}

fn skill_metadata_from_extension_manifest(manifest: ExtensionManifestSpec) -> SkillMetadata {
    let ExtensionManifestSpec {
        manifest_path,
        manifest,
    } = manifest;

    SkillMetadata {
        name: manifest.extension.id,
        description: manifest.extension.description,
        version: manifest.extension.version,
        author: manifest.extension.author,
        tags: manifest.extension.tags,
        tools: manifest.skill.map_or_else(Vec::new, |spec| spec.tools),
        location: Some(manifest_path),
    }
}

fn render_extension_declarations(
    manifest: &crate::plugins::extensions::ExtensionManifest,
) -> Option<String> {
    let capability_names = manifest
        .extension
        .capabilities
        .iter()
        .map(|capability| capability.as_str().trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let permission_names = manifest
        .extension
        .permissions
        .iter()
        .map(|permission| permission.as_str().trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();

    if capability_names.is_empty() && permission_names.is_empty() {
        return None;
    }

    let mut block = String::from("[Extension Contract]\n");
    if !capability_names.is_empty() {
        block.push_str("Capabilities:\n");
        for capability in capability_names {
            block.push_str("- ");
            block.push_str(capability);
            block.push('\n');
        }
    }
    if !permission_names.is_empty() {
        block.push_str("Permissions:\n");
        for permission in permission_names {
            block.push_str("- ");
            block.push_str(permission);
            block.push('\n');
        }
    }

    Some(block)
}

fn manifest_kind_label(kind: ExtensionKind) -> &'static str {
    match kind {
        ExtensionKind::Skill => "skill",
        ExtensionKind::Agent => "agent",
        ExtensionKind::Hook => "hook",
        ExtensionKind::Mcp => "mcp",
    }
}

fn is_command_available(command: &str) -> bool {
    let candidate = Path::new(command);
    if candidate.components().count() > 1 {
        return candidate.is_file();
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    for dir in std::env::split_paths(&path_var) {
        let command_path = dir.join(command);
        if command_path.is_file() {
            return true;
        }

        if cfg!(windows)
            && let Some(pathext) = std::env::var_os("PATHEXT")
        {
            for ext in pathext.to_string_lossy().split(';') {
                let ext = ext.trim();
                if ext.is_empty() {
                    continue;
                }
                let normalized_ext = if ext.starts_with('.') {
                    ext.to_string()
                } else {
                    format!(".{ext}")
                };
                if dir.join(format!("{command}{normalized_ext}")).is_file() {
                    return true;
                }
            }
        }
    }

    false
}

/// Load a skill from a community open-skills markdown file.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub(super) fn load_open_skill_md(path: &Path) -> Result<Skill> {
    let content = std::fs::read_to_string(path)?;
    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("open-skill")
        .to_string();

    Ok(Skill {
        name,
        description: extract_description(&content),
        version: "open-skills".to_string(),
        author: Some("besoeasy/open-skills".to_string()),
        tags: vec!["open-skills".to_string()],
        tools: Vec::new(),
        prompts: vec![content],
        location: Some(path.to_path_buf()),
    })
}

/// Load metadata from a community open-skills markdown file without reading
/// the entire document into memory.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub(super) fn load_open_skill_md_metadata(path: &Path) -> Result<SkillMetadata> {
    let file = std::fs::File::open(path)?;
    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("open-skill")
        .to_string();

    Ok(SkillMetadata {
        name,
        description: extract_description_from_reader(BufReader::new(file))?,
        version: "open-skills".to_string(),
        author: Some("besoeasy/open-skills".to_string()),
        tags: vec!["open-skills".to_string()],
        tools: Vec::new(),
        location: Some(path.to_path_buf()),
    })
}

fn extract_description(content: &str) -> String {
    extract_description_from_reader(Cursor::new(content.as_bytes()))
        .unwrap_or_else(|_| "No description".to_string())
}

fn extract_description_from_reader<R: BufRead>(reader: R) -> Result<String> {
    for line in reader.lines() {
        let line = line?;
        if !line.starts_with('#') && !line.trim().is_empty() {
            return Ok(line.trim().to_string());
        }
    }

    Ok("No description".to_string())
}

fn validate_prompt_body_paths(manifest: &ExtensionManifest, manifest_path: &Path) -> Result<()> {
    for relative_path in manifest.prompt_body_paths() {
        let _ = resolve_extension_body_path(manifest_path, &relative_path)?;
    }

    Ok(())
}

/// Returns `true` if the name is safe for use as a skill directory
/// name (no empty, no `..`, no slashes).
pub(super) fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && !name.contains('/') && !name.contains('\\')
}
