//! `BrowserTool` implementation — dispatches browser actions via the `agent-browser` CLI.
//!
//! This module contains the `BrowserTool` struct, its URL validation logic,
//! the CLI argument builder (`browser_action_args`), and the `Tool` impl.
//! See the `mod.rs` docs for the full security model and availability check.

use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::{Duration, timeout};
use tracing::debug;

use super::domain::{extract_host, host_in_allowlist, is_private_host, normalize_domains};
use super::types::{AgentBrowserResponse, BrowserAction};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{
    Tool, ToolResult, ToolResultCompactionTarget, ToolResultTextField,
};
use crate::security::SecurityPolicy;
use crate::security::url_validation::validate_no_ssrf;

/// Browser automation tool backed by the `agent-browser` CLI.
///
/// Holds the security policy (for autonomy and rate-limit checks), the
/// normalized domain allowlist, and an optional persistent session name
/// passed to `agent-browser --session`.
pub struct BrowserTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    session_name: Option<String>,
}

const AGENT_BROWSER_COMMAND_TIMEOUT_SECS: u64 = 45;

fn parse_u32_arg(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
}

fn required_arg<'a>(args: &'a Value, key: &str, action: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing '{key}' for {action}"))
}

fn parse_browser_action(action: &str, args: &Value) -> anyhow::Result<BrowserAction> {
    let parsed = match action {
        "open" => BrowserAction::Open {
            url: required_arg(args, "url", "open action")?.into(),
        },
        "snapshot" => BrowserAction::Snapshot {
            interactive_only: args
                .get("interactive_only")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            compact: args
                .get("compact")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            depth: parse_u32_arg(args, "depth"),
        },
        "click" => BrowserAction::Click {
            selector: required_arg(args, "selector", "click")?.into(),
        },
        "fill" => BrowserAction::Fill {
            selector: required_arg(args, "selector", "fill")?.into(),
            value: required_arg(args, "value", "fill")?.into(),
        },
        "type" => BrowserAction::Type {
            selector: required_arg(args, "selector", "type")?.into(),
            text: required_arg(args, "text", "type")?.into(),
        },
        "get_text" => BrowserAction::GetText {
            selector: required_arg(args, "selector", "get_text")?.into(),
        },
        "get_title" => BrowserAction::GetTitle,
        "get_url" => BrowserAction::GetUrl,
        "screenshot" => BrowserAction::Screenshot {
            path: args
                .get("path")
                .and_then(|value| value.as_str())
                .map(String::from),
            full_page: args
                .get("full_page")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        },
        "wait" => BrowserAction::Wait {
            selector: args
                .get("selector")
                .and_then(|value| value.as_str())
                .map(String::from),
            ms: args.get("ms").and_then(serde_json::Value::as_u64),
            text: args
                .get("text")
                .and_then(|value| value.as_str())
                .map(String::from),
        },
        "press" => BrowserAction::Press {
            key: required_arg(args, "key", "press")?.into(),
        },
        "hover" => BrowserAction::Hover {
            selector: required_arg(args, "selector", "hover")?.into(),
        },
        "scroll" => BrowserAction::Scroll {
            direction: required_arg(args, "direction", "scroll")?.into(),
            pixels: parse_u32_arg(args, "pixels"),
        },
        "is_visible" => BrowserAction::IsVisible {
            selector: required_arg(args, "selector", "is_visible")?.into(),
        },
        "close" => BrowserAction::Close,
        "find" => BrowserAction::Find {
            by: required_arg(args, "by", "find")?.into(),
            value: required_arg(args, "value", "find")?.into(),
            action: required_arg(args, "find_action", "find")?.into(),
            fill_value: args
                .get("fill_value")
                .and_then(|value| value.as_str())
                .map(String::from),
        },
        _ => anyhow::bail!("Unknown action: {action}"),
    };

    Ok(parsed)
}

fn format_browser_output(data: Option<Value>) -> String {
    data.map(|value| serde_json::to_string_pretty(&value).unwrap_or_default())
        .unwrap_or_default()
}

fn browser_semantic_output_kind(action: &BrowserAction) -> Option<&'static str> {
    match action {
        BrowserAction::Snapshot { .. } => Some("browser.snapshot"),
        BrowserAction::Find { .. } => Some("browser.find"),
        _ => None,
    }
}

fn browser_action_may_navigate(action: &BrowserAction) -> bool {
    match action {
        BrowserAction::Click { .. } | BrowserAction::Press { .. } => true,
        BrowserAction::Find { action, .. } => matches!(action.as_str(), "click"),
        _ => false,
    }
}

