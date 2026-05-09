//! MCP client manager: bootstraps and manages connections to
//! multiple MCP servers, registering their tools as local proxies.

use std::sync::Arc;

use anyhow::Result;

use crate::config::schema::{McpConfig, McpTransport};
use crate::core::tools::Tool;
use crate::plugins::extensions::merge_mcp_config_with_workspace_extensions;
use crate::plugins::mcp::client_connection::McpConnection;
use crate::plugins::mcp::client_proxy_tool::McpToolProxy;
use crate::security::{ProcessSpawnClass, SecurityPolicy, enforce_process_spawn_policy_with_args};

const ROUTE_MCP_STDIO_BOOTSTRAP: &str = "plugins_mcp_stdio_bootstrap";

#[derive(Clone)]
struct ManagedTool {
    connection: Arc<McpConnection>,
    server_name: String,
    tool_name: String,
    description: String,
    parameters_schema: serde_json::Value,
}

/// Manages connections to multiple MCP servers and their proxy tools.
pub struct McpOrchestrator {
    connections: Vec<Arc<McpConnection>>,
    tools: Vec<ManagedTool>,
}

impl McpOrchestrator {
    /// # Errors
    ///
    /// Returns an error when MCP client bootstrap or tool registration fails
    /// for enabled servers.
    pub async fn from_config(config: &McpConfig, security: &SecurityPolicy) -> Result<Self> {
        if !config.enabled {
            return Ok(Self {
                connections: Vec::new(),
                tools: Vec::new(),
            });
        }

        let mut connections: Vec<Arc<McpConnection>> = Vec::new();
        let mut managed_tools = Vec::new();

        for server in config.enabled_servers() {
            if server.max_call_seconds == 0 {
                tracing::warn!(
                    server = %server.name,
                    "Skipping MCP server with invalid max_call_seconds=0"
                );
                continue;
            }

            match &server.transport {
                McpTransport::Stdio { command, args, env } => {
                    if let Some((connection, tools)) =
                        connect_stdio_server(server, command, args, env, security).await
                    {
                        managed_tools.extend(tools);
                        connections.push(connection);
                    }
                }
                McpTransport::Http { .. } => {
                    tracing::warn!(
                        server = %server.name,
                        "MCP HTTP transport is not supported yet; skipping server"
                    );
                }
            }
        }

        Ok(Self {
            connections,
            tools: managed_tools,
        })
    }

    /// Returns boxed proxy tools for all discovered MCP server tools.
    #[must_use]
    pub fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools
            .iter()
            .map(|tool| {
                Box::new(McpToolProxy::new(
                    tool.tool_name.clone(),
                    tool.description.clone(),
                    tool.parameters_schema.clone(),
                    Arc::clone(&tool.connection),
                    tool.server_name.clone(),
                )) as Box<dyn Tool>
            })
            .collect()
    }

    /// Shut down all active MCP connections gracefully.
    pub async fn shutdown(&self) {
        for connection in &self.connections {
            if let Err(error) = connection.shutdown().await {
                tracing::warn!(
                    server = %connection.name(),
                    error = %error,
                    "Failed to shutdown MCP connection"
                );
            }
        }
    }
}

/// Create MCP tools from config using the default security policy.
#[must_use]
pub fn create_mcp_tools(config: &McpConfig) -> Vec<Box<dyn Tool>> {
    let security = SecurityPolicy::default();
    create_mcp_tools_with_policy(config, &security)
}

