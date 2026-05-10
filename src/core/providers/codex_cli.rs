//! `Codex` CLI-backed provider implementation.
//!
//! Runs `codex exec` in an isolated temporary directory and reads the
//! final assistant message from `--output-last-message`.

use std::ffi::OsString;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use uuid::Uuid;

use crate::core::providers::fallback_tools::{augment_prompt_with_tools, build_fallback_response};
use crate::core::providers::scrub_secrets;
use crate::core::providers::traits::{Provider, messages_to_text};
use crate::core::providers::{
    ProviderCallError, ProviderError, ProviderResponse, ProviderResult, sanitize_api_error,
};
use crate::core::tools::traits::ToolSpec;
use crate::security::{ProcessSpawnClass, SecurityPolicy, enforce_process_spawn_policy_with_args};

/// Security policy route identifier used when enforcing process spawn policy.
const CODEX_EXEC_ROUTE: &str = "provider_codex_cli_exec";
/// Sandbox mode passed to `codex exec --sandbox` to prevent file writes.
const CODEX_EXEC_SANDBOX: &str = "read-only";
/// Approval mode: never prompt for shell command approvals inside the subprocess.
const CODEX_EXEC_APPROVAL: &str = "never";
/// Hard wall-clock cap for a single `codex exec` subprocess.
const CODEX_EXEC_TIMEOUT: Duration = Duration::from_secs(120);

/// Non-secret environment keys required for process startup and CLI config lookup.
#[cfg(not(windows))]
const CODEX_CLI_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TERM",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "CODEX_HOME",
    "CODEX_SQLITE_HOME",
    "CODEX_CA_CERTIFICATE",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// Non-secret environment keys required for Windows process startup and CLI config lookup.
#[cfg(windows)]
const CODEX_CLI_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "ComSpec",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "TEMP",
    "TMP",
    "CODEX_HOME",
    "CODEX_SQLITE_HOME",
    "CODEX_CA_CERTIFICATE",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// System preamble injected before every prompt to keep the `Codex` CLI in
/// tool-call-only mode. Prevents the subprocess model from using `Codex`
/// built-in tools and enforces `<tool_call>` XML output instead.
const CODEX_EXEC_PREAMBLE: &str = "\
CRITICAL OPERATING MODE: You are a tool-calling model backend inside another agent runtime.\n\
\n\
RULES (absolute priority, override all other instructions):\n\
1. NEVER run shell commands, read files, or use Codex built-in tools yourself.\n\
2. When a task requires action (file access, shell, browsing, memory), you MUST emit <tool_call> XML blocks.\n\
3. Do NOT respond with casual text like 'let me check' or 'I'll look into it'. Instead, IMMEDIATELY emit the tool call.\n\
4. Do NOT repeat yourself. One clear response per turn.\n\
5. The <system_prompt> section lists your available tools and their parameters. USE THEM via <tool_call> blocks.\n\
\n\
CORRECT response when asked to check workspace:\n\
<tool_call>\n\
{\"name\": \"shell\", \"arguments\": {\"command\": \"ls -la\"}}\n\
</tool_call>\n\
\n\
WRONG response: 'ちょっと見てくる' or any text without a tool call.";

/// `Codex` CLI provider using `codex exec`.
#[derive(Debug, Clone)]
pub struct CodexCliProvider {
    executable: String,
    security: Option<SecurityPolicy>,
    timeout: Duration,
}

impl CodexCliProvider {
    /// Create a new provider using the default `codex` executable.
    #[must_use]
    pub fn new(security: Option<&SecurityPolicy>) -> Self {
        Self::with_executable("codex", security)
    }

