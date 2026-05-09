//! External hook execution pipeline (WP-G2).
//!
//! Hooks are shell commands configured in `hooks.toml` (or loaded from a
//! [`HookConfigSet`]) that fire before and/or after tool invocations.  They
//! allow operators to plug in custom policy scripts without modifying
//! `Asterel` source code.
//!
//! # Communication protocol
//!
//! The hook subprocess receives a JSON-serialised [`HookPayload`] on stdin.
//! It communicates its decision via exit code and optional JSON on stdout:
//!
//! | Exit code | Meaning |
//! |-----------|---------|
//! | `0` | Allow (stdout may contain a [`HookResponse`] with optional overrides) |
//! | `2` | Deny (hard block — stdout is ignored) |
//! | other | Hook error (logged at WARN, does **not** block — fail-open) |
//!
//! A [`HookResponse`] with `decision: null` (or empty stdout on exit 0) means
//! the hook ran successfully but does not override the default decision.
//!
//! # Abort signal
//!
//! [`HookAbortSignal`] allows the UI layer to cancel a running hook subprocess
//! (e.g. if the user presses "Cancel" while a hook is waiting for input).
//!
//! # Design lineage
//!
//! Modelled after the `claude-code` `HookRunner`, `oh-my-openagent`
//! `safeCreateHook`, and `OpenHarness` `PromptHook` patterns identified in
//! the 2026-04-03 ecosystem survey.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use super::hook_types::{
    HookConfig, HookConfigSet, HookDecision, HookEvent, HookPayload, HookResponse,
};
use super::{ActionIntent, ExecutionContext, MiddlewareDecision, ToolMiddleware, ToolResult};

/// Cancellation signal shared between the hook runner and the caller (e.g. the UI layer).
///
/// Clone this cheaply — all clones share the same underlying `AtomicBool`.
/// Call [`HookAbortSignal::abort`] to request cancellation; the hook runner
/// checks [`HookAbortSignal::is_aborted`] between subprocess steps and after
/// the subprocess exits.
#[derive(Debug, Clone)]
pub struct HookAbortSignal(Arc<AtomicBool>);

impl HookAbortSignal {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn abort(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for HookAbortSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// Middleware (position 2) that runs external shell hooks around tool execution.
///
/// Hooks are evaluated in configuration order.  The first hook that returns a
/// non-`None` [`HookDecision`] short-circuits the remaining hooks for that
/// event.  Hook errors are logged at `WARN` and do not block execution
/// (fail-open semantics).
#[derive(Debug)]
pub struct HookMiddleware {
    configs: HookConfigSet,
    abort_signal: HookAbortSignal,
}

impl HookMiddleware {
    /// Create a new hook middleware with the given config set and abort signal.
    #[must_use]
    pub fn new(configs: HookConfigSet, abort_signal: HookAbortSignal) -> Self {
        Self {
            configs,
            abort_signal,
        }
    }

    /// Run all hooks registered for `event` and return the first override decision.
    ///
    /// Returns `Ok(None)` if no hook overrides the default decision.
    async fn run_hooks(
        &self,
        event: HookEvent,
        payload: &HookPayload,
    ) -> Result<Option<(HookDecision, Option<Value>)>> {
        for config in &self.configs.hooks {
            if !config.enabled || !config.events.contains(&event) {
                continue;
            }

            if self.abort_signal.is_aborted() {
                tracing::info!("hook aborted by signal before execution");
                return Ok(None);
            }

            match run_single_hook(config, payload, &self.abort_signal).await {
                Ok(Some(response)) => {
                    if let Some(decision) = response.decision {
                        return Ok(Some((decision, response.updated_input)));
                    }
                    // Hook ran but did not override — continue to next hook
                }
                Ok(None) => {
                    // Hook denied via exit code 2
                    return Ok(Some((HookDecision::Deny, None)));
                }
                Err(error) => {
                    // Hook error — log but don't block (fail-open for hook errors)
                    tracing::warn!(%error, command = %config.command, "hook execution failed, continuing");
                }
            }
        }
        Ok(None)
    }
}

impl ToolMiddleware for HookMiddleware {
    fn runs_after_pre_execution_return(&self) -> bool {
        false
    }

    fn before_execute<'a>(
        &'a self,
        tool_name: &'a str,
        args: &'a Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            let payload = HookPayload {
                event: HookEvent::PreToolUse,
                tool_name: tool_name.to_string(),
                args: args.clone(),
                session_id: ctx.session_id.as_deref().unwrap_or("").into(),
                entity_id: ctx.entity_id.clone(),
            };

            match self.run_hooks(HookEvent::PreToolUse, &payload).await? {
                Some((HookDecision::Deny, _)) => Ok(MiddlewareDecision::Block(
                    "blocked by pre-tool-use hook".to_string(),
                )),
                Some((HookDecision::Ask, _)) => {
                    Ok(MiddlewareDecision::RequireApproval(ActionIntent::new(
                        tool_name,
                        ctx.entity_id.as_str(),
                        serde_json::json!({
                            "tool": tool_name,
                            "reason": "hook requires approval",
                        }),
                    )))
                }
                Some((HookDecision::Allow, _)) | None => Ok(MiddlewareDecision::Continue),
            }
        })
    }

    fn after_execute<'a>(
        &'a self,
        tool_name: &'a str,
        _result: &'a mut ToolResult,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let payload = HookPayload {
                event: HookEvent::PostToolUse,
                tool_name: tool_name.to_string(),
                args: Value::Null,
                session_id: ctx.session_id.as_deref().unwrap_or("").into(),
                entity_id: ctx.entity_id.clone(),
            };

            if let Err(error) = self.run_hooks(HookEvent::PostToolUse, &payload).await {
                tracing::warn!(%error, tool_name, "post-tool-use hook failed");
            }
        })
    }
}

