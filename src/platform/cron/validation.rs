//! Cron command validation helpers shared across repository, scheduler,
//! CLI, and transport surfaces.

use std::fmt;

const LEGACY_PLANNER_CRON_COMMAND_MESSAGE: &str =
    "legacy planner cron commands are no longer accepted on the primary runtime";
const LEGACY_PLANNER_CRON_COMMAND_CODE: &str = "legacy_plan_command_forbidden";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronCommandValidationError {
    LegacyPlannerCommand,
}

impl CronCommandValidationError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::LegacyPlannerCommand => LEGACY_PLANNER_CRON_COMMAND_CODE,
        }
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::LegacyPlannerCommand => LEGACY_PLANNER_CRON_COMMAND_MESSAGE,
        }
    }
}

impl fmt::Display for CronCommandValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CronCommandValidationError {}

#[must_use]
pub fn is_legacy_planner_command(command: &str) -> bool {
    first_executable_token(command)
        .is_some_and(|token| token == "plan" || token.starts_with("plan:"))
}

fn first_executable_token(command: &str) -> Option<&str> {
    for token in command.split_whitespace() {
        if is_shell_env_assignment(token) {
            continue;
        }
        return Some(token);
    }

    None
}

fn is_shell_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// # Errors
/// Returns an error if the command shape belongs to the deleted planner cron path.
pub fn validate_main_runtime_cron_command(command: &str) -> Result<(), CronCommandValidationError> {
    if is_legacy_planner_command(command) {
        return Err(CronCommandValidationError::LegacyPlannerCommand);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CronCommandValidationError, is_legacy_planner_command, validate_main_runtime_cron_command,
    };

    #[test]
    fn detects_legacy_planner_prefix_after_whitespace() {
        assert!(is_legacy_planner_command("  plan:{\"id\":\"legacy\"}"));
        assert!(is_legacy_planner_command("plan -m \"legacy\""));
        assert!(is_legacy_planner_command("FOO=1 plan -m \"legacy\""));
        assert!(!is_legacy_planner_command("echo ok"));
        assert!(!is_legacy_planner_command("echo \"plan -m legacy\""));
        assert!(!is_legacy_planner_command("agent -m \"make a plan\""));
        assert!(!is_legacy_planner_command("planning-tool run"));
    }

    #[test]
    fn validator_rejects_legacy_planner_command() {
        let error = validate_main_runtime_cron_command("plan:{\"id\":\"legacy\"}")
            .expect_err("legacy planner commands must be rejected");
        assert_eq!(error, CronCommandValidationError::LegacyPlannerCommand);
        assert_eq!(error.code(), "legacy_plan_command_forbidden");

        let error = validate_main_runtime_cron_command("plan -m \"legacy\"")
            .expect_err("legacy planner executable forms must be rejected");
        assert_eq!(error, CronCommandValidationError::LegacyPlannerCommand);
    }
}
