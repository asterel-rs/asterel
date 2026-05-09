//! Workspace/bootstrap prompt sections and host environment helpers.
use std::path::Path;

use crate::security::external_content::{
    ExternalAction, decide_action, detect_injection, sanitize_marker_collision,
};

/// Maximum characters per injected workspace file.
pub(crate) const BOOTSTRAP_MAX_CHARS: usize = 20_000;

/// Get the system hostname, with fallback to "unknown".
pub(super) fn get_hostname() -> String {
    #[cfg(unix)]
    {
        if let Ok(hostname) = std::env::var("HOSTNAME") {
            return hostname;
        }
        if let Ok(content) = std::fs::read_to_string("/etc/hostname") {
            return content.trim().to_string();
        }
        "unknown".to_string()
    }

    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

pub(super) fn inject_workspace_file(prompt: &mut String, workspace_dir: &Path, filename: &str) {
    use std::fmt::Write;

    let path = workspace_dir.join(filename);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.len() > BOOTSTRAP_MAX_CHARS
                && trimmed.chars().count() > BOOTSTRAP_MAX_CHARS
            {
                trimmed
                    .char_indices()
                    .nth(BOOTSTRAP_MAX_CHARS)
                    .map_or(trimmed, |(idx, _)| &trimmed[..idx])
            } else {
                trimmed
            };
            let action = decide_action(&detect_injection(truncated));

            match action {
                ExternalAction::Block => {
                    let _ = writeln!(
                        prompt,
                        "[bootstrap content blocked by external-content policy: {filename}]\n"
                    );
                }
                ExternalAction::Sanitize | ExternalAction::Allow => {
                    let to_inject = if action == ExternalAction::Sanitize {
                        sanitize_marker_collision(truncated)
                    } else {
                        truncated.to_string()
                    };

                    if truncated.len() < trimmed.len() {
                        prompt.push_str(&to_inject);
                        let _ = writeln!(
                            prompt,
                            "\n\n[... truncated at {BOOTSTRAP_MAX_CHARS} chars — use `read` for full file]\n"
                        );
                    } else {
                        prompt.push_str(&to_inject);
                        prompt.push_str("\n\n");
                    }
                }
            }
        }
        Err(_) => {
            let _ = writeln!(prompt, "### {filename}\n\n[File not found: {filename}]\n");
        }
    }
}
