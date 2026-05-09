//! MCP tool proxy: wraps a remote MCP server tool as a local
//! `Tool` impl, forwarding invocations over the MCP connection.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::{Tool, ToolResult};
use crate::plugins::mcp::client_connection::McpConnection;
use crate::plugins::mcp::content::{ToolContent, render_text};

const MCP_OUTPUT_TAINT_LABEL: &str = "external:mcp";

fn mcp_output_taint_labels() -> Vec<String> {
    vec![MCP_OUTPUT_TAINT_LABEL.to_string()]
}

/// Wraps a remote MCP server tool as a local `Tool` implementation.
pub(crate) struct McpToolProxy {
    tool_name: String,
    namespaced_name: String,
    description: String,
    parameters_schema: serde_json::Value,
    connection: Arc<McpConnection>,
}

impl McpToolProxy {
    /// Create a new proxy tool for the given MCP server tool.
    pub(crate) fn new(
        tool_name: impl Into<String>,
        description: impl Into<String>,
        parameters_schema: serde_json::Value,
        connection: Arc<McpConnection>,
        server_name: impl Into<String>,
    ) -> Self {
        let tool_name = tool_name.into();
        let server_name = server_name.into();
        let namespaced_name = format!("mcp_{server_name}_{tool_name}");

        Self {
            tool_name,
            namespaced_name,
            description: description.into(),
            parameters_schema,
            connection,
        }
    }

    fn result_from_content(content: &[ToolContent]) -> ToolResult {
        ToolResult {
            success: true,
            output: render_text(content),
            error: None,
            attachments: Vec::new(),
            taint_labels: mcp_output_taint_labels(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        }
    }

    fn result_from_error(error: &anyhow::Error) -> ToolResult {
        ToolResult {
            success: false,
            output: String::new(),
            error: Some(error.to_string()),
            attachments: Vec::new(),
            taint_labels: mcp_output_taint_labels(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        }
    }

    /// Returns the original tool name on the upstream MCP server.
    #[cfg(test)]
    #[must_use]
    fn upstream_tool_name(&self) -> &str {
        &self.tool_name
    }
}

impl Tool for McpToolProxy {
    fn name(&self) -> &str {
        &self.namespaced_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.parameters_schema.clone()
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let result = match self.connection.call_tool(&self.tool_name, args).await {
                Ok(content) => Self::result_from_content(&content),
                Err(ref error) => Self::result_from_error(error),
            };
            Ok(result)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::security::SecurityPolicy;

    #[test]
    fn proxy_tool_construction_namespaces_name() {
        let proxy = McpToolProxy::new(
            "search",
            "Search files",
            json!({"type": "object"}),
            Arc::new(McpConnection::disconnected_for_test("filesystem")),
            "filesystem",
        );

        assert_eq!(proxy.name(), "mcp_filesystem_search");
        assert_eq!(proxy.upstream_tool_name(), "search");
    }

    #[test]
    fn proxy_tool_content_conversion_renders_text() {
        let result = McpToolProxy::result_from_content(&[
            ToolContent::Text {
                text: "ok".to_string(),
            },
            ToolContent::Image {
                mime_type: "image/png".to_string(),
                data: "abc".to_string(),
            },
        ]);

        assert!(result.success);
        assert_eq!(result.error, None);
        assert_eq!(result.output, "ok\n[Image: image/png]");
        assert_eq!(result.taint_labels, vec!["external:mcp"]);
    }

    #[test]
    fn proxy_tool_spec_generation_uses_mcp_schema() {
        let proxy = McpToolProxy::new(
            "echo",
            "Echo input",
            json!({
                "type": "object",
                "properties": {"message": {"type": "string"}},
                "required": ["message"]
            }),
            Arc::new(McpConnection::disconnected_for_test("utility")),
            "utility",
        );

        let spec = proxy.spec();
        assert_eq!(spec.name, "mcp_utility_echo");
        assert_eq!(spec.description, "Echo input");
        assert_eq!(spec.parameters["required"][0], "message");
    }

    #[tokio::test]
    async fn proxy_tool_execute_returns_error_result_when_disconnected() {
        let proxy = McpToolProxy::new(
            "echo",
            "Echo input",
            json!({"type": "object"}),
            Arc::new(McpConnection::disconnected_for_test("utility")),
            "utility",
        );
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));

        let result = proxy
            .execute(json!({"message": "hello"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.is_empty());
        assert!(result.error.is_some());
    }
}
