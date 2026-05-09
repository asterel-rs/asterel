//! CLI command handlers for skill management (install, remove,
//! list, init, sync).

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::parse::is_valid_skill_name;
use crate::config::SkillsRuntimeConfig;
#[cfg(windows)]
use crate::security::{ProcessSpawnClass, enforce_spawn_policy};
use crate::security::{RootBoundPathKind, SecurityPolicy, canonicalize_path_within_root};
use crate::ui::style as ui;

/// Recursively copy a directory into the canonical workspace skills area.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Handle the `skills` CLI command
///
/// # Errors
///
/// Returns an error when command handling requires filesystem or process
/// operations that fail, or when command validation fails.
pub fn handle_command(
    command: crate::SkillCommands,
    workspace_dir: &Path,
    security: &SecurityPolicy,
    skills_config: &SkillsRuntimeConfig,
) -> Result<()> {
    match command {
        crate::SkillCommands::List => {
            handle_list(workspace_dir, security, skills_config);
            Ok(())
        }
        crate::SkillCommands::Install { source } => {
            handle_install(&source, workspace_dir, security)
        }
        crate::SkillCommands::Remove { name } => handle_remove(&name, workspace_dir),
    }
}

fn handle_list(
    workspace_dir: &Path,
    security: &SecurityPolicy,
    skills_config: &SkillsRuntimeConfig,
) {
    let skills = super::load_skills_with_policy_and_config(workspace_dir, security, skills_config);
    println!();
    println!("  {}", ui::section("Skills"));
    if skills.is_empty() {
        println!();
        println!("{}", ui::note_line("No skills installed."));
        println!("{}", ui::note_line("Create one:"));
        println!(
            "{}",
            ui::command_line("mkdir -p ~/.asterel/workspace/skills/my-skill")
        );
        println!(
            "{}",
            ui::command_line(
                "printf '[extension]\\nid = \"my-skill\"\\nkind = \"skill\"\\ndescription = \"What this skill does\"\\n\\n[skill]\\nprompt_bodies = [\"SKILL.md\"]\\n' > ~/.asterel/workspace/skills/my-skill/extension.toml"
            )
        );
        println!(
            "{}",
            ui::command_line("echo '# My Skill' > ~/.asterel/workspace/skills/my-skill/SKILL.md")
        );
        println!();
        println!("{}", ui::note_line("Or install a reviewed local skill:"));
        println!(
            "{}",
            ui::command_line("asterel skills install <local-skill-dir>")
        );
    } else {
        println!("{}", ui::field_line("Installed", skills.len()));
        println!();
        for skill in &skills {
            println!(
                "  {} {} {}",
                ui::ok_badge("installed"),
                ui::header(&skill.name),
                ui::dim(format!("v{}", skill.version))
            );
            println!("{}", ui::field_line("Description", &skill.description));
            if !skill.tools.is_empty() {
                let mut tool_names = String::new();
                for t in &skill.tools {
                    if !tool_names.is_empty() {
                        tool_names.push_str(", ");
                    }
                    tool_names.push_str(t.name.as_str());
                }
                println!("{}", ui::field_line("Tools", tool_names));
            }
            if !skill.tags.is_empty() {
                println!("{}", ui::field_line("Tags", skill.tags.join(", ")));
            }
            println!();
        }
    }
    println!();
}

fn handle_install(source: &str, workspace_dir: &Path, security: &SecurityPolicy) -> Result<()> {
    println!();
    println!("  {}", ui::section("Install Skill"));
    println!("{}", ui::field_line("Source", source));

    let skills_path = super::skills_dir(workspace_dir);
    std::fs::create_dir_all(&skills_path)?;

    if source.starts_with("https://") || source.starts_with("http://") {
        anyhow::bail!(
            "Remote skill install is disabled. Review the skill locally inside the workspace and install it from a local directory."
        );
    }

    install_from_local(source, workspace_dir, &skills_path, security)
}

fn install_from_local(
    source: &str,
    workspace_dir: &Path,
    skills_path: &Path,
    security: &SecurityPolicy,
) -> Result<()> {
    let src = PathBuf::from(source);
    if !src.exists() {
        anyhow::bail!("Source path does not exist: {source}");
    }
    let canonical_src =
        canonicalize_path_within_root(&src, workspace_dir, RootBoundPathKind::Directory).map_err(
            |error| {
                if error.to_string().contains("outside allowed root") {
                    anyhow::anyhow!("Source path escapes workspace: {}", src.display())
                } else {
                    error
                }
            },
        )?;

    let skill_name = canonical_src
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid source path: {source}"))?;
    if !is_valid_skill_name(skill_name) {
        anyhow::bail!("Invalid skill name from source path: {skill_name}");
    }

    let dest = skills_path.join(skill_name);
    if dest.exists() {
        anyhow::bail!("Skill destination already exists: {}", dest.display());
    }

    copy_skill_install(&canonical_src, &dest, security)
}

fn copy_skill_install(canonical_src: &Path, dest: &Path, _security: &SecurityPolicy) -> Result<()> {
    #[cfg(windows)]
    enforce_spawn_policy(
        _security,
        "cmd",
        "plugins_skills_install_windows_junction",
        ProcessSpawnClass::OperatorPlane,
    )?;

    copy_dir_recursive(canonical_src, dest)?;
    println!("{}", ui::field_line("Result", ui::ok_badge("copied")));
    println!("{}", ui::field_line("Location", dest.display()));
    Ok(())
}

fn handle_remove(name: &str, workspace_dir: &Path) -> Result<()> {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        anyhow::bail!("Invalid skill name: {name}");
    }

    let skill_path = resolve_skill_remove_path(name, workspace_dir)?;

    let meta = std::fs::symlink_metadata(&skill_path)?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(&skill_path)?;
    } else {
        std::fs::remove_dir_all(&skill_path)?;
    }
    println!();
    println!("  {}", ui::section("Remove Skill"));
    println!("{}", ui::field_line("Skill", name));
    println!("{}", ui::field_line("Result", ui::ok_badge("removed")));
    Ok(())
}

fn resolve_skill_remove_path(name: &str, workspace_dir: &Path) -> Result<PathBuf> {
    let skills_path = super::skills_dir(workspace_dir);
    let direct_path = skills_path.join(name);
    if std::fs::symlink_metadata(&direct_path).is_ok() {
        return Ok(direct_path);
    }

    let metadata = super::directory::load_workspace_skill_metadata(workspace_dir, true);
    let matches = metadata
        .into_iter()
        .filter(|skill| skill.name == name)
        .filter_map(|skill| {
            skill
                .location
                .and_then(|path| path.parent().map(Path::to_path_buf))
        })
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => anyhow::bail!("Skill not found: {name}"),
        [path] => Ok(path.clone()),
        _ => anyhow::bail!("Skill name is ambiguous: {name}"),
    }
}