fn current_url_from_response(resp: &AgentBrowserResponse) -> Option<&str> {
    let data = resp.data.as_ref()?;
    data.as_str().or_else(|| {
        data.get("url")
            .or_else(|| data.get("current_url"))
            .or_else(|| data.get("href"))
            .and_then(serde_json::Value::as_str)
    })
}

fn is_known_browser_action(action: &str) -> bool {
    matches!(
        action,
        "open"
            | "snapshot"
            | "click"
            | "fill"
            | "type"
            | "get_text"
            | "get_title"
            | "get_url"
            | "screenshot"
            | "wait"
            | "press"
            | "hover"
            | "scroll"
            | "is_visible"
            | "close"
            | "find"
    )
}

fn browser_action_args(action: &BrowserAction) -> Vec<String> {
    match action {
        BrowserAction::Open { url } => vec!["open".to_string(), url.clone()],
        BrowserAction::Snapshot {
            interactive_only,
            compact,
            depth,
        } => {
            let mut args = vec!["snapshot".to_string()];
            if *interactive_only {
                args.push("-i".to_string());
            }
            if *compact {
                args.push("-c".to_string());
            }
            if let Some(depth) = depth {
                args.push("-d".to_string());
                args.push(depth.to_string());
            }
            args
        }
        BrowserAction::Click { selector } => vec!["click".to_string(), selector.clone()],
        BrowserAction::Fill { selector, value } => {
            vec!["fill".to_string(), selector.clone(), value.clone()]
        }
        BrowserAction::Type { selector, text } => {
            vec!["type".to_string(), selector.clone(), text.clone()]
        }
        BrowserAction::GetText { selector } => {
            vec!["get".to_string(), "text".to_string(), selector.clone()]
        }
        BrowserAction::GetTitle => vec!["get".to_string(), "title".to_string()],
        BrowserAction::GetUrl => vec!["get".to_string(), "url".to_string()],
        BrowserAction::Screenshot { path, full_page } => {
            let mut args = vec!["screenshot".to_string()];
            if let Some(path) = path {
                args.push(path.clone());
            }
            if *full_page {
                args.push("--full".to_string());
            }
            args
        }
        BrowserAction::Wait { selector, ms, text } => {
            let mut args = vec!["wait".to_string()];
            if let Some(selector) = selector {
                args.push(selector.clone());
            } else if let Some(ms) = ms {
                args.push(ms.to_string());
            } else if let Some(text) = text {
                args.push("--text".to_string());
                args.push(text.clone());
            }
            args
        }
        BrowserAction::Press { key } => vec!["press".to_string(), key.clone()],
        BrowserAction::Hover { selector } => vec!["hover".to_string(), selector.clone()],
        BrowserAction::Scroll { direction, pixels } => {
            let mut args = vec!["scroll".to_string(), direction.clone()];
            if let Some(pixels) = pixels {
                args.push(pixels.to_string());
            }
            args
        }
        BrowserAction::IsVisible { selector } => {
            vec!["is".to_string(), "visible".to_string(), selector.clone()]
        }
        BrowserAction::Close => vec!["close".to_string()],
        BrowserAction::Find {
            by,
            value,
            action,
            fill_value,
        } => {
            let mut args = vec![
                "find".to_string(),
                by.clone(),
                value.clone(),
                action.clone(),
            ];
            if let Some(fill_value) = fill_value {
                args.push(fill_value.clone());
            }
            args
        }
    }
}

impl BrowserTool {
    /// Create a new browser tool with the given security policy,
    /// domain allowlist, and optional persistent session name.
    #[must_use]
    pub fn new(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        session_name: Option<String>,
    ) -> Self {
        Self {
            security,
            allowed_domains: normalize_domains(allowed_domains),
            session_name,
        }
    }