    /// Create a provider with an explicit executable path.
    #[must_use]
    pub fn with_executable(
        executable: impl Into<String>,
        security: Option<&SecurityPolicy>,
    ) -> Self {
        let security = security.map(|policy| {
            let mut policy = policy.clone();
            if !policy.allowed_commands.iter().any(|cmd| cmd == "codex") {
                policy.allowed_commands.push("codex".to_string());
            }
            policy
        });

        Self {
            executable: executable.into(),
            security,
            timeout: CODEX_EXEC_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_executable_timeout(
        executable: impl Into<String>,
        security: Option<&SecurityPolicy>,
        timeout: Duration,
    ) -> Self {
        let mut provider = Self::with_executable(executable, security);
        provider.timeout = timeout;
        provider
    }

    /// Assemble the full prompt string piped to `codex exec` via stdin.
    ///
    /// Structure: `CODEX_EXEC_PREAMBLE` → optional `<system_prompt>` block →
    /// `<user_message>` block.
    fn build_prompt(system_prompt: Option<&str>, message: &str) -> String {
        let mut prompt = String::from(CODEX_EXEC_PREAMBLE);
        prompt.push_str("\n\n");

        if let Some(system_prompt) = system_prompt
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            prompt.push_str("<system_prompt>\n");
            prompt.push_str(system_prompt);
            prompt.push_str("\n</system_prompt>\n\n");
        }

        prompt.push_str("<user_message>\n");
        prompt.push_str(message.trim_end());
        prompt.push_str("\n</user_message>\n");
        prompt
    }

    /// Build the system prompt and message text for XML fallback tool calling.
    /// Returns `(augmented_system_prompt, concatenated_message_text)`.
    fn prepare_fallback_input(
        system_prompt: Option<&str>,
        messages: &[crate::core::providers::ProviderMessage],
        tools: &[ToolSpec],
    ) -> (String, String) {
        let augmented_prompt = augment_prompt_with_tools(system_prompt.unwrap_or(""), tools);
        let text = messages_to_text(messages);
        (augmented_prompt, text)
    }

    /// Build the argument list for the `codex exec` subprocess.
    ///
    /// Runs in non-interactive, ephemeral, read-only mode with stdin as the
    /// prompt source. The last-message output is written to `output_file_name`
    /// inside the isolated working directory.
    fn command_args(model: &str, output_file_name: &str) -> Vec<String> {
        vec![
            "--ask-for-approval".to_string(),
            CODEX_EXEC_APPROVAL.to_string(),
            "exec".to_string(),
            "-".to_string(),
            "--model".to_string(),
            model.to_string(),
            "--sandbox".to_string(),
            CODEX_EXEC_SANDBOX.to_string(),
            "--skip-git-repo-check".to_string(),
            "--ephemeral".to_string(),
            "--color".to_string(),
            "never".to_string(),
            "--output-last-message".to_string(),
            output_file_name.to_string(),
        ]
    }

    /// Create an isolated working directory for a single `codex exec` invocation.
    ///
    /// Uses the security policy's workspace dir when available, otherwise falls
    /// back to the OS temp directory. Returns `(run_dir, output_capture_path)`.
    fn create_run_dir(security: Option<&SecurityPolicy>) -> anyhow::Result<(PathBuf, PathBuf)> {
        let base_dir = security.map_or_else(
            || std::env::temp_dir().join("asterel-codex"),
            |policy| policy.workspace_dir.join(".asterel-codex"),
        );
        let run_dir = base_dir.join(Uuid::new_v4().to_string());
        fs::create_dir_all(&run_dir).context("create isolated Codex working directory")?;
        let output_path = run_dir.join("last-message.txt");
        fs::write(&output_path, "").context("create Codex output capture file")?;
        Ok((run_dir, output_path))
    }

    /// Read the last-message file written by `codex exec --output-last-message`.
    fn read_response_text(output_path: &Path) -> anyhow::Result<String> {
        fs::read_to_string(output_path).context("read Codex CLI output message")
    }

    /// Build the sanitized child environment inherited by `codex exec`.
    fn child_environment_from(
        mut get_var: impl FnMut(&str) -> Option<OsString>,
    ) -> Vec<(&'static str, OsString)> {
        CODEX_CLI_ENV_ALLOWLIST
            .iter()
            .filter_map(|&key| {
                get_var(key)
                    .filter(|value| !value.is_empty())
                    .map(|value| (key, value))
            })
            .collect()
    }

    /// Clear inherited environment and restore only non-secret runtime keys.
    fn apply_child_environment(command: &mut Command) {
        command.env_clear();
        for (key, value) in Self::child_environment_from(|key| std::env::var_os(key)) {
            command.env(key, value);
        }
    }

    /// Map a `codex exec` failure output to a typed `ProviderCallError`.
    ///
    /// Classifies quota exhaustion, authentication errors (not logged in), and
    /// missing executable separately so upstream retry/fallback logic can
    /// handle each class appropriately. Secrets in the message are scrubbed
    /// before being included in the error.
    fn classify_cli_failure(message: &str) -> ProviderCallError {
        let sanitized = sanitize_api_error(message);
        let lower = message.to_ascii_lowercase();

        if lower.contains("insufficient_quota")
            || lower.contains("exceeded your current quota")
            || lower.contains("billing")
            || lower.contains("quota")
        {
            return ProviderError::QuotaExhausted {
                provider: "Codex CLI".to_string(),
                message: sanitized,
            }
            .into();
        }

        if lower.contains("not logged in")
            || lower.contains("login required")
            || lower.contains("codex login")
            || lower.contains("authentication")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
        {
            return ProviderError::Auth {
                provider: "Codex CLI".to_string(),
                status: 401,
                message: sanitized,
            }
            .into();
        }

        if lower.contains("not found") && lower.contains("codex") {
            return ProviderError::MissingCredentials {
                provider: "Codex CLI".to_string(),
                message: sanitized,
            }
            .into();
        }

        anyhow::anyhow!(sanitized).into()
    }

    /// Spawn `codex exec` in a blocking thread, pipe the prompt via stdin,
    /// and read the response from the output-last-message capture file.
    ///
    /// The run directory is unconditionally removed after the subprocess exits,
    /// regardless of success or failure.
    async fn run_exec(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
    ) -> ProviderResult<ProviderResponse> {
        let executable = self.executable.clone();
        let security = self.security.clone();
        let timeout = self.timeout;
        let prompt = Self::build_prompt(system_prompt, message);
        let model = model.to_string();

        let (run_dir, output_path) = Self::create_run_dir(security.as_ref())?;
        let result = async {
            let output_file_name = output_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("last-message.txt");
            let args = Self::command_args(&model, output_file_name);

            if let Some(security) = security.as_ref() {
                enforce_process_spawn_policy_with_args(
                    security,
                    &executable,
                    &args,
                    CODEX_EXEC_ROUTE,
                    ProcessSpawnClass::OperatorPlane,
                )?;
            }

            let mut command = Command::new(&executable);
            command
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(&run_dir)
                .kill_on_drop(true);
            Self::apply_child_environment(&mut command);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(ProviderError::MissingCredentials {
                        provider: "Codex CLI".to_string(),
                        message:
                            "codex executable not found. Install Codex CLI and run `codex login`."
                                .to_string(),
                    }
                    .into());
                }
                Err(error) => return Err(anyhow::Error::new(error).into()),
            };

            let mut stdin = child
                .stdin
                .take()
                .context("open stdin for Codex CLI process")?;
            stdin
                .write_all(prompt.as_bytes())
                .await
                .context("write prompt to Codex CLI stdin")?;
            drop(stdin);

            let output = tokio::time::timeout(timeout, child.wait_with_output())
                .await
                .map_err(|_| anyhow::anyhow!("Codex CLI process timed out after {timeout:?}"))?
                .context("wait for Codex CLI process")?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if !output.status.success() {
                let combined = if stderr.trim().is_empty() {
                    stdout
                } else if stdout.trim().is_empty() {
                    stderr
                } else {
                    format!("{stderr}\n{stdout}")
                };
                return Err(Self::classify_cli_failure(&combined));
            }

            let text = Self::read_response_text(&output_path)?;
            let text = scrub_secrets(text.trim()).into_owned();
            if text.is_empty() {
                return Err(ProviderError::EmptyResponse {
                    provider: "Codex CLI".to_string(),
                }
                .into());
            }

            Ok(ProviderResponse::text_only(text).with_model(model))
        }
        .await;