/// Run a single hook subprocess and parse its response.
///
/// # Returns
///
/// - `Ok(Some(response))` — hook exited `0`; stdout parsed as [`HookResponse`]
///   (or an empty response if stdout is blank or not valid JSON).
/// - `Ok(None)` — hook exited `2` (deny signal).
/// - `Err(...)` — hook error: non-`0`/`2` exit code, spawn failure, or timeout.
async fn run_single_hook(
    config: &HookConfig,
    payload: &HookPayload,
    abort: &HookAbortSignal,
) -> Result<Option<HookResponse>> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let payload_json = serde_json::to_string(payload)
        .map_err(|e| anyhow::anyhow!("serialize hook payload: {e}"))?;

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&config.command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn hook `{}`: {e}", config.command))?;

    // Deliver the JSON payload and close stdin so the subprocess can read EOF.
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(error) = stdin.write_all(payload_json.as_bytes()).await
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(error.into());
        }
        drop(stdin);
    }

    // Await the subprocess with the configured timeout.
    let timeout = Duration::from_secs(config.timeout_secs);
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => anyhow::bail!("hook `{}` wait failed: {e}", config.command),
        Err(_) => anyhow::bail!(
            "hook `{}` timed out after {}s",
            config.command,
            config.timeout_secs
        ),
    };

    if abort.is_aborted() {
        anyhow::bail!("hook `{}` aborted by signal", config.command);
    }

    let code = output.status.code().unwrap_or(-1);
    match code {
        0 => {
            // Parse stdout as a HookResponse.  Blank or invalid JSON is
            // treated as "no override" rather than an error (fail-open).
            let stdout = String::from_utf8_lossy(&output.stdout);
            let response = if stdout.trim().is_empty() {
                HookResponse {
                    decision: None,
                    system_message: None,
                    updated_input: None,
                }
            } else {
                serde_json::from_str(stdout.trim()).unwrap_or(HookResponse {
                    decision: None,
                    system_message: None,
                    updated_input: None,
                })
            };
            Ok(Some(response))
        }
        2 => Ok(None), // exit 2 = deny signal
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "hook `{}` exited with code {code}: {stderr}",
                config.command
            );
        }
    }
}