/// Create MCP tools from config, enforcing the given security policy.
///
/// This function is intentionally synchronous because it is called from the
/// synchronous `all_tools()` factory. To avoid a nested `block_on` deadlock
/// when an ambient Tokio runtime is active, a dedicated OS thread with its own
/// single-threaded runtime is spawned. When no runtime is active (CLI
/// startup), the builder runs directly without the thread overhead.
pub fn create_mcp_tools_with_policy(
    config: &McpConfig,
    security: &SecurityPolicy,
) -> Vec<Box<dyn Tool>> {
    let merged_config = merge_mcp_config_with_workspace_extensions(config, &security.workspace_dir);
    if !merged_config.enabled {
        return Vec::new();
    }

    let config = merged_config;
    let security = security.clone();
    let builder = move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::warn!(error = %error, "Failed to build runtime for MCP tool creation");
                return Vec::new();
            }
        };

        runtime.block_on(async move {
            match McpOrchestrator::from_config(&config, &security).await {
                Ok(manager) => manager.tools(),
                Err(error) => {
                    tracing::warn!(error = %error, "Failed to create MCP manager from config");
                    Vec::new()
                }
            }
        })
    };

    // DEADLOCK GUARD: when called from within a Tokio executor, block_on()
    // on the same runtime would deadlock. Spawn a separate OS thread that
    // owns its own current-thread runtime to safely bridge sync → async.
    if tokio::runtime::Handle::try_current().is_ok() {
        if let Ok(tools) = std::thread::spawn(builder).join() {
            tools
        } else {
            tracing::warn!("MCP tool creation thread panicked");
            Vec::new()
        }
    } else {
        builder()
    }
}

/// Validates the security policy, connects a stdio MCP server, and lists its tools.
///
/// Returns `Some((connection, tools))` on success, or `None` when the server should
/// be skipped (empty command, security-policy rejection, connection failure, or tool
/// listing failure that is treated as non-fatal).
async fn connect_stdio_server(
    server: &crate::config::schema::McpServerConfig,
    command: &str,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
    security: &SecurityPolicy,
) -> Option<(Arc<McpConnection>, Vec<ManagedTool>)> {
    if command.is_empty() {
        tracing::warn!(
            server = %server.name,
            "Skipping MCP stdio server with empty command"
        );
        return None;
    }

    let spawn_args = args.to_vec();
    if let Err(error) = enforce_process_spawn_policy_with_args(
        security,
        command,
        &spawn_args,
        ROUTE_MCP_STDIO_BOOTSTRAP,
        ProcessSpawnClass::ExternalConnector,
    ) {
        tracing::warn!(
            server = %server.name,
            command = %command,
            error = %error,
            "Skipping MCP stdio server blocked by security policy"
        );
        return None;
    }

    let connection = match McpConnection::connect_stdio(
        server.name.clone(),
        command,
        args,
        env,
        server.max_call_seconds,
    )
    .await
    {
        Ok(connection) => Arc::new(connection),
        Err(error) => {
            tracing::warn!(
                server = %server.name,
                error = %error,
                "Failed to connect MCP stdio server"
            );
            return None;
        }
    };

    let tools = collect_server_tools(&connection, &server.name).await;
    Some((connection, tools))
}

/// Lists all tools exposed by a connected MCP server and maps them to `ManagedTool`
/// records that hold a shared reference to the connection.
async fn collect_server_tools(
    connection: &Arc<McpConnection>,
    server_name: &str,
) -> Vec<ManagedTool> {
    match connection.list_tools().await {
        Ok(server_tools) => server_tools
            .into_iter()
            .map(|tool| ManagedTool {
                connection: Arc::clone(connection),
                server_name: server_name.to_string(),
                tool_name: tool.name.into_owned(),
                description: tool
                    .description
                    .map_or_else(String::new, std::borrow::Cow::into_owned),
                parameters_schema: serde_json::Value::Object(tool.input_schema.as_ref().clone()),
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                server = %server_name,
                error = %error,
                "Failed to list MCP tools from server"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::create_mcp_tools_with_policy;
    use crate::config::schema::{McpConfig, McpServerConfig, McpTransport};
    use crate::security::SecurityPolicy;

    #[test]
    fn create_mcp_tools_blocks_non_allowlisted_stdio_command() {
        let config = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![McpServerConfig {
                name: "blocked-server".to_string(),
                transport: McpTransport::Stdio {
                    command: "forbidden-mcp-binary".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                },
                enabled: true,
                max_call_seconds: 30,
            }],
        };

        let security = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };

        let tools = create_mcp_tools_with_policy(&config, &security);
        assert!(
            tools.is_empty(),
            "blocked MCP command should produce no proxy tools"
        );
    }
}
