//! Unified task-delegation tool (`delegate`).
//!
//! Exposes subagent lifecycle management to the LLM through a single
//! multi-action tool rather than four separate tools.  The `action`
//! parameter selects one of:
//!
//! | `action` | Effect |
//! |----------|--------|
//! | `run` | Spawn a subagent to execute `task`; sync or async. |
//! | `status` | Query the current state of a background run by `run_id`. |
//! | `list` | List all runs visible to the calling entity. |
//! | `cancel` | Request cancellation of a running subagent. |
//!
//! # Security
//!
//! Delegation is blocked entirely in `ReadOnly` autonomy mode.  In
//! `Supervised` mode the `SecurityMiddleware` will require approval before
//! the orchestrator proceeds (delegation is not in the read-only tool set).
//!
//! The delegation depth and child quota are enforced by
//! `build_delegation_options` (in `crate::core::tools::subagent`), which
//! reads `ctx.delegation_depth`, `ctx.max_delegation_depth`, and
//! `ctx.remaining_child_delegations` from the execution context.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use super::traits::{Tool, ToolResult};
use crate::contracts::ids::RunId;
use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::core::subagents;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::subagent::subagent_output_taint_labels;

/// Tool for spawning, querying, and cancelling subagent runs.
///
/// When `ctx.subagent_manager` is set (runtime mode), operations are routed
/// through the injected [`SubagentOrchestrator`].  Otherwise the process-global
/// singleton in `crate::core::subagents` is used (single-process mode).
pub struct DelegateTool;

impl DelegateTool {
    /// Create a new delegate tool instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for DelegateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for DelegateTool {
    fn name(&self) -> &'static str {
        "delegate"
    }

    fn description(&self) -> &'static str {
        "Unified task delegation tool: run, status, list, cancel"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["run", "status", "list", "cancel"],
                    "description": "Delegate operation"
                },
                "task": { "type": "string", "description": "Task instruction (required for action=run)" },
                "objective": { "type": "string", "description": "Optional high-level goal for the delegated task" },
                "done_when": { "type": "string", "description": "Optional completion condition for the delegated task" },
                "context": { "type": "string", "description": "Optional supporting context for the delegated task" },
                "constraints": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional constraints the delegated task must respect"
                },
                "run_id": { "type": "string", "description": "Run id (required for action=status|cancel)" },
                "label": { "type": "string", "description": "Optional run label for action=run" },
                "model": { "type": "string", "description": "Optional model override for action=run" },
                "run_in_background": { "type": "boolean", "description": "For action=run: true=async, false=sync", "default": false }
            },
            "required": ["action"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

            match action {
                "run" => execute_run(&args, ctx).await,
                "status" => execute_status(&args, ctx),
                "list" => execute_list(ctx),
                "cancel" => execute_cancel(&args, ctx),
                other => anyhow::bail!("unsupported action: {other}"),
            }
        })
    }
}

async fn execute_run(args: &Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
    let task = args
        .get("task")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing 'task' parameter for action=run"))?
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
    let handoff = crate::core::tools::subagent::parse_handoff_envelope(args)?;
    let run_in_background = args
        .get("run_in_background")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if ctx.autonomy_level == crate::security::policy::AutonomyLevel::ReadOnly {
        anyhow::bail!("{SECURITY_POLICY_BLOCK_PREFIX}delegate is not allowed in read-only mode");
    }

    let options =
        crate::core::tools::subagent::build_delegation_options(ctx, label, model, handoff)?;

    if run_in_background {
        let snapshot = if let Some(manager) = ctx.subagent_manager.as_ref() {
            manager.spawn_with_options(task, options)?
        } else {
            subagents::spawn_with_options(task, options)?
        };
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

    let output = if let Some(manager) = ctx.subagent_manager.as_ref() {
        manager.run_inline_with_options(task, options).await?
    } else {
        subagents::run_inline_with_options(task, options).await?
    };
    Ok(ToolResult {
        success: true,
        output: serde_json::to_string(&json!({
            "status": "completed",
            "output": output,
        }))?,
        error: None,
        attachments: Vec::new(),
        taint_labels: subagent_output_taint_labels(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    })
}

fn execute_status(args: &Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
    let run_id = args
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing 'run_id' parameter for action=status"))?;
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
        taint_labels: subagent_output_taint_labels(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    })
}

fn execute_list(ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
    let snapshots = if let Some(manager) = ctx.subagent_manager.as_ref() {
        manager.list_scoped(ctx.entity_id.as_str())
    } else {
        subagents::list_scoped(ctx.entity_id.as_str())
    };
    Ok(ToolResult {
        success: true,
        output: serde_json::to_string(&snapshots)?,
        error: None,
        attachments: Vec::new(),
        taint_labels: subagent_output_taint_labels(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    })
}

#[cfg(test)]
mod tests {
    use crate::core::tools::subagent::{SUBAGENT_OUTPUT_TAINT_LABEL, subagent_output_taint_labels};

    #[test]
    fn subagent_output_taint_label_marks_child_output_boundary() {
        assert_eq!(
            subagent_output_taint_labels(),
            vec![SUBAGENT_OUTPUT_TAINT_LABEL.to_string()]
        );
    }
}

fn execute_cancel(args: &Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
    let run_id = args
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing 'run_id' parameter for action=cancel"))?;
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
}