    /// Check if agent-browser CLI is available (result is cached for the
    /// process lifetime since binary availability doesn't change at runtime).
    pub async fn is_available() -> bool {
        use tokio::sync::OnceCell;
        static AVAILABLE: OnceCell<bool> = OnceCell::const_new();
        *AVAILABLE
            .get_or_init(|| async {
                Command::new("agent-browser")
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
            .await
    }

    /// Validate URL against allowlist
    pub(super) async fn validate_url(&self, url: &str) -> anyhow::Result<()> {
        let url = url.trim();

        if url.is_empty() {
            anyhow::bail!(
                "URL cannot be empty: provide a fully-qualified URL starting with http:// or https://"
            );
        }

        if url.starts_with("file://") {
            anyhow::bail!("file:// URLs are blocked by default");
        }

        if !url.starts_with("https://") && !url.starts_with("http://") {
            anyhow::bail!("Only http:// and https:// URLs are allowed");
        }

        if self.allowed_domains.is_empty() {
            anyhow::bail!(
                "Browser tool enabled but no allowed_domains configured. \
                Add [browser].allowed_domains in config.toml"
            );
        }

        let host = extract_host(url)?;

        if is_private_host(&host) {
            anyhow::bail!("Blocked local/private host: {host}");
        }

        if !host_in_allowlist(&host, &self.allowed_domains) {
            anyhow::bail!("Host '{host}' not in browser.allowed_domains");
        }

        validate_no_ssrf(url).await?;

        Ok(())
    }

    /// Execute an agent-browser command
    async fn run_command(&self, args: &[&str]) -> anyhow::Result<AgentBrowserResponse> {
        let mut cmd = Command::new("agent-browser");

        // Add session if configured
        if let Some(ref session) = self.session_name {
            cmd.arg("--session").arg(session);
        }

        // Add --json for machine-readable output
        cmd.args(args).arg("--json");
        // Ensure timed-out runs don't leave orphan child processes.
        cmd.kill_on_drop(true);

        debug!("Running: agent-browser {} --json", args.join(" "));

        let output = timeout(
            Duration::from_secs(AGENT_BROWSER_COMMAND_TIMEOUT_SECS),
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "agent-browser command timed out after {AGENT_BROWSER_COMMAND_TIMEOUT_SECS}s"
            )
        })??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !stderr.is_empty() {
            debug!("agent-browser stderr: {}", stderr);
        }

        // Parse JSON response
        if let Ok(resp) = serde_json::from_str::<AgentBrowserResponse>(&stdout) {
            return Ok(resp);
        }

        // Fallback for non-JSON output
        if output.status.success() {
            Ok(AgentBrowserResponse {
                success: true,
                data: Some(json!({ "output": stdout.trim() })),
                error: None,
            })
        } else {
            Ok(AgentBrowserResponse {
                success: false,
                data: None,
                error: Some(stderr.trim().to_string()),
            })
        }
    }

    /// Execute a browser action
    async fn execute_action(&self, action: BrowserAction) -> anyhow::Result<ToolResult> {
        if let BrowserAction::Open { url } = &action {
            self.validate_url(url).await?;
        }

        let args = browser_action_args(&action);
        let command_args: Vec<&str> = args.iter().map(String::as_str).collect();
        let response = self.run_command(&command_args).await?;
        if response.success && browser_action_may_navigate(&action) {
            self.verify_current_url_after_navigation().await?;
        }
        Ok(Self::to_result(
            response,
            browser_semantic_output_kind(&action),
        ))
    }

    async fn verify_current_url_after_navigation(&self) -> anyhow::Result<()> {
        let response = self.run_command(&["get_url"]).await?;
        if !response.success {
            let error = response
                .error
                .unwrap_or_else(|| "agent-browser did not return current URL".to_string());
            anyhow::bail!("could not verify browser session URL after navigation: {error}");
        }
        let current_url = current_url_from_response(&response).ok_or_else(|| {
            anyhow::anyhow!("could not verify browser session URL after navigation: missing url")
        })?;
        if let Err(error) = self.validate_url(current_url).await {
            let _ = self.run_command(&["close"]).await;
            anyhow::bail!("browser navigation left allowed domains: {error}");
        }
        Ok(())
    }

    fn to_result(resp: AgentBrowserResponse, output_kind: Option<&'static str>) -> ToolResult {
        if resp.success {
            let output = format_browser_output(resp.data);
            let result = ToolResult::success(output);
            match output_kind {
                Some(output_kind) => result
                    .with_output_kind(output_kind)
                    .with_compaction_target(ToolResultCompactionTarget::Output)
                    .with_source_fields([ToolResultTextField::Output]),
                None => result,
            }
        } else {
            ToolResult {
                success: false,
                output: String::new(),
                error: resp.error,

                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            }
        }
    }
}