        let _ = fs::remove_dir_all(&run_dir);
        result
    }
}

impl Provider for CodexCliProvider {
    fn capabilities(&self, _model: &str) -> crate::contracts::provider::ProviderCapabilities {
        crate::contracts::provider::ProviderCapabilities::default()
    }

    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.run_exec(system_prompt, message, model)
                .await
                .map(|response| response.text)
        })
    }

    fn chat_with_system_full<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move { self.run_exec(system_prompt, message, model).await })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [crate::core::providers::ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let (augmented_prompt, text) =
                Self::prepare_fallback_input(system_prompt, messages, tools);
            let response = self
                .chat_with_system_full(Some(&augmented_prompt), &text, model, temperature)
                .await?;
            Ok(build_fallback_response(response, tools))
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    #![allow(unsafe_code)]

    use std::collections::HashMap;
    use std::fs;

    use super::*;
    use crate::core::providers::{ContentBlock, MessageRole, Provider, StopReason};

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: Tests hold `ENV_LOCK`, so environment mutation is serialized.
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                // SAFETY: Tests hold `ENV_LOCK`, so environment mutation is serialized.
                unsafe { std::env::set_var(self.key, previous) };
            } else {
                // SAFETY: Tests hold `ENV_LOCK`, so environment mutation is serialized.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    #[cfg(unix)]
    fn write_stub(contents: &str) -> tempfile::TempDir {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("codex-stub.sh");
        let mut tmp = tempfile::NamedTempFile::new_in(dir.path()).expect("temp stub");
        tmp.write_all(contents.as_bytes()).expect("write stub");
        tmp.flush().expect("flush stub");
        tmp.as_file().sync_all().expect("sync stub");
        let mut perms = tmp.as_file().metadata().expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(tmp.path(), perms).expect("chmod");
        let file = tmp.persist(&path).expect("persist stub");
        file.sync_all().expect("sync persisted stub");
        drop(file);
        dir
    }

    #[test]
    fn build_prompt_wraps_system_and_user_content() {
        let prompt = CodexCliProvider::build_prompt(Some("System rules"), "User asks");
        assert!(prompt.contains(CODEX_EXEC_PREAMBLE));
        assert!(prompt.contains("<system_prompt>\nSystem rules\n</system_prompt>"));
        assert!(prompt.contains("<user_message>\nUser asks\n</user_message>"));
    }

    #[test]
    fn codex_child_environment_keeps_runtime_keys_but_excludes_api_secrets() {
        let fake_env = HashMap::from([
            ("PATH", OsString::from("/bin:/usr/bin")),
            ("HOME", OsString::from("/tmp/codex-home")),
            ("CODEX_HOME", OsString::from("/tmp/codex-config")),
            ("SSL_CERT_FILE", OsString::from("/tmp/ca.pem")),
            ("OPENAI_API_KEY", OsString::from("sk-test-secret")),
            ("ANTHROPIC_API_KEY", OsString::from("anthropic-secret")),
            ("ANTHROPIC_OAUTH_TOKEN", OsString::from("oauth-secret")),
            ("ASTEREL_API_KEY", OsString::from("generic-secret")),
            ("GITHUB_TOKEN", OsString::from("github-secret")),
        ]);

        let child_env = CodexCliProvider::child_environment_from(|key| fake_env.get(key).cloned());
        let child_keys = child_env.iter().map(|(key, _)| *key).collect::<Vec<_>>();

        assert!(child_keys.contains(&"PATH"));
        assert!(child_keys.contains(&"CODEX_HOME"));
        assert!(child_keys.contains(&"SSL_CERT_FILE"));
        for secret_key in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_OAUTH_TOKEN",
            "ASTEREL_API_KEY",
            "GITHUB_TOKEN",
        ] {
            assert!(
                !child_keys
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(secret_key))
            );
            assert!(
                !CODEX_CLI_ENV_ALLOWLIST
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(secret_key))
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn chat_with_system_does_not_expose_parent_api_secret_env() {
        let _env_lock = ENV_LOCK.lock().expect("lock env");
        let _openai_key = EnvVarGuard::set("OPENAI_API_KEY", "SHOULD_NOT_REACH_CODEX_CLI");
        let dir = write_stub(
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output-last-message)
      out="$2"
      shift 2
      ;;
    -m|--model|--sandbox|-a|--ask-for-approval|--color|-C|--cd)
      shift 2
      ;;
    exec|-|--skip-git-repo-check|--ephemeral)
      shift
      ;;
    *)
      shift
      ;;
  esac
