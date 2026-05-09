//! Composio tool provider: optional managed surface with 1000+
//! OAuth integrations.
//!
//! When enabled, executes actions on Gmail, Notion, GitHub, Slack,
//! etc. through Composio's API without storing raw OAuth tokens
//! locally. Opt-in only; the API key lives in the secret store.

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::traits::{Tool, ToolResult};
use crate::core::tools::middleware::ExecutionContext;

const COMPOSIO_API_BASE: &str = "https://backend.composio.dev/api/v2";

/// A tool that proxies actions to the Composio managed tool platform.
pub struct ComposioTool {
    api_key: String,
    client: Client,
    api_base: String,
}

fn composio_parameters_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "description": "The operation: 'list' (list available actions), 'execute' (run an action), or 'connect' (get OAuth URL)",
                "enum": ["list", "execute", "connect"]
            },
            "app": {
                "type": "string",
                "description": "App name filter for 'list', or app name for 'connect' (e.g. 'gmail', 'notion', 'github')"
            },
            "action_name": {
                "type": "string",
                "description": "The Composio action name to execute (e.g. 'GMAIL_FETCH_EMAILS')"
            },
            "params": {
                "type": "object",
                "description": "Parameters to pass to the action"
            },
        },
        "required": ["action"]
    })
}

fn format_actions_output(actions: &[ComposioAction]) -> String {
    let total = actions.len();
    let mut summary = String::new();
    for action in actions.iter().take(20) {
        if !summary.is_empty() {
            summary.push('\n');
        }
        let _ = write!(
            summary,
            "- {} ({}): {}",
            action.name,
            action.app_name.as_deref().unwrap_or("?"),
            action.description.as_deref().unwrap_or("")
        );
    }
    if total > 20 {
        format!(
            "Found {total} available actions:\n{summary}\n... and {} more",
            total - 20
        )
    } else {
        format!("Found {total} available actions:\n{summary}")
    }
}

fn success_tool_result(output: String) -> ToolResult {
    ToolResult {
        success: true,
        output,
        error: None,
        attachments: Vec::new(),
        taint_labels: Vec::new(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}

fn error_tool_result(message: impl Into<String>) -> ToolResult {
    ToolResult {
        success: false,
        output: String::new(),
        error: Some(message.into()),
        attachments: Vec::new(),
        taint_labels: Vec::new(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}

impl ComposioTool {
    /// Create a new Composio tool with the given API key.
    #[must_use]
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            client: crate::utils::http::build_http_client_with(
                Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .connect_timeout(std::time::Duration::from_secs(10)),
            ),
            api_base: COMPOSIO_API_BASE.to_string(),
        }
    }

    #[cfg(test)]
    fn new_with_api_base(api_key: &str, api_base: impl Into<String>) -> Self {
        Self {
            api_key: api_key.to_string(),
            client: crate::utils::http::build_http_client_with(
                Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .connect_timeout(std::time::Duration::from_secs(10)),
            ),
            api_base: api_base.into(),
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}{}", self.api_base.trim_end_matches('/'), path)
    }

    /// List available Composio apps/actions for the authenticated user.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails, API status is non-success, or decoding fails.
    pub async fn list_actions(
        &self,
        app_name: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioAction>> {
        let mut url = self.api_url("/actions");
        if let Some(app) = app_name {
            url = format!("{url}?appNames={app}");
        }

        let resp = self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Composio API error: {err}");
        }

        let body: ComposioActionsResponse = resp.json().await?;
        Ok(body.items)
    }

    /// Execute a Composio action by name with given parameters.
    ///
    /// # Errors
    /// Returns an error if request building fails, API status is non-success, or decoding fails.
    pub async fn execute_action(
        &self,
        action_name: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = self.api_url(&format!("/actions/{action_name}/execute"));

        let mut body = json!({
            "input": params,
        });

        if let Some(entity) = entity_id {
            body["entityId"] = json!(entity);
        }

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Composio action execution failed: {err}");
        }

        let result: serde_json::Value = resp.json().await?;
        Ok(result)
    }

    /// Get the OAuth connection URL for a specific app.
    ///
    /// # Errors
    /// Returns an error if the request fails, API status is non-success, or URL is absent.
    pub async fn get_connection_url(
        &self,
        app_name: &str,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let url = self.api_url("/connectedAccounts");

        let body = json!({
            "integrationId": app_name,
            "entityId": entity_id,
        });

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get connection URL: {err}");
        }

        let result: serde_json::Value = resp.json().await?;
        result
            .get("redirectUrl")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in response"))
    }

    async fn handle_list_action(&self, app: Option<&str>) -> ToolResult {
        match self.list_actions(app).await {
            Ok(actions) => success_tool_result(format_actions_output(&actions)),
            Err(error) => error_tool_result(format!("Failed to list actions: {error}")),
        }
    }

    async fn handle_execute_action(
        &self,
        args: &serde_json::Value,
        entity_id: &str,
    ) -> anyhow::Result<ToolResult> {
        let action_name = args
            .get("action_name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action_name' for execute"))?;
        let params = args.get("params").cloned().unwrap_or(json!({}));

        let result = match self
            .execute_action(action_name, params, Some(entity_id))
            .await
        {
            Ok(value) => {
                let output =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{value:?}"));
                success_tool_result(output)
            }
            Err(error) => error_tool_result(format!("Action execution failed: {error}")),
        };
        Ok(result)
    }

    async fn handle_connect_action(
        &self,
        args: &serde_json::Value,
        entity_id: &str,
    ) -> anyhow::Result<ToolResult> {
        let app = args
            .get("app")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'app' for connect"))?;

        let result = match self.get_connection_url(app, entity_id).await {
            Ok(url) => success_tool_result(format!("Open this URL to connect {app}:\n{url}")),
            Err(error) => error_tool_result(format!("Failed to get connection URL: {error}")),
        };
        Ok(result)
    }
}