impl Tool for BrowserTool {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn description(&self) -> &'static str {
        "Web browser automation using agent-browser. Supports navigation, clicking, \
        filling forms, taking screenshots, and getting accessibility snapshots with refs. \
        Use 'snapshot' to get interactive elements with refs (@e1, @e2), then use refs \
        for precise element interaction. Allowed domains only."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "snapshot", "click", "fill", "type", "get_text",
                             "get_title", "get_url", "screenshot", "wait", "press",
                             "hover", "scroll", "is_visible", "close", "find"],
                    "description": "Browser action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for 'open' action)"
                },
                "selector": {
                    "type": "string",
                    "description": "Element selector: @ref (e.g. @e1), CSS (#id, .class), or text=..."
                },
                "value": {
                    "type": "string",
                    "description": "Value to fill or type"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type or wait for"
                },
                "key": {
                    "type": "string",
                    "description": "Key to press (Enter, Tab, Escape, etc.)"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction"
                },
                "pixels": {
                    "type": "integer",
                    "description": "Pixels to scroll"
                },
                "interactive_only": {
                    "type": "boolean",
                    "description": "For snapshot: only show interactive elements"
                },
                "compact": {
                    "type": "boolean",
                    "description": "For snapshot: remove empty structural elements"
                },
                "depth": {
                    "type": "integer",
                    "description": "For snapshot: limit tree depth"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "For screenshot: capture full page"
                },
                "path": {
                    "type": "string",
                    "description": "File path for screenshot"
                },
                "ms": {
                    "type": "integer",
                    "description": "Milliseconds to wait"
                },
                "by": {
                    "type": "string",
                    "enum": ["role", "text", "label", "placeholder", "testid"],
                    "description": "For find: semantic locator type"
                },
                "find_action": {
                    "type": "string",
                    "enum": ["click", "fill", "text", "hover", "check"],
                    "description": "For find: action to perform on found element"
                },
                "fill_value": {
                    "type": "string",
                    "description": "For find with fill action: value to fill"
                }
            },
            "required": ["action"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            if !self.security.can_act() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Action blocked: autonomy is read-only".into()),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            if !self.security.record_action() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Action blocked: rate limit exceeded".into()),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            // Check if agent-browser is available
            if !Self::is_available().await {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(
                        "agent-browser CLI not found. Install with: npm install -g agent-browser"
                            .into(),
                    ),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

            match parse_browser_action(action, &args) {
                Ok(parsed) => self.execute_action(parsed).await,
                Err(_) if !is_known_browser_action(action) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Unknown action: {action}")),
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
                Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AgentBrowserResponse, BrowserAction, BrowserTool};

    #[test]
    fn snapshot_success_emits_semantic_metadata() {
        let result = BrowserTool::to_result(
            AgentBrowserResponse {
                success: true,
                data: Some(json!({
                    "snapshot": "- heading \"Example\" [ref=e1]",
                    "refs": { "e1": { "role": "heading", "name": "Example" } }
                })),
                error: None,
            },
            Some("browser.snapshot"),
        );

        assert!(result.success);
        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("browser.snapshot")
        );
    }

    #[test]
    fn helper_maps_only_supported_actions() {
        assert_eq!(
            super::browser_semantic_output_kind(&BrowserAction::Snapshot {
                interactive_only: true,
                compact: true,
                depth: Some(2),
            }),
            Some("browser.snapshot")
        );
        assert_eq!(
            super::browser_semantic_output_kind(&BrowserAction::Find {
                by: "role".to_string(),
                value: "button".to_string(),
                action: "click".to_string(),
                fill_value: None,
            }),
            Some("browser.find")
        );
        assert_eq!(
            super::browser_semantic_output_kind(&BrowserAction::Open {
                url: "https://example.com".to_string(),
            }),
            None
        );
    }

    #[test]
    fn helper_marks_only_navigation_capable_actions_for_post_check() {
        assert!(super::browser_action_may_navigate(&BrowserAction::Click {
            selector: "@e1".to_string(),
        }));
        assert!(super::browser_action_may_navigate(&BrowserAction::Find {
            by: "role".to_string(),
            value: "link".to_string(),
            action: "click".to_string(),
            fill_value: None,
        }));
        assert!(!super::browser_action_may_navigate(&BrowserAction::Find {
            by: "role".to_string(),
            value: "textbox".to_string(),
            action: "text".to_string(),
            fill_value: None,
        }));
        assert!(!super::browser_action_may_navigate(
            &BrowserAction::Snapshot {
                interactive_only: true,
                compact: true,
                depth: None,
            }
        ));
    }

    #[test]
    fn helper_extracts_current_url_from_agent_browser_response() {
        let response = AgentBrowserResponse {
            success: true,
            data: Some(json!({ "current_url": "https://example.com/after-click" })),
            error: None,
        };

        assert_eq!(
            super::current_url_from_response(&response),
            Some("https://example.com/after-click")
        );
    }
}
