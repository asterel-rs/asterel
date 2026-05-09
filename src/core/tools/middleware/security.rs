//! Security middleware (`SecurityMiddleware`) — first in the chain.
//!
//! Runs before every tool execution and enforces all security policies that
//! must be checked *before* a tool is allowed to proceed.  It is split into
//! three sub-checks applied in order:
//!
//! 1. **Global policy** ([`check_global_policy`])
//!    - `ReadOnly` autonomy → allow only safe read-only tools.
//!    - Tool allowlist → reject tools not in `ctx.allowed_tools`.
//!    - External-action gate → reject `composio`, `mcp_*`, and side-effecting
//!      channel broker tools when `external_action_execution` is `Disabled`.
//!    - Network-isolation gate → reject network tools for isolated groups.
//!    - Capability check → reject tools whose required capabilities are not
//!      in `ctx.granted_capabilities`.
//!
//! 2. **Tool-specific policy** ([`check_tool_specific_policy`])
//!    - `shell` → validate command against the allowlist via
//!      [`enforce_shell_command_guardrails`].
//!    - `file_read` / `file_write` → canonicalise and check the target path
//!      against workspace boundaries (including group-workspace boundary when
//!      applicable).
//!    - `memory_governance` → requires `can_act()`.
//!
//! 3. **Approval routing** ([`check_approval_requirement`])
//!    - When a `PolicyEngine` is present, delegates the full
//!      allow/deny/ask decision to it.
//!    - Otherwise falls back to the raw permission-grant store and the
//!      `Supervised`-autonomy require-approval rule (all tools).
//!
//! All three checks return `None` / `Continue` on the happy path.  A
//! non-`None` / non-`Continue` short-circuits the remaining middleware.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::Value;

use super::{
    ExecutionContext, MiddlewareDecision, ToolMiddleware, enforce_shell_command_guardrails,
    is_critical_bootstrap_target, is_external_action_tool, is_network_boundary_tool,
};
use crate::config::GroupIsolationLevel;
use crate::contracts::strings::verdicts::{
    SECURITY_BLOCK_AUTONOMY_READ_ONLY, SECURITY_POLICY_BLOCK_PREFIX,
};
use crate::core::tools::traits::{ActionIntent, ToolResult};
use crate::security::ExternalActionExecution;
use crate::security::approval::summarize_args;
use crate::security::capability::check_capabilities;
use crate::security::policy::AutonomyLevel;
use crate::security::tool_policy::{PolicyDecisionKind, SUPERVISED_FALLBACK_APPROVAL_REASON};

/// First middleware in the chain: enforces the full security policy before
/// any tool is allowed to execute.
///
/// Its `after_execute` hook is a no-op — security enforcement happens
/// entirely in the `before_execute` phase.
#[derive(Debug)]
pub struct SecurityMiddleware;

impl ToolMiddleware for SecurityMiddleware {
    fn before_execute<'a>(
        &'a self,
        tool_name: &'a str,
        args: &'a Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(decision) = check_global_policy(tool_name, ctx) {
                return Ok(decision);
            }

            if let Some(decision) = check_tool_specific_policy(tool_name, args, ctx).await {
                return Ok(decision);
            }

            Ok(check_approval_requirement(tool_name, args, ctx))
        })
    }

    fn after_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _result: &'a mut ToolResult,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {})
    }
}

fn check_global_policy(tool_name: &str, ctx: &ExecutionContext) -> Option<MiddlewareDecision> {
    if ctx.autonomy_level == AutonomyLevel::ReadOnly {
        match tool_name {
            super::tool_names::FILE_READ
            | super::tool_names::MEMORY_RECALL
            | super::tool_names::MEMORY_LOOKUP => {}
            _ => {
                return Some(MiddlewareDecision::Block(
                    SECURITY_BLOCK_AUTONOMY_READ_ONLY.to_string(),
                ));
            }
        }
    }

    if let Some(allowed_tools) = &ctx.allowed_tools
        && !allowed_tools.contains(tool_name)
    {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}tool '{tool_name}' is not allowed for this entity"
        )));
    }

    if is_external_action_tool(tool_name)
        && ctx.security.external_action_execution == ExternalActionExecution::Disabled
    {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}external_action_execution is disabled for tool '{tool_name}'"
        )));
    }

    if ctx.network_isolation != GroupIsolationLevel::Shared && is_network_boundary_tool(tool_name) {
        let group = ctx.routing_group.as_deref().unwrap_or("default");
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}network-isolated group '{group}' forbids tool '{tool_name}'"
        )));
    }

    // Capability-based check: if the context has a capability set,
    // verify the tool's required capabilities are granted.
    if let Some(granted) = &ctx.granted_capabilities
        && let Err(missing) = check_capabilities(&ctx.current_tool_capabilities, granted)
    {
        let names: Vec<String> = missing.iter().map(ToString::to_string).collect();
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}tool '{tool_name}' requires capabilities not granted: [{}]",
            names.join(", ")
        )));
    }

    None
}

