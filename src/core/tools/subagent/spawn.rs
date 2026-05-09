//! Subagent spawn tool — starts an isolated sub-agent run or an inline task.
//!
//! # What it does
//!
//! `subagent_spawn` launches a new agent run with its own execution context,
//! tool registry, and delegation depth. Two modes are supported:
//!
//! * **Background** (`run_in_background: true`, default) — the run is queued
//!   immediately; the tool returns `{ "status": "accepted", "run_id": "…" }`.
//!   The caller polls progress with `subagent_output` and cancels with
//!   `subagent_cancel`.
//! * **Inline** (`run_in_background: false`) — the run completes before the
//!   tool returns; output is embedded in the response. Suitable for short
//!   tasks that must finish before the parent can continue.
//!
//! An optional structured handoff envelope (`objective`, `done_when`,
//! `context`, `constraints`) can be attached to help the child agent
//! understand what is expected of it.
//!
//! # Middleware integration
//!
//! The tool calls `build_delegation_options` which enforces both the delegation
//! depth limit and the per-context child quota before the run is launched.
//! `ReadOnly` autonomy blocks spawn entirely.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::core::subagents;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that launches an isolated sub-agent run or an inline delegated task.
pub struct SubagentSpawnTool;

impl SubagentSpawnTool {
    /// Create a new subagent-spawn tool instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubagentSpawnTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for SubagentSpawnTool {
    fn name(&self) -> &'static str {
        "subagent_spawn"
    }

    fn description(&self) -> &'static str {
        "Spawn an isolated sub-agent run or execute inline task"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "Task instruction for sub-agent" },
                "objective": { "type": "string", "description": "Optional high-level goal for the delegated task" },
                "done_when": { "type": "string", "description": "Optional completion condition for the delegated task" },
                "context": { "type": "string", "description": "Optional supporting context for the delegated task" },
                "constraints": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional constraints the delegated task must respect"
                },
                "label": { "type": "string", "description": "Optional run label" },
                "model": { "type": "string", "description": "Optional model override" },
                "run_in_background": { "type": "boolean", "description": "If false, run inline and return output", "default": true }
            },
            "required": ["task"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let task = args
                .get("task")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("Missing 'task' parameter"))?
                .trim()
                .to_string();
            if task.is_empty() {
                anyhow::bail!("'task' must not be empty");
            }
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            let model = args
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            let handoff = crate::core::tools::subagent::parse_handoff_envelope(&args)?;
            if ctx.autonomy_level == crate::security::policy::AutonomyLevel::ReadOnly {
                anyhow::bail!(
                    "{SECURITY_POLICY_BLOCK_PREFIX}subagent_spawn is not allowed in read-only mode"
                );
            }

            let background = args
                .get("run_in_background")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            let options =
                crate::core::tools::subagent::build_delegation_options(ctx, label, model, handoff)?;

            if background {
                let snapshot = subagents::spawn_with_options(task, options)?;
                return Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string(&json!({
                        "status": "accepted",
                        "run_id": snapshot.run_id,
                        "started_at": snapshot.started_at,
                    }))?,
                    error: None,
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            let output = subagents::run_inline_with_options(task, options).await?;
            Ok(ToolResult {
                success: true,
                output: serde_json::to_string(&json!({
                    "status": "completed",
                    "output": output,
                }))?,
                error: None,
                attachments: Vec::new(),
                taint_labels: super::subagent_output_taint_labels(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}
