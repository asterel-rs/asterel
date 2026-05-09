//! Subagent cancel tool — signals a background sub-agent run to stop.
//!
//! # What it does
//!
//! `subagent_cancel` looks up a run by `run_id` in the run registry (scoped to
//! the calling entity's `entity_id`) and requests cancellation. Whether the
//! run is actually interrupted depends on the backend — some backends support
//! cooperative cancellation, others may wait for the current step to finish.
//!
//! # Middleware integration
//!
//! Mirrors `subagent_output`: uses the `ExecutionContext`-injected
//! `subagent_manager` when present, falling back to the global registry.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::contracts::ids::RunId;
use crate::core::subagents;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that cancels a running background sub-agent by its run ID.
pub struct SubagentCancelTool;

impl SubagentCancelTool {
    /// Create a new subagent-cancel tool instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubagentCancelTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for SubagentCancelTool {
    fn name(&self) -> &'static str {
        "subagent_cancel"
    }

    fn description(&self) -> &'static str {
        "Cancel a running sub-agent"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Sub-agent run id" }
            },
            "required": ["run_id"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let run_id = args
                .get("run_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("Missing 'run_id' parameter"))?;
            if let Some(manager) = ctx.subagent_manager.as_ref() {
                manager.cancel_scoped(&RunId::from(run_id), ctx.entity_id.as_str())?;
            } else {
                subagents::cancel_scoped(&RunId::from(run_id), ctx.entity_id.as_str())?;
            }

            Ok(ToolResult {
                success: true,
                output: serde_json::to_string(&json!({
                    "status": "cancelled",
                    "run_id": run_id,
                }))?,
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
