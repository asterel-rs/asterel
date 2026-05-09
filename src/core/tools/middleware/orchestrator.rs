//! Shared tool-execution orchestration layer.
//!
//! [`ToolExecutionOrchestrator`] is the component called by [`ToolRegistry`]
//! after a tool is looked up.  It:
//!
//! 1. Runs the `before_execute` phase of each middleware in order.
//! 2. When a middleware returns [`MiddlewareDecision::RequireApproval`],
//!    routes the intent through the approval flow
//!    (`request_approval_with_cache`), then continues or blocks accordingly.
//! 3. Calls the tool with transient-error retry (up to
//!    [`MAX_TRANSIENT_TOOL_ATTEMPTS`] with exponential back-off capped at
//!    [`MAX_TRANSIENT_BACKOFF_MS`]).
//! 4. Records success/violation signals in the [`DomainTrustTracker`].
//! 5. Runs the `after_execute` phase.
//! 6. Emits a [`ToolExecutionAuditRecord`] to the audit sink.
//!
//! In addition this module exposes two standalone guardrail helpers that
//! are called from both the `SecurityMiddleware` (pre-execution) and from
//! tool implementations directly:
//!
//! - [`enforce_shell_command_guardrails`] — validates a shell command string
//!   against the security policy and process-isolation rules.
//! - [`enforce_process_spawn_guardrails`] — validates direct process spawns
//!   by class (`ToolEquivalent` or `ExternalConnector`).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{ExecutionContext, MiddlewareDecision, ToolExecutionAuditRecord, ToolMiddleware};
use crate::config::GroupIsolationLevel;
use crate::contracts::ids::RequestId;
use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::core::tools::traits::{ActionIntent, Tool, ToolResult};
use crate::security::approval::{
    ApprovalDecision, ApprovalRequest, classify_risk_args, summarize_args,
};
use crate::security::governance::{
    AutonomyVerdict, GovernanceAuditContext, GovernanceAuditRecord, GovernanceDecision,
    GovernanceTrustState, TrustLevel, evaluate_governance,
};
use crate::security::process_spawn::{ProcessSpawnClass, enforce_process_spawn_policy_with_args};

const TOOL_EXECUTION_AUDIT_SUMMARY_LIMIT: usize = 180;
const MAX_TRANSIENT_TOOL_ATTEMPTS: usize = 3;
const INITIAL_TRANSIENT_BACKOFF_MS: u64 = 150;
const MAX_TRANSIENT_BACKOFF_MS: u64 = 1_000;

/// Outcome of routing an [`ActionIntent`] through the approval flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResolution {
    /// The intent was approved (either by the broker or a cached grant).
    Approved,
    /// The intent was explicitly denied; `reason` is surfaced to the model.
    Denied { reason: String },
}

enum PreExecutionOutcome {
    Continue,
    Return(Box<ToolResult>),
}

/// Per-call orchestrator that drives the middleware chain and the tool itself.
///
/// Created fresh for each registry dispatch — it holds only a borrow of the
/// middleware slice, not an owned copy.
pub struct ToolExecutionOrchestrator<'a> {
    middleware: &'a [Arc<dyn ToolMiddleware>],
}

impl<'a> ToolExecutionOrchestrator<'a> {
    #[must_use]
    pub fn new(middleware: &'a [Arc<dyn ToolMiddleware>]) -> Self {
        Self { middleware }
    }