/// Poll-based async helper that resolves once the abort signal is set.
///
/// Not currently used in the hot path (the abort signal is checked
/// synchronously between subprocess steps), but retained for potential
/// future use in select! patterns.
// TODO(hooks): use in tokio::select! for async hook steps that need concurrent abort monitoring.
#[allow(dead_code)]
async fn wait_for_abort(abort: &HookAbortSignal) {
    loop {
        if abort.is_aborted() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_signal_starts_false() {
        let signal = HookAbortSignal::new();
        assert!(!signal.is_aborted());
    }

    #[test]
    fn abort_signal_can_be_set() {
        let signal = HookAbortSignal::new();
        signal.abort();
        assert!(signal.is_aborted());
    }

    #[test]
    fn hook_payload_serialization() {
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            tool_name: "shell".to_string(),
            args: serde_json::json!({"command": "ls"}),
            session_id: crate::contracts::ids::SessionId::new("sess-1"),
            entity_id: crate::contracts::ids::EntityId::new("person:local"),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("pre_tool_use"));
        assert!(json.contains("shell"));
    }

    #[test]
    fn hook_response_deserialization() {
        let json = r#"{"decision": "deny", "system_message": "blocked by policy"}"#;
        let response: HookResponse = serde_json::from_str(json).expect("deserialize");
        assert_eq!(response.decision, Some(HookDecision::Deny));
        assert_eq!(
            response.system_message.as_deref(),
            Some("blocked by policy")
        );
    }

    #[test]
    fn empty_hook_response() {
        let json = "{}";
        let response: HookResponse = serde_json::from_str(json).expect("deserialize");
        assert!(response.decision.is_none());
    }

    #[test]
    fn hook_config_toml_roundtrip() {
        let toml = r#"
[[hooks]]
command = "python3 check_policy.py"
events = ["pre_tool_use"]
timeout_secs = 5

[[hooks]]
command = "bash audit_log.sh"
events = ["post_tool_use", "post_tool_use_failure"]
"#;
        let config: HookConfigSet = toml::from_str(toml).expect("parse");
        assert_eq!(config.hooks.len(), 2);
        assert_eq!(config.hooks[0].timeout_secs, 5);
        assert!(config.hooks[1].enabled); // default true
    }

    #[tokio::test]
    async fn hook_exit_0_returns_no_override() {
        let config = HookConfig {
            command: "echo '{}'".to_string(),
            events: vec![HookEvent::PreToolUse],
            timeout_secs: 5,
            enabled: true,
        };
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            tool_name: "test".to_string(),
            args: Value::Null,
            session_id: crate::contracts::ids::SessionId::new(""),
            entity_id: crate::contracts::ids::EntityId::new(""),
        };
        let result = run_single_hook(&config, &payload, &HookAbortSignal::new()).await;
        let response = result.expect("should succeed");
        let response = response.expect("exit 0 should return Some");
        assert!(
            response.decision.is_none(),
            "empty JSON means no override, not explicit allow"
        );
    }

    #[tokio::test]
    async fn hook_malformed_json_falls_back_to_no_override() {
        let config = HookConfig {
            command: "cat >/dev/null; echo 'not valid json'".to_string(),
            events: vec![HookEvent::PreToolUse],
            timeout_secs: 5,
            enabled: true,
        };
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            tool_name: "test".to_string(),
            args: Value::Null,
            session_id: crate::contracts::ids::SessionId::new(""),
            entity_id: crate::contracts::ids::EntityId::new(""),
        };
        let result = run_single_hook(&config, &payload, &HookAbortSignal::new()).await;
        let response = result.expect("should succeed despite malformed JSON");
        let response = response.expect("exit 0 should return Some");
        assert!(
            response.decision.is_none(),
            "malformed JSON should fall back to empty response"
        );
    }

    #[tokio::test]
    async fn hook_exit_2_denies() {
        let config = HookConfig {
            command: "exit 2".to_string(),
            events: vec![HookEvent::PreToolUse],
            timeout_secs: 5,
            enabled: true,
        };
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            tool_name: "test".to_string(),
            args: Value::Null,
            session_id: crate::contracts::ids::SessionId::new(""),
            entity_id: crate::contracts::ids::EntityId::new(""),
        };
        let result = run_single_hook(&config, &payload, &HookAbortSignal::new()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none()); // exit 2 = None (deny)
    }

    #[tokio::test]
    async fn hook_exit_1_errors() {
        let config = HookConfig {
            command: "exit 1".to_string(),
            events: vec![HookEvent::PreToolUse],
            timeout_secs: 5,
            enabled: true,
        };
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            tool_name: "test".to_string(),
            args: Value::Null,
            session_id: crate::contracts::ids::SessionId::new(""),
            entity_id: crate::contracts::ids::EntityId::new(""),
        };
        let result = run_single_hook(&config, &payload, &HookAbortSignal::new()).await;
        assert!(result.is_err()); // non-0/2 = error
    }

    #[tokio::test]
    async fn hook_with_json_decision() {
        let config = HookConfig {
            command: r#"echo '{"decision": "ask", "system_message": "please confirm"}'"#
                .to_string(),
            events: vec![HookEvent::PreToolUse],
            timeout_secs: 5,
            enabled: true,
        };
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            tool_name: "shell".to_string(),
            args: Value::Null,
            session_id: crate::contracts::ids::SessionId::new(""),
            entity_id: crate::contracts::ids::EntityId::new(""),
        };
        let result = run_single_hook(&config, &payload, &HookAbortSignal::new())
            .await
            .expect("should succeed");
        let response = result.expect("should have response");
        assert_eq!(response.decision, Some(HookDecision::Ask));
        assert_eq!(response.system_message.as_deref(), Some("please confirm"));
    }
}
