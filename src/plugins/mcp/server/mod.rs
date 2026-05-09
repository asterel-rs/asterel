//! MCP server — exposes `Asterel` tools via MCP protocol.
//!

use std::sync::Arc;

use crate::core::memory::Memory;
use crate::core::tools::traits::Tool;

pub mod memory_tools;

pub use memory_tools::{McpMemoryGraphQueryTool, McpMemoryLookupTool, McpMemoryRecallTool};

pub fn create_mcp_memory_tools(memory: Arc<dyn Memory>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(memory_tools::McpMemoryRecallTool::new(Arc::clone(&memory))),
        Box::new(memory_tools::McpMemoryLookupTool::new(Arc::clone(&memory))),
        Box::new(memory_tools::McpMemoryGraphQueryTool::new(memory)),
    ]
}