    /// Execute a tool through shared approval, retry, and audit handling.
    ///
    /// # Errors
    ///
    /// Returns an error when the orchestration flow itself fails, such as
    /// a missing required approval broker.
    pub async fn execute(
        &self,
        tool_name: &str,
        args: Value,
        ctx: &ExecutionContext,
        tool: &Arc<dyn Tool>,
    ) -> Result<ToolResult> {
        let mut execution_ctx = ctx.clone();
        execution_ctx.current_tool_capabilities = tool.spec().required_capabilities;
        let ctx = &execution_ctx;
        let args_summary = summarize_args(tool_name, &args);

        match self.before_execute(tool_name, &args, ctx).await? {
            PreExecutionOutcome::Continue => {}
            PreExecutionOutcome::Return(result) => {
                let mut result = *result;
                for middleware in self.middleware {
                    if middleware.runs_after_pre_execution_return() {
                        middleware.after_execute(tool_name, &mut result, ctx).await;
                    }
                }
                emit_tool_execution_audit(ctx, tool_name, &args_summary, &result).await;
                return Ok(result);
            }
        }

        let mut result = execute_with_retry(tool_name, &args, ctx, tool).await;

        if let Some(tracker) = &ctx.trust_tracker {
            if result.success {
                tracker.record_success(tool_name);
            } else {
                tracker.record_violation(tool_name);
            }
        }

        for middleware in self.middleware {
            middleware.after_execute(tool_name, &mut result, ctx).await;
        }

        emit_tool_execution_audit(ctx, tool_name, &args_summary, &result).await;
        Ok(result)
    }

    async fn before_execute(
        &self,
        tool_name: &str,
        args: &Value,
        ctx: &ExecutionContext,
    ) -> Result<PreExecutionOutcome> {
        for middleware in self.middleware {
            match middleware.before_execute(tool_name, args, ctx).await? {
                MiddlewareDecision::Continue => {}
                MiddlewareDecision::Block(reason) => {
                    return Ok(PreExecutionOutcome::Return(Box::new(blocked_tool_result(
                        reason,
                    ))));
                }
                MiddlewareDecision::RequireApproval(intent) => {
                    match resolve_action_intent(ctx, &intent).await? {
                        ApprovalResolution::Approved => {}
                        ApprovalResolution::Denied { reason } => {
                            return Ok(PreExecutionOutcome::Return(Box::new(blocked_tool_result(
                                format!("tool execution denied by approval broker: {reason}"),
                            ))));
                        }
                    }
                }
            }
        }

        Ok(PreExecutionOutcome::Continue)
    }
}

