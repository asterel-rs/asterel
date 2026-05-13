//! MCP client connection: stdio-based lifecycle management for a
//! single Model Context Protocol server process.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::plugins::mcp::bridge::from_rmcp_contents;
use crate::plugins::mcp::content::ToolContent;

type McpService = RunningService<RoleClient, ()>;
type McpPeer = Peer<RoleClient>;

const MCP_SAFE_ENV_VARS: &[&str] = &["PATH", "HOME", "TMPDIR", "TEMP", "TMP", "LANG", "LC_ALL"];

/// A single stdio-based connection to an external MCP server process.
pub(crate) struct McpConnection {
    name: String,
    service: Arc<RwLock<Option<McpService>>>,
    max_call_seconds: u64,
}

impl McpConnection {
    /// # Errors
    /// Returns an error if the MCP child process cannot be started or handshake fails.
    pub(super) async fn connect_stdio(
        name: impl Into<String>,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        max_call_seconds: u64,
    ) -> Result<Self> {
        let service = ()
            .serve(TokioChildProcess::new(Command::new(command).configure(
                |cmd| {
                    cmd.env_clear();
                    for var in MCP_SAFE_ENV_VARS {
                        if let Ok(value) = std::env::var(var) {
                            cmd.env(var, value);
                        }
                    }
                    cmd.args(args);
                    cmd.envs(env.iter());
                },
            ))?)
            .await
            .with_context(|| format!("failed to connect MCP server '{command}' over stdio"))?;

        Ok(Self {
            name: name.into(),
            service: Arc::new(RwLock::new(Some(service))),
            max_call_seconds,
        })
    }

    /// Returns the display name of this MCP connection.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// # Errors
    /// Returns an error if the connection is inactive or tool discovery fails.
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>> {
        let peer = self.active_peer().await?;

        let tools = peer
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools for MCP server '{}'", self.name))?;
        Ok(tools)
    }

    /// # Errors
    /// Returns an error if arguments are invalid, the connection is inactive, or the tool call fails.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<Vec<ToolContent>> {
        let arguments = match args {
            serde_json::Value::Object(object) => Some(object),
            serde_json::Value::Null => None,
            _ => {
                return Err(anyhow!(
                    "MCP tool '{tool_name}' requires JSON object arguments"
                ));
            }
        };

        let mut request = CallToolRequestParams::new(tool_name.to_string());
        request.arguments = arguments;

        let peer = self.active_peer().await?;

        let result = tokio::time::timeout(
            Duration::from_secs(self.max_call_seconds),
            peer.call_tool(request),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "MCP tool '{}' on server '{}' timed out after {}s",
                tool_name,
                self.name,
                self.max_call_seconds
            )
        })?
        .with_context(|| {
            format!(
                "MCP tool '{}' call failed on server '{}'",
                tool_name, self.name
            )
        })?;

        Ok(from_rmcp_contents(&result.content))
    }

    async fn active_peer(&self) -> Result<McpPeer> {
        let service_guard = self.service.read().await;
        let service = service_guard
            .as_ref()
            .ok_or_else(|| anyhow!("MCP connection '{}' is not active", self.name))?;
        Ok(Peer::clone(service))
    }

    /// # Errors
    /// Returns an error if shutting down the running MCP service fails.
    pub async fn shutdown(&self) -> Result<()> {
        let service = self.service.write().await.take();
        if let Some(service) = service {
            service
                .cancel()
                .await
                .with_context(|| format!("failed to shutdown MCP server '{}'", self.name))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn disconnected_for_test(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            service: Arc::new(RwLock::new(None)),
            max_call_seconds: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn disconnected_list_tools_reports_not_active() {
        let connection = McpConnection::disconnected_for_test("utility");

        let error = connection
            .list_tools()
            .await
            .expect_err("disconnected connection should not list tools");

        assert!(
            error.to_string().contains("not active"),
            "inactive error should mention not active: {error:#}"
        );
    }

    #[tokio::test]
    async fn disconnected_call_tool_reports_not_active() {
        let connection = McpConnection::disconnected_for_test("utility");

        let error = connection
            .call_tool("echo", json!({"message": "hello"}))
            .await
            .expect_err("disconnected connection should not call tools");

        assert!(
            error.to_string().contains("not active"),
            "inactive error should mention not active: {error:#}"
        );
    }
}
