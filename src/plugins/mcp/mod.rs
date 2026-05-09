//! MCP (Model Context Protocol) subsystem.
//!
//! Provides client connections to external MCP servers and
//! optional server mode for exposing tools via MCP.

pub mod bridge;
pub(crate) mod client_connection;
pub(crate) mod client_manager;
pub(crate) mod client_proxy_tool;
pub mod content;
pub mod server;

pub use client_manager::{McpOrchestrator, create_mcp_tools, create_mcp_tools_with_policy};
