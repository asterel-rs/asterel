//! Model Context Protocol (MCP) server configuration: server
//! definitions, transport types (stdio/http), and validation.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

fn default_max_call_seconds() -> u64 {
    30
}

fn default_enabled_true() -> bool {
    true
}

/// Top-level MCP configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    /// Whether MCP support is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Optional path to a Claude Desktop-style mcp.json file to import.
    /// Servers defined here are merged with inline `servers` entries.
    /// Inline entries with the same name take precedence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_json: Option<String>,

    /// MCP server definitions.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server.
    pub name: String,

    /// Transport kind.
    #[serde(default)]
    pub transport: McpTransport,

    /// Whether this server is enabled.
    #[serde(default = "default_enabled_true")]
    pub enabled: bool,

    /// Maximum seconds per tool call.
    #[serde(default = "default_max_call_seconds")]
    pub max_call_seconds: u64,
}

/// Transport configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransport {
    /// Stdio transport -- spawns a child process.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// HTTP transport -- connects to a remote server.
    Http { url: String },
}

impl Default for McpTransport {
    fn default() -> Self {
        Self::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
        }
    }
}

impl McpConfig {
    /// Validate the configuration, returning errors for invalid entries.
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut names: HashSet<&str> = HashSet::new();

        for server in &self.servers {
            if server.name.is_empty() {
                errors.push("MCP server name cannot be empty".to_string());
            }
            if !names.insert(server.name.as_str()) {
                errors.push(format!("Duplicate MCP server name: {}", server.name));
            }
            match &server.transport {
                McpTransport::Stdio { command, .. } => {
                    if command.is_empty() {
                        errors.push(format!(
                            "MCP server '{}': stdio transport requires a command",
                            server.name
                        ));
                    }
                }
                McpTransport::Http { url } => {
                    if url.is_empty() {
                        errors.push(format!(
                            "MCP server '{}': http transport requires a url",
                            server.name
                        ));
                    }
                    if let Ok(parsed) = url::Url::parse(url)
                        && let Some(host) = parsed.host_str()
                        && crate::contracts::network::is_private_host(host)
                    {
                        errors.push(format!(
                            "MCP server '{}': http URL points to private/internal address",
                            server.name
                        ));
                    }
                }
            }
            if server.max_call_seconds == 0 {
                errors.push(format!(
                    "MCP server '{}': max_call_seconds must be > 0",
                    server.name
                ));
            }
        }

        errors
    }

    /// Return only enabled servers.
    #[must_use]
    pub fn enabled_servers(&self) -> Vec<&McpServerConfig> {
        self.servers
            .iter()
            .filter(|server| server.enabled)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_disabled_with_no_servers() {
        let cfg = McpConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.import_json.is_none());
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn config_toml_round_trip() {
        let cfg = McpConfig {
            enabled: true,
            import_json: Some("~/.config/claude/mcp.json".to_string()),
            servers: vec![McpServerConfig {
                name: "filesystem".to_string(),
                transport: McpTransport::Stdio {
                    command: "npx".to_string(),
                    args: vec![
                        "-y".to_string(),
                        "@modelcontextprotocol/server-filesystem".to_string(),
                    ],
                    env: HashMap::new(),
                },
                enabled: true,
                max_call_seconds: 30,
            }],
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: McpConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "filesystem");
    }

    #[test]
    fn validate_rejects_empty_name() {
        let cfg = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![McpServerConfig {
                name: String::new(),
                transport: McpTransport::Stdio {
                    command: "npx".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                },
                enabled: true,
                max_call_seconds: 30,
            }],
        };
        let errors = cfg.validate();
        assert!(errors.iter().any(|error| error.contains("cannot be empty")));
    }

    #[test]
    fn validate_rejects_duplicate_names() {
        let server = McpServerConfig {
            name: "test".to_string(),
            transport: McpTransport::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
            enabled: true,
            max_call_seconds: 30,
        };
        let cfg = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![server.clone(), server],
        };
        let errors = cfg.validate();
        assert!(errors.iter().any(|error| error.contains("Duplicate")));
    }

    #[test]
    fn validate_rejects_empty_stdio_command() {
        let cfg = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![McpServerConfig {
                name: "bad".to_string(),
                transport: McpTransport::Stdio {
                    command: String::new(),
                    args: vec![],
                    env: HashMap::new(),
                },
                enabled: true,
                max_call_seconds: 30,
            }],
        };
        let errors = cfg.validate();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("requires a command"))
        );
    }

    #[test]
    fn validate_rejects_empty_http_url() {
        let cfg = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![McpServerConfig {
                name: "bad".to_string(),
                transport: McpTransport::Http { url: String::new() },
                enabled: true,
                max_call_seconds: 30,
            }],
        };
        let errors = cfg.validate();
        assert!(errors.iter().any(|error| error.contains("requires a url")));
    }

    #[test]
    fn validate_rejects_zero_timeout() {
        let cfg = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![McpServerConfig {
                name: "bad".to_string(),
                transport: McpTransport::Stdio {
                    command: "echo".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                },
                enabled: true,
                max_call_seconds: 0,
            }],
        };
        let errors = cfg.validate();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("max_call_seconds"))
        );
    }

    #[test]
    fn enabled_servers_filters_disabled() {
        let cfg = McpConfig {
            enabled: true,
            import_json: None,
            servers: vec![
                McpServerConfig {
                    name: "on".to_string(),
                    transport: McpTransport::Stdio {
                        command: "echo".to_string(),
                        args: vec![],
                        env: HashMap::new(),
                    },
                    enabled: true,
                    max_call_seconds: 30,
                },
                McpServerConfig {
                    name: "off".to_string(),
                    transport: McpTransport::Stdio {
                        command: "echo".to_string(),
                        args: vec![],
                        env: HashMap::new(),
                    },
                    enabled: false,
                    max_call_seconds: 30,
                },
            ],
        };
        let enabled = cfg.enabled_servers();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "on");
    }

    #[test]
    fn deserialize_minimal_toml() {
        let toml_str = r#"
enabled = true

[[servers]]
name = "test"

[servers.transport]
kind = "stdio"
command = "echo"
"#;
        let cfg: McpConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].max_call_seconds, 30);
    }
}