async fn check_tool_specific_policy(
    tool_name: &str,
    args: &Value,
    ctx: &ExecutionContext,
) -> Option<MiddlewareDecision> {
    match tool_name {
        super::tool_names::SHELL => check_shell_policy(args, ctx),
        super::tool_names::FILE_READ => check_file_read_policy(args, ctx).await,
        super::tool_names::FILE_WRITE => check_file_write_policy(args, ctx).await,
        super::tool_names::FILE_DELETE => check_file_delete_policy(args, ctx).await,
        super::tool_names::MEMORY_GOVERNANCE => (!ctx.security.can_act())
            .then(|| MiddlewareDecision::Block(SECURITY_BLOCK_AUTONOMY_READ_ONLY.to_string())),
        _ => None,
    }
}

fn check_shell_policy(args: &Value, ctx: &ExecutionContext) -> Option<MiddlewareDecision> {
    let command = args.get("command").and_then(Value::as_str).unwrap_or("");
    enforce_shell_command_guardrails(ctx, command, "tool:shell")
        .err()
        .map(|error| MiddlewareDecision::Block(error.to_string()))
}

async fn check_file_read_policy(
    args: &Value,
    ctx: &ExecutionContext,
) -> Option<MiddlewareDecision> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    if !ctx.security.is_path_allowed(path) {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}path not allowed: {path}"
        )));
    }

    // This is a pre-execution path check.  The `file_read` tool adds a
    // second, TOCTOU-safer layer by opening with `O_NOFOLLOW` (Unix) or
    // `FILE_FLAG_OPEN_REPARSE_POINT` (Windows) on the final file handle.
    // On non-Unix / non-Windows platforms a narrow TOCTOU window remains
    // between this check and `File::open`; it is mitigated by the
    // workspace-confinement check here.
    let full_path = ctx.workspace_dir.join(path);
    if let Ok(resolved_path) = tokio::fs::canonicalize(&full_path).await {
        let workspace_root = tokio::fs::canonicalize(&ctx.workspace_dir)
            .await
            .unwrap_or_else(|_| ctx.workspace_dir.clone());
        let policy_workspace_root = tokio::fs::canonicalize(&ctx.security.workspace_dir)
            .await
            .unwrap_or_else(|_| ctx.security.workspace_dir.clone());
        let enforce_group_workspace_boundary =
            ctx.security.workspace_only || workspace_root != policy_workspace_root;
        if enforce_group_workspace_boundary && !resolved_path.starts_with(&workspace_root) {
            return Some(MiddlewareDecision::Block(format!(
                "{SECURITY_POLICY_BLOCK_PREFIX}resolved path escapes group workspace: {}",
                resolved_path.display()
            )));
        }
        if !ctx.security.is_path_allowed_resolved(&resolved_path) {
            return Some(MiddlewareDecision::Block(format!(
                "{SECURITY_POLICY_BLOCK_PREFIX}resolved path escapes workspace: {}",
                resolved_path.display()
            )));
        }
    } else if tokio::fs::symlink_metadata(&full_path).await.is_ok() {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}path exists but cannot be resolved \
             (possible symlink escape): {}",
            full_path.display()
        )));
    }
    None
}

async fn check_file_write_policy(
    args: &Value,
    ctx: &ExecutionContext,
) -> Option<MiddlewareDecision> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    if !ctx.security.is_path_allowed(path) {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}path not allowed: {path}"
        )));
    }
    if is_critical_bootstrap_target(path) {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}write target is protected bootstrap file: {path}"
        )));
    }

    let full_path = ctx.workspace_dir.join(path);
    let workspace_root = tokio::fs::canonicalize(&ctx.workspace_dir)
        .await
        .unwrap_or_else(|_| ctx.workspace_dir.clone());
    let policy_workspace_root = tokio::fs::canonicalize(&ctx.security.workspace_dir)
        .await
        .unwrap_or_else(|_| ctx.security.workspace_dir.clone());
    let enforce_group_workspace_boundary =
        ctx.security.workspace_only || workspace_root != policy_workspace_root;
    if let Some(parent) = full_path.parent() {
        let mut candidate: Option<&Path> = Some(parent);
        while let Some(current) = candidate {
            if current.exists() {
                if let Ok(resolved) = tokio::fs::canonicalize(current).await {
                    if enforce_group_workspace_boundary && !resolved.starts_with(&workspace_root) {
                        return Some(MiddlewareDecision::Block(format!(
                            "{SECURITY_POLICY_BLOCK_PREFIX}resolved path escapes group workspace: {}",
                            resolved.display()
                        )));
                    }
                    if !ctx.security.is_path_allowed_resolved(&resolved) {
                        return Some(MiddlewareDecision::Block(format!(
                            "{SECURITY_POLICY_BLOCK_PREFIX}resolved path escapes workspace: {}",
                            resolved.display()
                        )));
                    }
                }
                break;
            }
            candidate = current.parent();
        }
    }
    None
}

