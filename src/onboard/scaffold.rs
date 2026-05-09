//! Workspace scaffolding for new installations.
//!
//! Creates the directory tree and renders initial template files
//! (system prompt, persona) into the workspace after onboarding.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Result;

use super::prompts::ProjectContext;
use crate::ui::style as ui;

/// Escape `{{` and `}}` sequences in user input to prevent template injection.
fn sanitize_template_input(input: &str) -> String {
    input.replace("{{", "").replace("}}", "")
}

fn render(template: &str, agent: &str, user: &str, tz: &str, comm_style: &str) -> String {
    let agent = sanitize_template_input(agent);
    let user = sanitize_template_input(user);
    let tz = sanitize_template_input(tz);
    let comm_style = sanitize_template_input(comm_style);
    template
        .replace("{{agent}}", &agent)
        .replace("{{user}}", &user)
        .replace("{{tz}}", &tz)
        .replace("{{comm_style}}", &comm_style)
}

/// # Errors
///
/// Returns an error when creating scaffold directories or writing template
/// files fails.
pub(crate) fn scaffold_workspace(workspace_dir: &Path, ctx: &ProjectContext) -> Result<()> {
    let agent = if ctx.agent_name.is_empty() {
        "Asterel"
    } else {
        &ctx.agent_name
    };
    let user = if ctx.user_name.is_empty() {
        "User"
    } else {
        &ctx.user_name
    };
    let tz = if ctx.timezone.is_empty() {
        "UTC"
    } else {
        &ctx.timezone
    };
    let comm_style = if ctx.communication_style.is_empty() {
        "Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing."
    } else {
        &ctx.communication_style
    };

    let r = |tpl: &str| render(tpl, agent, user, tz, comm_style);

    let files: Vec<(&str, String)> = vec![
        ("SOUL.md", r(include_str!("templates/SOUL.md"))),
        ("CHARACTER.md", r(include_str!("templates/CHARACTER.md"))),
        ("AGENTS.md", r(include_str!("templates/AGENTS.md"))),
        ("HEARTBEAT.md", r(include_str!("templates/HEARTBEAT.md"))),
        ("USER.md", r(include_str!("templates/USER.md"))),
        ("TOOLS.md", include_str!("templates/TOOLS.md").to_string()),
        ("BOOTSTRAP.md", r(include_str!("templates/BOOTSTRAP.md"))),
        ("MEMORY.md", include_str!("templates/MEMORY.md").to_string()),
    ];

    let subdirs = ["sessions", "memory", "state", "cron", "skills"];
    for dir in &subdirs {
        fs::create_dir_all(workspace_dir.join(dir))?;
    }

    let mut created = 0;
    let mut skipped = 0;

    for (filename, content) in &files {
        let path = workspace_dir.join(filename);
        // Use create_new() to atomically create-if-not-exists (no TOCTOU race)
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                created += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                skipped += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }

    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.scaffold.created",
            created = created,
            skipped = skipped,
            dirs = subdirs.len()
        )
    );

    println!();
    println!("  {}", ui::dim(t!("onboard.scaffold.layout_header")));
    println!("  {}", ui::dim(format!("  {}/", workspace_dir.display())));
    for dir in &subdirs {
        println!("  {}", ui::dim(format!("  ├── {dir}/")));
    }
    for (i, (filename, _)) in files.iter().enumerate() {
        let prefix = if i == files.len() - 1 {
            "└──"
        } else {
            "├──"
        };
        println!("  {}", ui::dim(format!("  {prefix} {filename}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn default_ctx() -> ProjectContext {
        ProjectContext {
            user_name: "TestUser".to_string(),
            timezone: "Asia/Tokyo".to_string(),
            agent_name: "TestAgent".to_string(),
            communication_style: "Be concise.".to_string(),
        }
    }

    #[test]
    fn scaffold_creates_files_and_dirs() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&dir).unwrap();
        scaffold_workspace(&dir, &default_ctx()).unwrap();

        // 8 template files
        for file in &[
            "CHARACTER.md",
            "AGENTS.md",
            "HEARTBEAT.md",
            "SOUL.md",
            "USER.md",
            "TOOLS.md",
            "BOOTSTRAP.md",
            "MEMORY.md",
        ] {
            assert!(dir.join(file).exists(), "{file} should exist");
        }

        // 5 subdirectories
        for sub in &["sessions", "memory", "state", "cron", "skills"] {
            assert!(dir.join(sub).is_dir(), "{sub}/ should exist");
        }
    }

    #[test]
    fn scaffold_renders_template_variables() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&dir).unwrap();
        scaffold_workspace(&dir, &default_ctx()).unwrap();

        let character = std::fs::read_to_string(dir.join("CHARACTER.md")).unwrap();
        assert!(
            character.contains("TestAgent"),
            "CHARACTER.md should contain agent_name"
        );
    }

    #[test]
    fn scaffold_skips_existing_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&dir).unwrap();

        let sentinel = "DO NOT OVERWRITE";
        std::fs::write(dir.join("CHARACTER.md"), sentinel).unwrap();

        scaffold_workspace(&dir, &default_ctx()).unwrap();

        let content = std::fs::read_to_string(dir.join("CHARACTER.md")).unwrap();
        assert_eq!(content, sentinel, "existing file should not be overwritten");
    }

    #[test]
    fn sanitize_template_input_removes_braces() {
        assert_eq!(sanitize_template_input("hello"), "hello");
        assert_eq!(sanitize_template_input("{{agent}}"), "agent");
        assert_eq!(sanitize_template_input("a{{b}}c"), "abc");
        assert_eq!(sanitize_template_input("no braces here"), "no braces here");
    }

    #[test]
    fn scaffold_sanitizes_template_injection() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&dir).unwrap();

        let malicious_ctx = ProjectContext {
            user_name: "{{agent}}".to_string(),
            timezone: "{{comm_style}}".to_string(),
            agent_name: "Good".to_string(),
            communication_style: "Normal".to_string(),
        };
        scaffold_workspace(&dir, &malicious_ctx).unwrap();

        let character = std::fs::read_to_string(dir.join("CHARACTER.md")).unwrap();
        // The injected {{agent}} in user_name should have been sanitized,
        // not interpreted as a template directive.
        assert!(
            !character.contains("{{agent}}"),
            "template directives should be stripped from user input"
        );
    }

    #[test]
    fn scaffold_defaults_empty_fields() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&dir).unwrap();

        let empty_ctx = ProjectContext::default();
        scaffold_workspace(&dir, &empty_ctx).unwrap();

        let character = std::fs::read_to_string(dir.join("CHARACTER.md")).unwrap();
        assert!(
            character.contains("Asterel"),
            "empty agent_name should default to Asterel"
        );
    }
}