/// Build an [`ApprovalRequest`] from an [`ActionIntent`].
///
/// Extracts `args_summary` from `intent.payload["args_summary"]`, derives
/// the channel name from the first segment of the operator string
/// (e.g. `"discord:user-1"` → `"discord"`), and classifies the risk level
/// using [`classify_risk_args`].
#[must_use]
pub fn approval_request_from_intent(intent: &ActionIntent) -> ApprovalRequest {
    let tool_name = intent.action_kind.clone();
    let args_summary = intent
        .payload
        .get("args_summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let entity_id = crate::contracts::ids::EntityId::new(intent.operator.clone());
    let channel = entity_id
        .as_str()
        .split(':')
        .next()
        .unwrap_or("unknown")
        .to_string();

    ApprovalRequest {
        intent_id: intent.intent_id.clone(),
        tool_name: tool_name.clone(),
        args_summary,
        risk_level: classify_risk_args(&tool_name, &intent.payload),
        entity_id,
        channel,
    }
}

/// Resolve approval using broker + permission cache semantics shared across tools.
///
/// # Errors
///
/// Returns an error when approval is required but no broker is available or
/// the broker itself fails.
pub async fn request_approval_with_cache(
    ctx: &ExecutionContext,
    request: &ApprovalRequest,
    cache_tool_name: &str,
    cache_args_summary: &str,
) -> Result<ApprovalResolution> {
    if let Some(permission_store) = &ctx.permission_store {
        permission_store.set_entity_allowlist(ctx.entity_id.as_str(), ctx.allowed_tools.clone());
        if permission_store.is_granted(
            cache_tool_name,
            cache_args_summary,
            ctx.entity_id.as_str(),
            &ctx.tenant_context,
        ) {
            return Ok(ApprovalResolution::Approved);
        }
    }

    let Some(broker) = &ctx.approval_broker else {
        bail!(
            "tool execution requires approval: action_kind='{}' intent_id='{}'",
            request.tool_name,
            request.intent_id
        );
    };

    match broker.request_approval(request).await? {
        ApprovalDecision::Approved => {
            let mut governance = evaluate_approval_governance(ctx, request);
            if governance.verdict == AutonomyVerdict::Deny {
                governance.verdict = AutonomyVerdict::Warn;
                governance.rationale = format!(
                    "{}; explicit operator approval overrode the default deny path",
                    governance.rationale
                );
            }
            emit_governance_audit(ctx, request, &governance).await;
            Ok(ApprovalResolution::Approved)
        }
        ApprovalDecision::ApprovedWithGrant(grant) => {
            let mut governance = evaluate_approval_governance(ctx, request);
            if governance.verdict == AutonomyVerdict::Deny {
                governance.verdict = AutonomyVerdict::Warn;
                governance.rationale = format!(
                    "{}; explicit operator approval overrode the default deny path",
                    governance.rationale
                );
            }
            emit_governance_audit(ctx, request, &governance).await;
            if let Some(permission_store) = &ctx.permission_store {
                permission_store
                    .add_grant(grant, ctx.entity_id.as_str(), &ctx.tenant_context)
                    .with_context(|| {
                        format!(
                            "failed to persist approval grant for entity '{}'",
                            ctx.entity_id
                        )
                    })?;
            }
            Ok(ApprovalResolution::Approved)
        }
        ApprovalDecision::Denied { reason } => {
            let mut governance = evaluate_approval_governance(ctx, request);
            governance.verdict = AutonomyVerdict::Deny;
            governance.rationale.clone_from(&reason);
            emit_governance_audit(ctx, request, &governance).await;
            Ok(ApprovalResolution::Denied { reason })
        }
    }
}

fn evaluate_approval_governance(
    ctx: &ExecutionContext,
    request: &ApprovalRequest,
) -> GovernanceDecision {
    let trust_level = ctx
        .trust_tracker
        .as_ref()
        .map_or(TrustLevel::FirstSeen, |tracker| {
            let trust = tracker.get_trust(&request.tool_name);
            if trust.success_count == 0 && trust.violation_count == 0 {
                TrustLevel::FirstSeen
            } else if trust.score < 0.35 {
                TrustLevel::Sandboxed
            } else if trust.score < 0.6 {
                TrustLevel::Restricted
            } else if trust.score < 0.85 {
                TrustLevel::Trusted
            } else {
                TrustLevel::Verified
            }
        });

    evaluate_governance(GovernanceTrustState {
        trust_level,
        risk_level: request.risk_level,
        taint_labels: Vec::new(),
    })
}

async fn emit_governance_audit(
    ctx: &ExecutionContext,
    request: &ApprovalRequest,
    decision: &GovernanceDecision,
) {
    let audit = GovernanceAuditRecord {
        request_id: RequestId::new(request.intent_id.clone()),
        decision: decision.clone(),
        context: GovernanceAuditContext {
            actor: request.entity_id.to_string(),
            action: request.tool_name.clone(),
            channel: request.channel.clone(),
        },
    };
    if let Some(sink) = &ctx.execution_audit_sink {
        sink.record_governance_approval(&audit).await;
    }
    tracing::info!(governance_audit = ?audit, "governance approval audit recorded");
}

/// Enforce shell-specific guardrails from the shared orchestration layer.
///
/// # Errors
///
/// Returns an error if process isolation or command policy blocks execution.
pub fn enforce_shell_command_guardrails(
    ctx: &ExecutionContext,
    command: &str,
    route_marker: &str,
) -> Result<()> {
    if ctx.process_isolation != GroupIsolationLevel::Shared {
        let group = ctx.routing_group.as_deref().unwrap_or("default");
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}process-isolated group '{group}' forbids shell execution \
             (route='{route_marker}')"
        );
    }

    if !ctx
        .security
        .is_command_allowed_in_workspace(command, &ctx.workspace_dir)
    {
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}command not allowed: {command} (route='{route_marker}')"
        );
    }

    Ok(())
}