done
cat >/dev/null
if [ "${OPENAI_API_KEY:-}" = "SHOULD_NOT_REACH_CODEX_CLI" ]; then
  printf 'LEAKED_SECRET_ENV' > "$out"
else
  printf 'NO_SECRET_ENV' > "$out"
fi
"#,
        );
        let provider = CodexCliProvider::with_executable(
            dir.path().join("codex-stub.sh").display().to_string(),
            None,
        );

        let response = provider
            .chat_with_system_full(Some("system"), "message", "gpt-5.4", 0.0)
            .await
            .expect("codex cli response");

        assert_eq!(response.text, "NO_SECRET_ENV");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn chat_with_system_reads_last_message_file() {
        let dir = write_stub(
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output-last-message)
      out="$2"
      shift 2
      ;;
    -m|--model|--sandbox|-a|--ask-for-approval|--color|-C|--cd)
      shift 2
      ;;
    exec|-|--skip-git-repo-check|--ephemeral)
      shift
      ;;
    *)
      shift
      ;;
  esac
done
cat >/dev/null
printf 'CLI_TEST_OK' > "$out"
"#,
        );
        let provider = CodexCliProvider::with_executable(
            dir.path().join("codex-stub.sh").display().to_string(),
            None,
        );

        let response = provider
            .chat_with_system_full(Some("system"), "message", "gpt-5.4", 0.0)
            .await
            .expect("codex cli response");

        assert_eq!(response.text, "CLI_TEST_OK");
        assert_eq!(response.model.as_deref(), Some("gpt-5.4"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn chat_with_tools_uses_fallback_tool_parsing() {
        let dir = write_stub(
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output-last-message)
      out="$2"
      shift 2
      ;;
    -m|--model|--sandbox|-a|--ask-for-approval|--color|-C|--cd)
      shift 2
      ;;
    exec|-|--skip-git-repo-check|--ephemeral)
      shift
      ;;
    *)
      shift
      ;;
  esac