async fn check_file_delete_policy(
    args: &Value,
    ctx: &ExecutionContext,
) -> Option<MiddlewareDecision> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    if !ctx.security.is_path_allowed(path) {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}path not allowed: {path}"
        )));
    }
    if is_critical_bootstrap_target(path) {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}delete target is protected bootstrap file: {path}"
        )));
    }

    let full_path = ctx.workspace_dir.join(path);
    if let Ok(resolved_path) = tokio::fs::canonicalize(&full_path).await {
        let workspace_root = tokio::fs::canonicalize(&ctx.workspace_dir)
            .await
            .unwrap_or_else(|_| ctx.workspace_dir.clone());
        let policy_workspace_root = tokio::fs::canonicalize(&ctx.security.workspace_dir)
            .await
            .unwrap_or_else(|_| ctx.security.workspace_dir.clone());
        let enforce_group_workspace_boundary =
            ctx.security.workspace_only || workspace_root != policy_workspace_root;
        if enforce_group_workspace_boundary && !resolved_path.starts_with(&workspace_root) {
            return Some(MiddlewareDecision::Block(format!(
                "{SECURITY_POLICY_BLOCK_PREFIX}resolved path escapes group workspace: {}",
                resolved_path.display()
            )));
        }
        if !ctx.security.is_path_allowed_resolved(&resolved_path) {
            return Some(MiddlewareDecision::Block(format!(
                "{SECURITY_POLICY_BLOCK_PREFIX}resolved path escapes workspace: {}",
                resolved_path.display()
            )));
        }
    } else if tokio::fs::symlink_metadata(&full_path).await.is_ok() {
        return Some(MiddlewareDecision::Block(format!(
            "{SECURITY_POLICY_BLOCK_PREFIX}path exists but cannot be resolved \
             (possible symlink escape): {}",
            full_path.display()
        )));
    }
    None
}

fn check_approval_requirement(
    tool_name: &str,
    args: &Value,
    ctx: &ExecutionContext,
) -> MiddlewareDecision {
    let args_summary = summarize_args(tool_name, args);

    // When a PolicyEngine is present, delegate the full decision to it.
    // The engine's precedence chain (deny → ask → allow → grant → autonomy)
    // subsumes both the raw permission-store check and the autonomy fallback
    // in the legacy path below.
    if let Some(ref engine) = ctx.policy_engine {
        let has_grant = ctx.permission_store.as_ref().is_some_and(|store| {
            store.set_entity_allowlist(ctx.entity_id.as_str(), ctx.allowed_tools.clone());
            store.is_granted(
                tool_name,
                &args_summary,
                ctx.entity_id.as_str(),
                &ctx.tenant_context,
            )
        });

        let eval = engine.evaluate(tool_name, &args_summary, has_grant, ctx.autonomy_level);

        return match eval.decision {
            PolicyDecisionKind::Allow => MiddlewareDecision::Continue,
            PolicyDecisionKind::Deny => MiddlewareDecision::Block(eval.reason),
            PolicyDecisionKind::RequireApproval => {
                MiddlewareDecision::RequireApproval(ActionIntent::new(
                    tool_name,
                    ctx.entity_id.as_str(),
                    serde_json::json!({
                        "tool": tool_name,
                        "args_summary": args_summary,
                        "policy_reason": eval.reason,
                    }),
                ))
            }
        };
    }

    // Legacy path (no PolicyEngine): check the raw permission-grant store
    // first, then fall back to the Supervised-autonomy approval rule.
    if let Some(permission_store) = &ctx.permission_store {
        permission_store.set_entity_allowlist(ctx.entity_id.as_str(), ctx.allowed_tools.clone());
        if permission_store.is_granted(
            tool_name,
            &args_summary,
            ctx.entity_id.as_str(),
            &ctx.tenant_context,
        ) {
            return MiddlewareDecision::Continue;
        }
    }

    if ctx.autonomy_level == AutonomyLevel::Supervised {
        return MiddlewareDecision::RequireApproval(ActionIntent::new(
            tool_name,
            ctx.entity_id.as_str(),
            serde_json::json!({
                "tool": tool_name,
                "args_summary": args_summary,
                "policy_reason": SUPERVISED_FALLBACK_APPROVAL_REASON,
            }),
        ));
    }

    MiddlewareDecision::Continue
}