/// Enforce shared process/network guardrails for direct child-process spawns.
///
/// # Errors
///
/// Returns an error if isolation rules or spawn policy reject the command.
pub fn enforce_process_spawn_guardrails(
    ctx: &ExecutionContext,
    command: &str,
    args: &[String],
    route_marker: &str,
    class: ProcessSpawnClass,
) -> Result<()> {
    if class == ProcessSpawnClass::ToolEquivalent
        && ctx.process_isolation != GroupIsolationLevel::Shared
    {
        let group = ctx.routing_group.as_deref().unwrap_or("default");
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}process-isolated group '{group}' forbids direct process spawn \
             (route='{route_marker}', class='tool_equivalent')"
        );
    }

    if class == ProcessSpawnClass::ExternalConnector
        && ctx.network_isolation != GroupIsolationLevel::Shared
    {
        let group = ctx.routing_group.as_deref().unwrap_or("default");
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}network-isolated group '{group}' forbids external connector spawn \
             (route='{route_marker}', class='external_connector')"
        );
    }

    enforce_process_spawn_policy_with_args(&ctx.security, command, args, route_marker, class)
}

async fn resolve_action_intent(
    ctx: &ExecutionContext,
    intent: &ActionIntent,
) -> Result<ApprovalResolution> {
    let request = approval_request_from_intent(intent);
    request_approval_with_cache(ctx, &request, &request.tool_name, &request.args_summary).await
}

async fn emit_tool_execution_audit(
    ctx: &ExecutionContext,
    tool_name: &str,
    args_summary: &str,
    result: &ToolResult,
) {
    let Some(sink) = &ctx.execution_audit_sink else {
        return;
    };
    let record = ToolExecutionAuditRecord {
        tool_name: tool_name.to_string(),
        args_summary: args_summary.to_string(),
        success: result.success,
        summary: crate::utils::text::truncate_ellipsis(
            &tool_execution_summary(result),
            TOOL_EXECUTION_AUDIT_SUMMARY_LIMIT,
        ),
    };
    sink.record_tool_execution(&record).await;
}

async fn execute_with_retry(
    tool_name: &str,
    args: &Value,
    ctx: &ExecutionContext,
    tool: &Arc<dyn Tool>,
) -> ToolResult {
    let mut attempt = 1usize;
    let mut backoff_ms = INITIAL_TRANSIENT_BACKOFF_MS;

    loop {
        match tool.execute(args.clone(), ctx).await {
            Ok(result) => return result,
            Err(error) => {
                let retryable = is_retryable_tool_error(&error);
                let should_retry = retryable && attempt < MAX_TRANSIENT_TOOL_ATTEMPTS;

                if should_retry {
                    tracing::warn!(
                        tool = tool_name,
                        attempt,
                        max_attempts = MAX_TRANSIENT_TOOL_ATTEMPTS,
                        backoff_ms,
                        error = %error,
                        "transient tool execution failed, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms.saturating_mul(2).min(MAX_TRANSIENT_BACKOFF_MS);
                    attempt = attempt.saturating_add(1);
                    continue;
                }

                if retryable && attempt > 1 {
                    tracing::warn!(
                        tool = tool_name,
                        attempts = attempt,
                        error = %error,
                        "tool execution escalated after retry exhaustion"
                    );
                }

                let message = if retryable && attempt > 1 {
                    format!("tool execution failed after {attempt} attempt(s): {error}")
                } else {
                    error.to_string()
                };

                return ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(message),
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                };
            }
        }
    }
}

fn is_retryable_tool_error(error: &anyhow::Error) -> bool {
    let lower = error.to_string().to_ascii_lowercase();
    [
        "timeout",
        "timed out",
        "temporarily unavailable",
        "temporary failure",
        "try again",
        "connection reset",
        "connection refused",
        "broken pipe",
        "service unavailable",
        "deadline exceeded",
        "econnreset",
        "econnrefused",
        "429",
        "rate limit",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

fn blocked_tool_result(reason: String) -> ToolResult {
    ToolResult {
        success: false,
        output: String::new(),
        error: Some(reason),
        attachments: Vec::new(),
        taint_labels: Vec::new(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}

fn tool_execution_summary(result: &ToolResult) -> String {
    if let Some(error) = &result.error
        && !error.trim().is_empty()
    {
        return error.trim().to_string();
    }
    if !result.output.trim().is_empty() {
        return result
            .output
            .lines()
            .next()
            .map_or_else(String::new, |line| line.trim().to_string());
    }
    if result.success {
        "ok".to_string()
    } else {
        "failed".to_string()
    }
}
