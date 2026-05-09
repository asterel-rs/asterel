//! Security policy enforcement for cron job commands.
//!
//! Validates shell commands against the security policy allowlist,
//! checks for forbidden path arguments, and strips environment
//! variable prefixes before matching.

use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::security::SecurityPolicy;

fn is_env_assignment(word: &str) -> bool {
    word.contains('=')
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

fn forbidden_path_argument(security: &SecurityPolicy, command: &str) -> Option<String> {
    let mut normalized = command.to_string();
    for sep in ["&&", "||"] {
        normalized = normalized.replace(sep, "\x00");
    }
    for sep in ['\n', ';', '|'] {
        normalized = normalized.replace(sep, "\x00");
    }

    for segment in normalized.split('\x00') {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // Skip leading env assignments and executable token.
        let mut idx = 0;
        while idx < tokens.len() && is_env_assignment(tokens[idx]) {
            idx += 1;
        }
        if idx >= tokens.len() {
            continue;
        }
        idx += 1;

        for token in &tokens[idx..] {
            let candidate = strip_wrapping_quotes(token);
            if candidate.is_empty() || candidate.starts_with('-') || candidate.contains("://") {
                continue;
            }

            let looks_like_path = candidate.starts_with('/')
                || candidate.starts_with("./")
                || candidate.starts_with("../")
                || candidate.starts_with("~/")
                || candidate.contains('/');

            if looks_like_path && !security.is_path_allowed(candidate) {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

fn policy_denial(route_marker: &str, reason: impl Into<String>) -> String {
    format!("{route_marker}\n{}", reason.into())
}

pub(super) fn enforce_policy_invariants(
    security: &SecurityPolicy,
    command: &str,
    route_marker: &str,
) -> Result<(), String> {
    if let Some(path) = forbidden_path_argument(security, command) {
        return Err(policy_denial(
            route_marker,
            format!("{SECURITY_POLICY_BLOCK_PREFIX}forbidden path argument: {path}"),
        ));
    }

    if !security.is_command_allowed(command) {
        return Err(policy_denial(
            route_marker,
            format!("{SECURITY_POLICY_BLOCK_PREFIX}command not allowed: {command}"),
        ));
    }

    if let Err(policy_error) = security.consume_action_cost(0) {
        return Err(policy_denial(route_marker, policy_error));
    }

    Ok(())
}