done
cat >/dev/null
printf '<tool_call>{"name":"shell","arguments":{"command":"ls"}}</tool_call>' > "$out"
"#,
        );
        let provider = CodexCliProvider::with_executable(
            dir.path().join("codex-stub.sh").display().to_string(),
            None,
        );
        let messages = vec![crate::core::providers::ProviderMessage {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: "list files".to_string(),
            }],
        }];
        let tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "Execute a shell command".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            }),
            required_capabilities: Vec::new(),
            effect: crate::contracts::tools::ToolEffect::LocalMutation,
        }];

        let response = provider
            .chat_with_tools(None, &messages, &tools, "gpt-5.4", 0.0)
            .await
            .expect("fallback tool response");

        assert_eq!(response.stop_reason, Some(StopReason::ToolUse));
        assert!(matches!(
            response.content_blocks[0],
            ContentBlock::ToolUse { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn chat_with_system_times_out_hung_codex_process() {
        let dir = write_stub(
            r#"#!/bin/sh
cat >/dev/null
sleep 5
"#,
        );
        let provider = CodexCliProvider::with_executable_timeout(
            dir.path().join("codex-stub.sh").display().to_string(),
            None,
            Duration::from_millis(50),
        );

        let err = provider
            .chat_with_system_full(Some("system"), "message", "gpt-5.4", 0.0)
            .await
            .expect_err("hung codex process should time out");

        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn missing_executable_maps_to_missing_credentials() {
        let provider = CodexCliProvider::with_executable("/definitely/missing/codex", None);
        let err = provider
            .chat_with_system(None, "message", "gpt-5.4", 0.0)
            .await
            .expect_err("missing executable should fail");
        assert!(err.to_string().contains("credentials not configured"));
    }
}
