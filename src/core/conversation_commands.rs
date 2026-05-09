//! Conversation-level interactive commands (`/think`, `/new`, `/help`, etc.).
//!
//! These are commands parsed from user text input during an interactive session,
//! shared across CLI and channel adapters. Extracted into `core/` so that
//! `core::agent` and `transport::channels` can depend on them without pulling
//! in presentation-layer (`cli/`) code.

use serde::{Deserialize, Serialize};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Status,
    New,
    Compact,
    Think { level: Option<String> },
    Verbose,
    Usage,
    Help,
}

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub text: String,
    pub ephemeral: bool,
}

impl CommandResult {
    pub fn visible(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ephemeral: false,
        }
    }

    pub fn ephemeral(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ephemeral: true,
        }
    }
}

// ── Parser ─────────────────────────────────────────────────────────

pub fn parse_command(input: &str) -> Option<Command> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let cmd = parts.next()?.to_lowercase();
    let args = parts.next().unwrap_or("").trim();

    match cmd.as_str() {
        "/status" => Some(Command::Status),
        "/new" | "/reset" => Some(Command::New),
        "/compact" => Some(Command::Compact),
        "/think" => Some(Command::Think {
            level: if args.is_empty() {
                None
            } else {
                Some(args.to_string())
            },
        }),
        "/verbose" => Some(Command::Verbose),
        "/usage" => Some(Command::Usage),
        "/help" | "/?" => Some(Command::Help),
        _ => None,
    }
}

// ── Handlers ───────────────────────────────────────────────────────

#[must_use]
pub fn handle_command(command: &Command) -> CommandResult {
    match command {
        Command::Status => handle_status(),
        Command::New => handle_new(),
        Command::Compact => handle_compact(),
        Command::Think { level } => handle_think(level.as_deref()),
        Command::Verbose => handle_verbose(),
        Command::Usage => handle_usage(),
        Command::Help => handle_help(),
    }
}

fn handle_status() -> CommandResult {
    CommandResult::visible("✓ Asterel is running.")
}

fn handle_new() -> CommandResult {
    CommandResult::visible("Session reset. Starting fresh.")
}

fn handle_compact() -> CommandResult {
    CommandResult::visible("Session compacted.")
}

fn handle_think(level: Option<&str>) -> CommandResult {
    match level {
        Some(l) => CommandResult::ephemeral(format!("Thinking level set to: {l}")),
        None => CommandResult::ephemeral("Thinking level toggled."),
    }
}

fn handle_verbose() -> CommandResult {
    CommandResult::ephemeral("Verbose mode toggled.")
}

fn handle_usage() -> CommandResult {
    CommandResult::visible("Usage tracking not yet configured.")
}

fn handle_help() -> CommandResult {
    CommandResult::visible(
        "/status  — Show current status\n\
         /new     — Start a new session\n\
         /compact — Summarize session history\n\
         /think   — Toggle thinking mode (e.g. /think high, /think show, /think hide)\n\
         /verbose — Toggle verbose output\n\
         /usage   — Show token usage statistics\n\
         /help    — Show this help message",
    )
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_command() {
        assert_eq!(parse_command("/status"), Some(Command::Status));
    }

    #[test]
    fn status_case_insensitive() {
        assert_eq!(parse_command("/STATUS"), Some(Command::Status));
    }

    #[test]
    fn help_command() {
        assert_eq!(parse_command("/help"), Some(Command::Help));
    }

    #[test]
    fn help_question_mark() {
        assert_eq!(parse_command("/?"), Some(Command::Help));
    }

    #[test]
    fn new_command() {
        assert_eq!(parse_command("/new"), Some(Command::New));
    }

    #[test]
    fn reset_alias() {
        assert_eq!(parse_command("/reset"), Some(Command::New));
    }

    #[test]
    fn think_with_level() {
        assert_eq!(
            parse_command("/think high"),
            Some(Command::Think {
                level: Some("high".to_string())
            })
        );
    }

    #[test]
    fn think_without_level() {
        assert_eq!(
            parse_command("/think"),
            Some(Command::Think { level: None })
        );
    }

    #[test]
    fn compact_command() {
        assert_eq!(parse_command("/compact"), Some(Command::Compact));
    }

    #[test]
    fn verbose_command() {
        assert_eq!(parse_command("/verbose"), Some(Command::Verbose));
    }

    #[test]
    fn usage_command() {
        assert_eq!(parse_command("/usage"), Some(Command::Usage));
    }

    #[test]
    fn plain_text_returns_none() {
        assert_eq!(parse_command("hello"), None);
    }

    #[test]
    fn unknown_command_returns_none() {
        assert_eq!(parse_command("/unknown"), None);
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(parse_command(""), None);
    }

    #[test]
    fn status_ignores_extra_args() {
        assert_eq!(parse_command("/status extra args"), Some(Command::Status));
    }

    #[test]
    fn whitespace_only_returns_none() {
        assert_eq!(parse_command("   "), None);
    }

    #[test]
    fn leading_whitespace_accepted() {
        assert_eq!(parse_command("  /status"), Some(Command::Status));
    }

    #[test]
    fn handle_command_produces_visible_for_status() {
        let result = handle_command(&Command::Status);
        assert!(!result.ephemeral);
        assert!(result.text.contains("running"));
    }

    #[test]
    fn handle_command_produces_visible_for_help() {
        let result = handle_command(&Command::Help);
        assert!(!result.ephemeral);
        assert!(result.text.contains("/status"));
    }

    #[test]
    fn handle_command_produces_ephemeral_for_think() {
        let result = handle_command(&Command::Think { level: None });
        assert!(result.ephemeral);
    }
}
