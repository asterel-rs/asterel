//! Shell command validation against the security policy.
//!
//! Checks allowed/blocked commands, validates arguments for
//! dangerous patterns (e.g., git config code execution), and
//! enforces workspace-only restrictions.

use std::path::{Path, PathBuf};

use super::SecurityPolicy;
use super::types::AutonomyLevel;

/// Skip leading environment variable assignments (e.g. `FOO=bar cmd args`).
/// Returns the remainder starting at the first non-assignment word.
fn skip_env_assignments(s: &str) -> &str {
    let mut rest = s;
    loop {
        let Some(word) = rest.split_whitespace().next() else {
            return rest;
        };
        if word.contains('=')
            && word
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            rest = rest[word.len()..].trim_start();
        } else {
            return rest;
        }
    }
}

fn has_leading_env_assignments(s: &str) -> bool {
    skip_env_assignments(s) != s.trim_start()
}

/// Git-specific config keys that enable arbitrary code execution when passed
/// via `git -c <key>=<val>` or `git clone --config <key>=<val>`.
const GIT_BLOCKED_CONFIG_KEYS: &[&str] = &[
    "core.sshcommand",
    "core.fsmonitor",
    "core.pager",
    "core.editor",
    "core.askpass",
    "core.hookspath", // redirects git hooks to attacker-controlled scripts
    "core.gitproxy",  // arbitrary proxy binary execution
    "credential.",
    "diff.external",
    "merge.tool",
    "filter.",
    "http.proxy",  // traffic redirection via proxy
    "https.proxy", // traffic redirection via proxy
    "protocol.",   // protocol.*.allow can enable arbitrary transports
    "url.",        // covers url.*.insteadOf, url.*.pushInsteadOf
];

fn is_git_config_injection(args: &str) -> bool {
    let lower = args.to_lowercase();
    // `git -c <key>=<val>` or `git clone --config <key>=<val>`
    if !lower.contains("-c ") && !lower.contains("--config ") && !lower.contains("--config=") {
        return false;
    }
    GIT_BLOCKED_CONFIG_KEYS
        .iter()
        .any(|key| lower.contains(key))
}

fn is_path_like_argument(arg: &str) -> bool {
    arg.starts_with('/')
        || arg.starts_with("~/")
        || arg.contains('/')
        || arg.contains('\\')
        || arg.contains("..")
}

fn strip_wrapping_quotes(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('\"') && value.ends_with('\"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        return &value[1..value.len() - 1];
    }
    value
}

fn extract_path_arg(arg: &str) -> Option<&str> {
    let raw = arg.trim();
    if raw.is_empty() {
        return None;
    }

    // Defence-in-depth: check the raw argument (with quotes) first so that
    // quoting cannot hide path-like patterns from detection.
    if is_path_like_argument(raw) {
        let trimmed = strip_wrapping_quotes(raw);
        return Some(trimmed);
    }

    let trimmed = strip_wrapping_quotes(raw);
    if trimmed.is_empty() {
        return None;
    }

    if is_path_like_argument(trimmed) {
        return Some(trimmed);
    }

    let (_, value) = trimmed.split_once('=')?;
    let value = strip_wrapping_quotes(value.trim());
    (!value.is_empty() && is_path_like_argument(value)).then_some(value)
}

fn canonical_workspace_root(workspace_dir: &Path) -> Option<PathBuf> {
    match workspace_dir.canonicalize() {
        Ok(resolved) => Some(resolved),
        Err(error) => {
            tracing::warn!(
                workspace_dir = %workspace_dir.display(),
                %error,
                "failed to canonicalize workspace root; \
                 denying path argument to prevent symlink escape"
            );
            None
        }
    }
}

fn canonicalize_or_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn should_enforce_workspace_containment(policy: &SecurityPolicy, workspace_dir: &Path) -> bool {
    if policy.workspace_only {
        return true;
    }
    let policy_workspace = canonicalize_or_path(&policy.workspace_dir);
    let request_workspace = canonicalize_or_path(workspace_dir);
    request_workspace != policy_workspace
}

