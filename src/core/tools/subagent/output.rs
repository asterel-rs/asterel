//! Subagent output tool — polls status and output of a background sub-agent run.
//!
//! # What it does
//!
//! `subagent_output` looks up a run snapshot by `run_id` in the run registry,
//! scoped to the calling entity's `entity_id` to prevent one entity from
//! reading another's run output. If the run is not found (wrong ID, wrong
//! entity scope, or already garbage-collected), the tool returns a non-success
//! `ToolResult` rather than an `Err` so the caller can handle it gracefully.
//!
//! # Middleware integration
//!
//! When a custom `subagent_manager` is attached to the `ExecutionContext`
//! (used in tests and hosted environments), the tool delegates to it.
//! Otherwise the global process-level subagent registry is used.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::contracts::ids::RunId;
use crate::core::subagents;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that retrieves the status and output of a background sub-agent run.
pub struct SubagentOutputTool;

impl SubagentOutputTool {
    /// Create a new subagent-output tool instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubagentOutputTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for SubagentOutputTool {
    fn name(&self) -> &'static str {
        "subagent_output"
    }

    fn description(&self) -> &'static str {
        "Get status/output for a spawned sub-agent run"
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
            let snapshot = if let Some(manager) = ctx.subagent_manager.as_ref() {
                manager.get_scoped(&RunId::from(run_id), ctx.entity_id.as_str())
            } else {
                subagents::get_scoped(&RunId::from(run_id), ctx.entity_id.as_str())
            };
            let Some(snapshot) = snapshot else {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("subagent run not found: {run_id}")),
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            };

            Ok(ToolResult {
                success: true,
                output: serde_json::to_string(&snapshot)?,
                error: None,
                attachments: Vec::new(),
                taint_labels: super::subagent_output_taint_labels(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
