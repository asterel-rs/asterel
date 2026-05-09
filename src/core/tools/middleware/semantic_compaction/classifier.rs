use std::collections::VecDeque;

/// Classify a raw shell command into a stable semantic output kind.
#[must_use]
pub fn classify_shell_command_output_kind(command: &str) -> &'static str {
    let mut tokens = match shlex::split(command) {
        Some(tokens) if !tokens.is_empty() => VecDeque::from(tokens),
        _ => return "unknown",
    };

    strip_common_shell_wrappers(&mut tokens);
    if contains_control_operator(&tokens) {
        return "unknown";
    }

    match tokens.pop_front().as_deref() {
        Some("git") => classify_git_command(tokens),
        Some("cargo") => classify_cargo_command(tokens),
        Some("rg") => "shell.ripgrep",
        _ => "unknown",
    }
}

fn strip_common_shell_wrappers(tokens: &mut VecDeque<String>) {
    loop {
        let Some(token) = tokens.front().cloned() else {
            return;
        };

        let consumed = match token.as_str() {
            _ if looks_like_env_assignment(&token) => {
                tokens.pop_front();
                true
            }
            "env" => consume_env_wrapper(tokens),
            "sudo" => consume_sudo_wrapper(tokens),
            "command" | "builtin" | "exec" | "nohup" | "time" => {
                tokens.pop_front();
                true
            }
            _ => false,
        };

        if !consumed {
            return;
        }
    }
}

fn consume_env_wrapper(tokens: &mut VecDeque<String>) -> bool {
    let Some(token) = tokens.pop_front() else {
        return false;
    };
    if token != "env" {
        return false;
    }

    loop {
        let Some(next) = tokens.front().cloned() else {
            return true;
        };

        match next.as_str() {
            "-i" | "--ignore-environment" => {
                tokens.pop_front();
            }
            "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string" => {
                tokens.pop_front();
                tokens.pop_front();
            }
            _ if next.starts_with('-') => {
                tokens.pop_front();
            }
            _ if looks_like_env_assignment(&next) => {
                tokens.pop_front();
            }
            _ => return true,
        }
    }
}

fn consume_sudo_wrapper(tokens: &mut VecDeque<String>) -> bool {
    let Some(token) = tokens.pop_front() else {
        return false;
    };
    if token != "sudo" {
        return false;
    }

    loop {
        let Some(next) = tokens.front().cloned() else {
            return true;
        };

        match next.as_str() {
            "-u" | "--user" | "-g" | "--group" | "-h" | "--host" | "-D" | "--chdir" | "-p"
            | "--prompt" | "-C" | "--close-from" => {
                tokens.pop_front();
                tokens.pop_front();
            }
            _ if next.starts_with('-') => {
                tokens.pop_front();
            }
            _ => return true,
        }
    }
}

fn classify_git_command(mut tokens: VecDeque<String>) -> &'static str {
    while let Some(token) = tokens.pop_front() {
        match token.as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" => {
                tokens.pop_front();
            }
            "--paginate"
            | "--no-pager"
            | "--bare"
            | "--no-optional-locks"
            | "--literal-pathspecs"
            | "--no-literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs" => {}
            _ if token.starts_with('-') => {}
            "status" => return "shell.git_status",
            "diff" => return "shell.git_diff",
            _ => return "unknown",
        }
    }

    "unknown"
}

fn classify_cargo_command(mut tokens: VecDeque<String>) -> &'static str {
    while let Some(token) = tokens.pop_front() {
        match token.as_str() {
            _ if token.starts_with('+') => {}
            "-Z" | "--config" | "--manifest-path" | "--color" => {
                tokens.pop_front();
            }
            _ if token.starts_with('-') => {}
            "test" => return "shell.cargo_test",
            "clippy" => return "shell.cargo_clippy",
            _ => return "unknown",
        }
    }

    "unknown"
}

fn contains_control_operator(tokens: &VecDeque<String>) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "&&" | "||" | "|" | "|&" | ";" | "&" | ">" | ">>" | "<" | "<<" | "2>" | "&>"
        )
    })
}

fn looks_like_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::classify_shell_command_output_kind;

    #[test]
    fn classifier_handles_common_wrappers() {
        assert_eq!(
            classify_shell_command_output_kind("env FOO=bar git -C repo status --short"),
            "shell.git_status"
        );
        assert_eq!(
            classify_shell_command_output_kind("FOO=bar cargo test"),
            "shell.cargo_test"
        );
        assert_eq!(
            classify_shell_command_output_kind("sudo cargo clippy --workspace"),
            "shell.cargo_clippy"
        );
        assert_eq!(
            classify_shell_command_output_kind("command rg foo src/"),
            "shell.ripgrep"
        );
    }

    #[test]
    fn classifier_rejects_control_operator_sequences() {
        assert_eq!(
            classify_shell_command_output_kind("git status && cargo test"),
            "unknown"
        );
    }

    #[test]
    fn classifier_handles_cargo_global_flag_values() {
        assert_eq!(
            classify_shell_command_output_kind("cargo --color always test"),
            "shell.cargo_test"
        );
    }
}