impl Tool for ComposioTool {
    fn name(&self) -> &'static str {
        "composio"
    }

    fn description(&self) -> &'static str {
        "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). \
         Use action='list' to see available actions, or action='execute' with action_name and params."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        composio_parameters_schema()
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

            let entity_id = ctx.entity_id.as_str();

            match action {
                "list" => Ok(self
                    .handle_list_action(args.get("app").and_then(|v| v.as_str()))
                    .await),
                "execute" => self.handle_execute_action(&args, entity_id).await,
                "connect" => self.handle_connect_action(&args, entity_id).await,
                _ => Ok(error_tool_result(format!(
                    "Unknown action '{action}'. Use 'list', 'execute', or 'connect'."
                ))),
            }
        })
    }
}

// ── API response types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ComposioActionsResponse {
    #[serde(default)]
    items: Vec<ComposioAction>,
}

/// Metadata for a single Composio-managed action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioAction {
    /// Unique action identifier (e.g. `"GMAIL_FETCH_EMAILS"`).
    pub name: String,
    /// Parent application name (e.g. `"gmail"`).
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    /// Human-readable description of the action.
    pub description: Option<String>,
    /// Whether the action is currently enabled.
    #[serde(default)]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::middleware::ExecutionContext;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Constructor ───────────────────────────────────────────

    #[test]
    fn composio_tool_has_correct_name() {
        let tool = ComposioTool::new("test-key");
        assert_eq!(tool.name(), "composio");
    }

    #[test]
    fn composio_tool_has_description() {
        let tool = ComposioTool::new("test-key");
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("1000+"));
    }

    #[test]
    fn composio_tool_schema_has_required_fields() {
        let tool = ComposioTool::new("test-key");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["action_name"].is_object());
        assert!(schema["properties"]["params"].is_object());
        assert!(schema["properties"]["app"].is_object());
        assert!(schema["properties"]["entity_id"].is_null());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("action")));
    }

    #[test]
    fn composio_tool_spec_roundtrip() {
        let tool = ComposioTool::new("test-key");
        let spec = tool.spec();
        assert_eq!(spec.name, "composio");
        assert!(spec.parameters.is_object());
    }

    // ── Execute validation ────────────────────────────────────

    #[tokio::test]
    async fn execute_missing_action_returns_error() {
        let tool = ComposioTool::new("test-key");
        let ctx = ExecutionContext::test_default(std::sync::Arc::new(
            crate::security::SecurityPolicy::default(),
        ));
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_unknown_action_returns_error() {
        let tool = ComposioTool::new("test-key");
        let ctx = ExecutionContext::test_default(std::sync::Arc::new(
            crate::security::SecurityPolicy::default(),
        ));
        let result = tool
            .execute(json!({"action": "unknown"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("Unknown action"));
    }

    #[tokio::test]
    async fn execute_without_action_name_returns_error() {
        let tool = ComposioTool::new("test-key");
        let ctx = ExecutionContext::test_default(std::sync::Arc::new(
            crate::security::SecurityPolicy::default(),
        ));
        let result = tool.execute(json!({"action": "execute"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_without_app_returns_error() {
        let tool = ComposioTool::new("test-key");
        let ctx = ExecutionContext::test_default(std::sync::Arc::new(
            crate::security::SecurityPolicy::default(),
        ));
        let result = tool.execute(json!({"action": "connect"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_uses_context_entity_id_not_model_supplied_entity() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/actions/TEST_ACTION/execute"))
            .and(body_string_contains(r#""entityId":"tenant-a:user-a""#))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;

        let tool = ComposioTool::new_with_api_base("test-key", server.uri());
        let ctx = ExecutionContext::test_default(std::sync::Arc::new(
            crate::security::SecurityPolicy::default(),
        ))
        .with_entity("tenant-a:user-a");

        let result = tool
            .execute(
                json!({
                    "action": "execute",
                    "action_name": "TEST_ACTION",
                    "entity_id": "tenant-b:user-b",
                    "params": {"message": "hello"}
                }),
                &ctx,
            )
            .await
            .expect("composio execution should return a tool result");

        assert!(result.success, "unexpected result: {result:?}");
    }

    // ── API response parsing ──────────────────────────────────

    #[test]
    fn composio_action_deserializes() {
        let json_str = r#"{"name": "GMAIL_FETCH_EMAILS", "appName": "gmail", "description": "Fetch emails", "enabled": true}"#;
        let action: ComposioAction = serde_json::from_str(json_str).unwrap();
        assert_eq!(action.name, "GMAIL_FETCH_EMAILS");
        assert_eq!(action.app_name.as_deref(), Some("gmail"));
        assert!(action.enabled);
    }

    #[test]
    fn composio_actions_response_deserializes() {
        let json_str = r#"{"items": [{"name": "TEST_ACTION", "appName": "test", "description": "A test", "enabled": true}]}"#;
        let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].name, "TEST_ACTION");
    }

    #[test]
    fn composio_actions_response_empty() {
        let json_str = r#"{"items": []}"#;
        let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
        assert!(resp.items.is_empty());
    }

    #[test]
    fn composio_actions_response_missing_items_defaults() {
        let json_str = r"{}";
        let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
        assert!(resp.items.is_empty());
    }
}