fn nearest_existing_path(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

fn is_path_argument_allowed(policy: &SecurityPolicy, workspace_dir: &Path, path_arg: &str) -> bool {
    if !policy.is_path_allowed(path_arg) {
        return false;
    }

    let enforce_workspace_containment = should_enforce_workspace_containment(policy, workspace_dir);
    let workspace_root = if enforce_workspace_containment {
        let Some(workspace_root) = canonical_workspace_root(workspace_dir) else {
            // Cannot verify containment — deny to prevent symlink escapes.
            return false;
        };
        Some(workspace_root)
    } else {
        None
    };
    let joined = if Path::new(path_arg).is_absolute() {
        PathBuf::from(path_arg)
    } else {
        workspace_dir.join(path_arg)
    };

    let is_allowed_path = |path: &Path| -> bool {
        let Ok(resolved) = path.canonicalize() else {
            return false;
        };
        if let Some(workspace_root) = &workspace_root
            && !resolved.starts_with(workspace_root)
        {
            return false;
        }
        policy.is_path_allowed_resolved(&resolved)
    };

    if joined.exists() {
        return is_allowed_path(&joined);
    }

    if let Some(existing_parent) = nearest_existing_path(&joined) {
        return is_allowed_path(&existing_parent);
    }

    // No ancestor exists on disk — cannot verify containment, deny.
    false
}

fn has_forbidden_path_argument(
    policy: &SecurityPolicy,
    workspace_dir: &Path,
    words: &[&str],
) -> bool {
    words
        .iter()
        .copied()
        .filter_map(extract_path_arg)
        .any(|word| !is_path_argument_allowed(policy, workspace_dir, word))
}

fn find_exec_terminator(token: &str) -> Option<(Option<&str>, bool)> {
    if token == "+" {
        return Some((None, true));
    }
    if token == r"\;" || token == ";" {
        return Some((None, true));
    }
    if let Some(stripped) = token.strip_suffix(r"\;") {
        let command_token = (!stripped.is_empty()).then_some(stripped);
        return Some((command_token, true));
    }
    None
}

fn extract_find_exec_payload(words: &[&str], start_idx: usize) -> Option<(String, usize)> {
    let mut exec_tokens = Vec::new();

    for (index, token) in words.iter().enumerate().skip(start_idx) {
        if let Some((command_token, is_terminator)) = find_exec_terminator(token) {
            if let Some(command_token) = command_token {
                exec_tokens.push(command_token);
            }
            if is_terminator {
                if exec_tokens.is_empty() {
                    return None;
                }
                return Some((exec_tokens.join(" "), index));
            }
        }

        exec_tokens.push(token);
    }

    None
}

fn has_blocked_arguments_with_context(
    base_cmd: &str,
    full_segment: &str,
    allowed_commands: &[String],
    policy: &SecurityPolicy,
    workspace_dir: &Path,
) -> bool {
    let args = full_segment
        .trim()
        .strip_prefix(base_cmd)
        .unwrap_or("")
        .trim_start();

    let words: Vec<&str> = args.split_whitespace().collect();
    let subcommand = words.first().copied().unwrap_or("");

    match base_cmd {
        "git" => {
            // Network egress
            if matches!(subcommand, "push" | "send-email" | "request-pull") {
                return true;
            }
            // Credential theft
            if subcommand == "credential" {
                return true;
            }
            // Remote mutation (allow read-only: -v, show, get-url)
            if subcommand == "remote" {
                let sub_action = words.get(1).copied().unwrap_or("");
                return !matches!(sub_action, "" | "-v" | "show" | "get-url");
            }
            // Config: allow reads, block writes and --global/--system
            if subcommand == "config" {
                let has_write_flag = words.iter().any(|w| matches!(*w, "--global" | "--system"));
                let positional_arg_count =
                    words.iter().skip(1).filter(|w| !w.starts_with('-')).count();
                return has_write_flag || positional_arg_count > 1;
            }
            // Submodule: block only `add` (pulls from external URL)
            if subcommand == "submodule" {
                let sub_action = words.get(1).copied().unwrap_or("");
                return sub_action == "add";
            }
            // Protocol-level code execution
            if words.iter().any(|w| {
                *w == "--upload-pack"
                    || w.starts_with("--upload-pack=")
                    || *w == "--receive-pack"
                    || w.starts_with("--receive-pack=")
            }) {
                return true;
            }
            // Config injection via -c / --config
            if is_git_config_injection(args) {
                return true;
            }
            false
        }
        "npm" => matches!(
            subcommand,
            "publish" | "login" | "adduser" | "owner" | "token" | "access" | "profile"
        ),
        "cargo" => matches!(subcommand, "publish" | "login" | "owner" | "yank"),
        "find" => {
            if words.contains(&"-delete") {
                return true;
            }
            let mut i = 0;
            while i < words.len() {
                if words[i] == "-exec" || words[i] == "-execdir" {
                    let Some((exec_payload, terminator_index)) =
                        extract_find_exec_payload(&words, i + 1)
                    else {
                        return true;
                    };

                    if has_leading_env_assignments(&exec_payload) {
                        return true;
                    }
                    let exec_cmd_part = exec_payload.as_str();
                    let Some(exec_cmd) = exec_cmd_part.split_whitespace().next() else {
                        return true;
                    };

                    // Keep `find -exec` aligned with the top-level command
                    // grammar: the executable itself must be an allowlisted
                    // basename, not a caller-supplied path. Otherwise an
                    // allowed basename such as `cat` could be smuggled through
                    // `/tmp/cat` or `C:\tmp\cat` and bypass the policy's
                    // executable-origin check.
                    if exec_cmd.contains('/') || exec_cmd.contains('\\') {
                        return true;
                    }

                    let exec_base = exec_cmd.rsplit('/').next().unwrap_or(exec_cmd);
                    if !allowed_commands.iter().any(|a| a == exec_base) {
                        return true;
                    }

                    let exec_words: Vec<&str> = exec_cmd_part.split_whitespace().collect();
                    if has_forbidden_path_argument(
                        policy,
                        workspace_dir,
                        exec_words.get(1..).unwrap_or(&[]),
                    ) {
                        return true;
                    }

                    if has_blocked_arguments_with_context(
                        exec_base,
                        exec_cmd_part,
                        allowed_commands,
                        policy,
                        workspace_dir,
                    ) {
                        return true;
                    }

                    i = terminator_index;
                }
                i += 1;
            }
            false
        }
        _ => false,
    }
}

#[cfg(test)]
fn has_blocked_args(base_cmd: &str, full_segment: &str, allowed_commands: &[String]) -> bool {
    has_blocked_arguments_with_context(
        base_cmd,
        full_segment,
        allowed_commands,
        &SecurityPolicy::default(),
        Path::new("."),
    )
}

impl SecurityPolicy {
    /// Check if a shell command is allowed.
    ///
    /// Validates the **entire** command string, not just the first word:
    /// - Blocks subshell operators (`` ` ``, `$(`) that hide arbitrary execution
    /// - Splits on command separators (`|`, `&&`, `||`, `;`, newlines) and
    ///   validates each sub-command against the allowlist
    /// - Blocks output redirections (`>`, `>>`) that could write outside workspace
    /// - Blocks dangerous arguments/subcommands that enable code execution,
    ///   network egress, or credential access
    ///
    /// # Quoting Limitation
    ///
    /// This function performs **text-level** parsing and does **not** interpret
    /// shell quoting (single quotes, double quotes, backslash escapes). As a
    /// result, quoted strings containing metacharacters (e.g. `echo "a;b"`) may
    /// be rejected even though a real shell would treat them as literal text.
    /// This is a deliberate **false-positive** (safe) bias: the parser may
    /// over-block but will never under-block due to quoting ambiguity.
    /// Callers that pass pre-split arguments should use
    /// [`enforce_process_spawn_policy_with_args`](crate::security::process_spawn)
    /// instead.
    #[must_use]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        self.is_command_allowed_in_workspace(command, &self.workspace_dir)
    }

    /// Check if a shell command is allowed within a specific workspace.
    #[must_use]
    pub fn is_command_allowed_in_workspace(&self, command: &str, workspace_dir: &Path) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }

        // Internal sentinel bytes are used below while splitting the command.
        // Reject them in user input before normalization so a literal control
        // character cannot spoof a separator or escaped-semicolon marker.
        if command.contains('\0') || command.contains('\u{1}') {
            return false;
        }

        // Block subshell/expansion operators — these allow hiding arbitrary
        // commands inside an allowed command (e.g. `echo $(rm -rf /)`)
        if command.contains('`')
            || command.contains("$(")
            || command.contains("${")
            || command.contains("<(")
            || command.contains(">(")
        {
            return false;
        }

        // Block output redirections — they can write to arbitrary paths
        if command.contains('>') {
            return false;
        }

        // Split on command separators and validate each sub-command.
        let mut normalized = command.to_string();
        for sep in ["&&", "||"] {
            normalized = normalized.replace(sep, "\x00");
        }
        if normalized.contains('&') {
            return false;
        }
        normalized = normalized.replace(r"\;", "\x01");
        for sep in ['\n', ';', '|'] {
            normalized = normalized.replace(sep, "\x00");
        }

        for segment in normalized.split('\x00') {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }

            let segment = segment.replace('\x01', r"\;");
            if has_leading_env_assignments(&segment) {
                return false;
            }
            let cmd_part = segment.as_str();

            let executable = cmd_part.split_whitespace().next().unwrap_or("");
            if executable.contains('/') || executable.contains('\\') {
                return false;
            }

            let base_cmd = executable;

            if base_cmd.is_empty() {
                continue;
            }

            if !self
                .allowed_commands
                .iter()
                .any(|allowed| allowed == base_cmd)
            {
                return false;
            }

            let words: Vec<&str> = cmd_part.split_whitespace().collect();
            if has_forbidden_path_argument(self, workspace_dir, words.get(1..).unwrap_or(&[])) {
                return false;
            }

            if has_blocked_arguments_with_context(
                base_cmd,
                cmd_part,
                &self.allowed_commands,
                self,
                workspace_dir,
            ) {
                return false;
            }
        }

        // At least one command must be present
        normalized.split('\x00').any(|s| {
            let s = s.trim();
            if has_leading_env_assignments(s) {
                return false;
            }
            s.split_whitespace().next().is_some_and(|w| !w.is_empty())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{
        AutonomyLevel, SecurityPolicy, has_blocked_args, has_leading_env_assignments,
        is_git_config_injection, skip_env_assignments,
    };

    fn policy_with_allowed(allowed_commands: &[&str]) -> SecurityPolicy {
        SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            allowed_commands: allowed_commands.iter().map(ToString::to_string).collect(),
            ..SecurityPolicy::default()
        }
    }

    #[test]
    fn skip_env_assignments_strips_single_assignment() {
        assert_eq!(skip_env_assignments("VAR=value cmd"), "cmd");
    }

    #[test]
    fn skip_env_assignments_strips_multiple_assignments() {
        assert_eq!(
            skip_env_assignments("VAR1=a VAR2=b cmd --flag"),
            "cmd --flag"
        );
    }

    #[test]
    fn skip_env_assignments_keeps_plain_command() {
        assert_eq!(skip_env_assignments("cmd"), "cmd");
    }

    #[test]
    fn skip_env_assignments_handles_empty_input() {
        assert_eq!(skip_env_assignments(""), "");
    }

    #[test]
    fn is_git_config_injection_ignores_normal_git_commands() {
        assert!(!is_git_config_injection("status"));
        assert!(!is_git_config_injection("log --oneline"));
        assert!(!is_git_config_injection("diff --name-only"));
    }

    #[test]
    fn is_git_config_injection_handles_dash_c_patterns() {
        assert!(!is_git_config_injection("-c user.email=evil@hack"));
        assert!(is_git_config_injection("-c core.sshCommand=sh"));
    }

    #[test]
    fn is_git_config_injection_detects_config_flags_in_various_positions() {
        assert!(is_git_config_injection("clone --config core.pager=sh repo"));
        assert!(is_git_config_injection(
            "clone --config=core.editor=sh repo"
        ));
        assert!(is_git_config_injection("status -c credential.helper=!sh"));
    }

    #[test]
    fn has_blocked_arguments_git_dangerous_subcommands_are_blocked() {
        assert!(has_blocked_args("git", "git push", &[]));
        assert!(has_blocked_args("git", "git send-email", &[]));
        assert!(has_blocked_args("git", "git request-pull", &[]));
        assert!(has_blocked_args("git", "git remote add origin x", &[]));
        assert!(has_blocked_args("git", "git remote set-url origin x", &[]));
        assert!(has_blocked_args(
            "git",
            "git config --global user.name x",
            &[]
        ));
        assert!(has_blocked_args("git", "git credential fill", &[]));
    }

    #[test]
    fn has_blocked_arguments_git_safe_subcommands_are_allowed() {
        assert!(!has_blocked_args("git", "git status", &[]));
        assert!(!has_blocked_args("git", "git log", &[]));
        assert!(!has_blocked_args("git", "git diff", &[]));
    }

    #[test]
    fn has_blocked_arguments_npm_rules() {
        assert!(has_blocked_args("npm", "npm publish", &[]));
        assert!(!has_blocked_args("npm", "npm list", &[]));
        assert!(!has_blocked_args("npm", "npm run test", &[]));
    }

    #[test]
    fn has_blocked_arguments_cargo_rules() {
        assert!(has_blocked_args("cargo", "cargo publish", &[]));
        assert!(!has_blocked_args("cargo", "cargo build", &[]));
        assert!(!has_blocked_args("cargo", "cargo test", &[]));
    }

    #[test]
    fn has_blocked_arguments_pip_is_unrestricted_by_this_filter() {
        assert!(!has_blocked_args("pip", "pip install foo", &[]));
        assert!(!has_blocked_args("pip", "pip uninstall foo", &[]));
        assert!(!has_blocked_args("pip", "pip list", &[]));
    }

    #[test]
    fn is_command_allowed_accepts_safe_allowed_commands() {
        let policy = policy_with_allowed(&["git", "npm", "cargo", "pip"]);
        assert!(policy.is_command_allowed("git status"));
        assert!(policy.is_command_allowed("npm run test"));
        assert!(policy.is_command_allowed("cargo test"));
    }

    #[test]
    fn is_command_allowed_rejects_blocked_arguments() {
        let policy = policy_with_allowed(&["git", "npm", "cargo"]);
        assert!(!policy.is_command_allowed("git push"));
        assert!(!policy.is_command_allowed("git -c core.sshCommand=sh status"));
        assert!(!policy.is_command_allowed("npm publish"));
        assert!(!policy.is_command_allowed("cargo publish"));
    }

    #[test]
    fn is_command_allowed_rejects_path_qualified_executables() {
        let policy = policy_with_allowed(&["git"]);
        assert!(!policy.is_command_allowed("/tmp/git status"));
        assert!(!policy.is_command_allowed(r"C:\\tmp\\git status"));
    }

    #[test]
    fn is_command_allowed_rejects_empty_and_whitespace_only_commands() {
        let policy = policy_with_allowed(&["git"]);
        assert!(!policy.is_command_allowed(""));
        assert!(!policy.is_command_allowed("   \t  \n  "));
    }

    #[test]
    fn has_leading_env_assignments_detects_assignment_prefix() {
        assert!(has_leading_env_assignments("VAR=a git status"));
        assert!(!has_leading_env_assignments("git status"));
    }

    #[test]
    fn is_command_allowed_rejects_env_prefixes() {
        let policy = policy_with_allowed(&["git"]);
        assert!(!policy.is_command_allowed("  VAR=a   git   status   "));
    }

    #[test]
    fn is_command_allowed_is_case_sensitive() {
        let policy = policy_with_allowed(&["git"]);
        assert!(policy.is_command_allowed("git status"));
        assert!(!policy.is_command_allowed("Git status"));
    }

    #[test]
    fn is_command_allowed_rejects_subshell_expansion_and_redirection() {
        let policy = policy_with_allowed(&["echo"]);
        assert!(!policy.is_command_allowed("echo $(whoami)"));
        assert!(!policy.is_command_allowed("echo hi > out.txt"));
    }

    #[test]
    fn is_command_allowed_rejects_background_operator_but_allows_logical_and() {
        let policy = policy_with_allowed(&["git", "echo"]);
        assert!(policy.is_command_allowed("git status && echo ok"));
        assert!(!policy.is_command_allowed("git status & echo ok"));
    }

    #[test]
    fn is_command_allowed_rejects_process_substitution() {
        let policy = policy_with_allowed(&["echo", "cat"]);
        assert!(!policy.is_command_allowed("echo <(cat Cargo.toml)"));
        assert!(!policy.is_command_allowed("cat >(wc -l)"));
    }

    #[test]
    fn is_command_allowed_rejects_mixed_segments_with_one_disallowed_command() {
        let policy = policy_with_allowed(&["git", "echo"]);
        assert!(policy.is_command_allowed("git status && echo ok"));
        assert!(!policy.is_command_allowed("git status && curl https://example.com"));
    }

    #[test]
    fn is_command_allowed_denies_all_in_read_only_mode() {
        let mut policy = policy_with_allowed(&["git"]);
        policy.autonomy = AutonomyLevel::ReadOnly;
        assert!(!policy.is_command_allowed("git status"));
    }

    #[test]
    fn has_blocked_arguments_find_exec_blocks_forbidden_paths() {
        let allowed = vec!["find".to_string(), "cat".to_string()];
        assert!(has_blocked_args(
            "find",
            r"find . -name '*.txt' -exec cat /etc/shadow \;",
            &allowed
        ));
    }

    #[test]
    fn has_blocked_arguments_find_exec_blocks_disallowed_subcommands() {
        let allowed = vec!["find".to_string(), "cat".to_string()];
        assert!(has_blocked_args(
            "find",
            r"find . -exec curl https://example.com \;",
            &allowed
        ));
    }

    #[test]
    fn is_command_allowed_rejects_find_exec_with_forbidden_paths() {
        let policy = policy_with_allowed(&["find", "cat"]);
        assert!(!policy.is_command_allowed(r"find . -name '*.txt' -exec cat /etc/shadow \;"));
    }

    #[test]
    fn is_command_allowed_rejects_find_exec_with_disallowed_subcommands() {
        let policy = policy_with_allowed(&["find", "cat"]);
        assert!(!policy.is_command_allowed(r"find . -type f -exec curl https://example.com \;"));
    }

    #[test]
    fn is_command_allowed_rejects_find_exec_with_path_qualified_executable() {
        let policy = policy_with_allowed(&["find", "cat"]);
        assert!(!policy.is_command_allowed(r"find . -type f -exec /bin/cat README.md \;"));
        assert!(!policy.is_command_allowed(r"find . -type f -exec C:\\tmp\\cat README.md \;"));
    }

    #[test]
    fn is_command_allowed_rejects_disallowed_executable_in_every_segment_kind() {
        let policy = policy_with_allowed(&["git", "echo"]);
        assert!(!policy.is_command_allowed("git status | curl https://example.com"));
        assert!(!policy.is_command_allowed("git status; curl https://example.com"));
        assert!(!policy.is_command_allowed("git status || curl https://example.com"));
    }

    #[test]
    fn is_command_allowed_rejects_path_qualified_executable_in_later_segment() {
        let policy = policy_with_allowed(&["git", "echo"]);
        assert!(!policy.is_command_allowed("git status && /bin/echo ok"));
    }

    #[test]
    #[cfg(unix)]
    fn is_command_allowed_in_workspace_blocks_symlink_escape_paths() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().expect("tempdir");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&outside).expect("create outside");
        fs::write(outside.join("secret.txt"), "secret").expect("write secret");
        symlink(&outside, workspace.join("skills")).expect("create symlink");

        let policy = SecurityPolicy {
            workspace_dir: workspace.clone(),
            allowed_commands: vec!["cat".to_string()],
            ..SecurityPolicy::default()
        };

        assert!(!policy.is_command_allowed_in_workspace("cat skills/secret.txt", &workspace));
    }

    #[test]
    fn is_command_allowed_in_workspace_allows_existing_workspace_path() {
        let root = TempDir::new().expect("tempdir");
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(workspace.join("README.md"), "ok").expect("write file");

        let policy = SecurityPolicy {
            workspace_dir: workspace.clone(),
            allowed_commands: vec!["cat".to_string()],
            ..SecurityPolicy::default()
        };

        assert!(policy.is_command_allowed_in_workspace("cat README.md", &workspace));
    }

    #[test]
    fn is_command_allowed_in_workspace_respects_workspace_only_false_for_primary_workspace() {
        let root = TempDir::new().expect("tempdir");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&outside).expect("create outside");
        let outside_file = outside.join("shared.txt");
        fs::write(&outside_file, "ok").expect("write outside file");

        let policy = SecurityPolicy {
            workspace_dir: workspace.clone(),
            workspace_only: false,
            forbidden_paths: vec![],
            allowed_commands: vec!["cat".to_string()],
            ..SecurityPolicy::default()
        };

        let command = format!("cat {}", outside_file.display());
        assert!(policy.is_command_allowed_in_workspace(&command, &workspace));
    }

    #[test]
    fn is_command_allowed_in_workspace_keeps_group_boundary_when_workspace_only_false() {
        let root = TempDir::new().expect("tempdir");
        let workspace = root.path().join("workspace");
        let group_a = workspace.join("groups/a");
        let group_b = workspace.join("groups/b");
        fs::create_dir_all(&group_a).expect("create group a");
        fs::create_dir_all(&group_b).expect("create group b");
        let group_b_file = group_b.join("secret.txt");
        fs::write(&group_b_file, "secret").expect("write group b file");

        let policy = SecurityPolicy {
            workspace_dir: workspace,
            workspace_only: false,
            forbidden_paths: vec![],
            allowed_commands: vec!["cat".to_string()],
            ..SecurityPolicy::default()
        };

        let command = format!("cat {}", group_b_file.display());
        assert!(!policy.is_command_allowed_in_workspace(&command, &group_a));
    }

    #[test]
    fn extract_path_candidate_detects_quoted_path_traversal() {
        use super::extract_path_arg;

        // Quoted path traversal must be detected even through wrapping quotes.
        assert!(extract_path_arg(r#""../../etc/passwd""#).is_some());
        assert!(extract_path_arg(r"'../../etc/passwd'").is_some());
        // Raw path traversal continues to work.
        assert!(extract_path_arg("../../etc/passwd").is_some());
        // Non-path arguments remain undetected.
        assert!(extract_path_arg("hello").is_none());
    }

    #[test]
    fn is_command_allowed_rejects_quoted_path_traversal() {
        let root = TempDir::new().expect("tempdir");
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");

        let policy = SecurityPolicy {
            workspace_dir: workspace.clone(),
            allowed_commands: vec!["cat".to_string()],
            ..SecurityPolicy::default()
        };

        // Quoted paths that escape workspace must be rejected.
        assert!(!policy.is_command_allowed_in_workspace(r#"cat "../../etc/passwd""#, &workspace,));
    }

    #[test]
    fn is_command_allowed_rejects_internal_sentinel_control_bytes() {
        let policy = SecurityPolicy {
            allowed_commands: vec!["echo".to_string()],
            ..SecurityPolicy::default()
        };

        assert!(!policy.is_command_allowed("echo safe\0echo bypass"));
        assert!(!policy.is_command_allowed("echo safe\u{1}echo bypass"));
    }
}
