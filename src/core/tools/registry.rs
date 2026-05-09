//! Tool registry: named-tool store with full middleware-chain dispatch.
//!
//! [`ToolRegistry`] is the single entry point for all tool invocations made
//! by the agent loop.  Calling [`ToolRegistry::execute`] runs the request
//! through the full middleware pipeline (security → rate-limit → audit →
//! compaction → truncation → sanitisation → secret-scrub → taint) and then
//! delegates to the concrete [`Tool`] implementation.
//!
//! After a successful execution the registry also:
//! - emits a structured tool-execution event to the memory backend
//!   (when `ctx.memory` is set), and
//! - writes an audit record to the [`ToolExecutionAuditSink`]
//!   (when `ctx.execution_audit_sink` is set).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::contracts::observability::ObserverEvent;
use crate::core::memory::build_tool_execution_event;
use crate::core::tools::middleware::{
    ExecutionContext, ToolExecutionAuditRecord, ToolExecutionOrchestrator, ToolMiddleware,
};
use crate::core::tools::traits::{Tool, ToolResult, ToolSpec};
use crate::security::summarize_args;

const TOOL_EXECUTION_AUDIT_SUMMARY_LIMIT: usize = 180;

/// Central store of named tools with middleware-chain dispatch.
///
/// Build one registry per surface (gateway, Discord, CLI, …) so that
/// each surface can carry its own middleware configuration.  Tools are
/// shared across invocations via `Arc<dyn Tool>` and must be stateless.
#[derive(Default)]
pub struct ToolRegistry {
    /// Registered tools, keyed by their canonical name.
    tools: HashMap<String, Arc<dyn Tool>>,
    /// Ordered middleware chain run before and after each tool execution.
    middleware: Vec<Arc<dyn ToolMiddleware>>,
}

impl ToolRegistry {
    /// Create a registry with the given middleware pipeline.
    #[must_use]
    pub fn new(middleware: Vec<Arc<dyn ToolMiddleware>>) -> Self {
        Self {
            tools: HashMap::new(),
            middleware,
        }
    }

    /// Register a tool, replacing any existing tool with the same name.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let tool: Arc<dyn Tool> = Arc::from(tool);
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Remove a tool by name; returns `true` if it was present.
    pub fn unregister(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    /// Look up a registered tool by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Return all registered tool names in sorted order.
    pub fn tool_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tools.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Return specs for all registered tools.
    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec()).collect()
    }

    /// Return specs filtered by the context's allowed-tools whitelist.
    #[must_use]
    pub fn specs_for_context(&self, ctx: &ExecutionContext) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .filter(|(name, _)| {
                ctx.allowed_tools
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(*name))
            })
            .map(|(_, tool)| tool.spec())
            .collect()
    }

    /// Execute a named tool through the full middleware pipeline.
    ///
    /// If the tool is not registered, returns a failed [`ToolResult`] rather
    /// than an `Err` so the agent loop can surface the message to the model.
    ///
    /// On success, a tool-execution event is appended to the memory backend
    /// (when `ctx.memory` is set).  The event is best-effort: a write failure
    /// is logged at `DEBUG` level and does not fail the call.
    ///
    /// # Errors
    ///
    /// Returns `Err` only when the orchestration layer itself fails (e.g. an
    /// approval broker returns an I/O error or middleware panics).
    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        let args_summary = summarize_args(name, &args);
        let start = std::time::Instant::now();
        let Some(tool) = self.tools.get(name) else {
            let result = ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Tool not found: {name}")),

                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            };
            emit_tool_execution_audit(ctx, name, &args_summary, &result).await;
            return Ok(result);
        };
        let result = ToolExecutionOrchestrator::new(&self.middleware)
            .execute(name, args, ctx, tool)
            .await?;
        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        emit_tool_execution_memory_event(ctx, name, &args_summary, &result, duration_ms).await;
        Ok(result)
    }
}

async fn emit_tool_execution_memory_event(
    ctx: &ExecutionContext,
    tool_name: &str,
    args_summary: &str,
    result: &ToolResult,
    duration_ms: u64,
) {
    let Some(memory) = &ctx.memory else {
        return;
    };
    if !result.success {
        return;
    }
    let result_summary =
        crate::utils::text::truncate_ellipsis(&tool_execution_summary(result), 500);
    let session_id = ctx.session_id.as_deref().unwrap_or("unknown");
    let event = build_tool_execution_event(
        ctx.entity_id.as_str(),
        tool_name,
        args_summary,
        &result_summary,
        duration_ms,
        session_id,
    );
    if let Err(error) = memory.append_event(event).await {
        let message = format!("tool execution memory event failed: {error}");
        tracing::warn!(%error, tool = tool_name, "tool execution memory event failed");
        ctx.observer.record_event(&ObserverEvent::Error {
            component: "tool_execution_memory_event".to_string(),
            message,
        });
    }
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
        summary: truncate_tool_audit_summary(&tool_execution_summary(result)),
    };
    sink.record_tool_execution(&record).await;
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

fn truncate_tool_audit_summary(input: &str) -> String {
    crate::utils::text::truncate_ellipsis(input, TOOL_EXECUTION_AUDIT_SUMMARY_LIMIT)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;
    use tempfile::{NamedTempFile, TempDir};

    use super::*;
    use crate::contracts::observability::{Observer, ObserverEvent, ObserverMetric};
    use crate::core::memory::{MarkdownMemory, Memory};
    use crate::core::tools::middleware::{
        ExecutionContext, MiddlewareDecision, SecurityMiddleware,
    };
    use crate::core::tools::traits::ActionIntent;
    use crate::security::{
        ApprovalBroker, ApprovalDecision, ApprovalRequest, GrantScope, PermissionGrant,
        PermissionStore, RiskLevel, SecurityPolicy,
    };

    #[derive(Debug)]
    struct TestTool;

    impl Tool for TestTool {
        fn name(&self) -> &'static str {
            "test_tool"
        }

        fn description(&self) -> &'static str {
            "test"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        fn execute<'a>(
            &'a self,
            _args: Value,
            _ctx: &'a ExecutionContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<ToolResult>> + Send + 'a>,
        > {
            Box::pin(async move {
                Ok(ToolResult {
                    success: true,
                    output: "ok".to_string(),
                    error: None,

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                })
            })
        }
    }

    #[derive(Debug)]
    struct FlakyTool {
        calls: Arc<AtomicUsize>,
        fail_count: usize,
    }

    impl Tool for FlakyTool {
        fn name(&self) -> &'static str {
            "flaky_tool"
        }

        fn description(&self) -> &'static str {
            "flaky"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        fn execute<'a>(
            &'a self,
            _args: Value,
            _ctx: &'a ExecutionContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<ToolResult>> + Send + 'a>,
        > {
            Box::pin(async move {
                let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt <= self.fail_count {
                    anyhow::bail!("temporarily unavailable");
                }

                Ok(ToolResult {
                    success: true,
                    output: format!("ok on attempt {attempt}"),
                    error: None,
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                })
            })
        }
    }

    #[derive(Debug)]
    struct BlockAllMiddleware;

    impl ToolMiddleware for BlockAllMiddleware {
        fn before_execute<'a>(
            &'a self,
            _tool_name: &'a str,
            _args: &'a Value,
            _ctx: &'a ExecutionContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>,
        > {
            Box::pin(async move { Ok(MiddlewareDecision::Block("blocked".to_string())) })
        }

        fn after_execute<'a>(
            &'a self,
            _tool_name: &'a str,
            _result: &'a mut ToolResult,
            _ctx: &'a ExecutionContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {})
        }
    }

    #[derive(Debug)]
    struct RequireApprovalMiddleware;

    impl ToolMiddleware for RequireApprovalMiddleware {
        fn before_execute<'a>(
            &'a self,
            _tool_name: &'a str,
            _args: &'a Value,
            _ctx: &'a ExecutionContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>,
        > {
            Box::pin(async move {
                Ok(MiddlewareDecision::RequireApproval(ActionIntent::new(
                    "shell",
                    "discord:user-1",
                    json!({"args_summary": "cargo test"}),
                )))
            })
        }

        fn after_execute<'a>(
            &'a self,
            _tool_name: &'a str,
            _result: &'a mut ToolResult,
            _ctx: &'a ExecutionContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {})
        }
    }

    #[derive(Debug)]
    struct CountingBroker {
        calls: Arc<AtomicUsize>,
        decision: ApprovalDecision,
        last_request: Arc<Mutex<Option<ApprovalRequest>>>,
    }

    #[derive(Default)]
    struct RecordingObserver {
        events: Mutex<Vec<ObserverEvent>>,
    }

    impl Observer for RecordingObserver {
        fn record_event(&self, event: &ObserverEvent) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event.clone());
        }

        fn record_metric(&self, _metric: &ObserverMetric) {}

        fn name(&self) -> &str {
            "recording"
        }
    }

    impl ApprovalBroker for CountingBroker {
        fn request_approval<'a>(
            &'a self,
            request: &'a ApprovalRequest,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<ApprovalDecision>> + Send + 'a>,
        > {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let mut slot = self
                    .last_request
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *slot = Some(request.clone());
                Ok(self.decision.clone())
            })
        }
    }

    #[tokio::test]
    async fn execute_runs_middleware_chain() {
        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security);
        let mut registry = ToolRegistry::new(vec![Arc::new(BlockAllMiddleware)]);
        registry.register(Box::new(TestTool));

        let result = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("blocked"));
    }

    #[tokio::test]
    async fn tool_memory_event_failure_records_observer_error() {
        let temp_file = NamedTempFile::new().expect("temp file");
        let security = Arc::new(SecurityPolicy::default());
        let observer = Arc::new(RecordingObserver::default());
        let mut ctx = ExecutionContext::test_default(security);
        ctx.memory = Some(Arc::new(MarkdownMemory::new(temp_file.path())) as Arc<dyn Memory>);
        ctx.observer = Arc::clone(&observer) as Arc<dyn Observer>;
        ctx.session_id = Some("session-1".to_string());

        let mut registry = ToolRegistry::new(vec![]);
        registry.register(Box::new(TestTool));

        let result = registry
            .execute("test_tool", json!({"input":"ok"}), &ctx)
            .await
            .expect("tool execution should still succeed");
        assert!(result.success);

        let events = observer
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(events.iter().any(|event| matches!(
            event,
            ObserverEvent::Error { component, .. }
                if component == "tool_execution_memory_event"
        )));
    }

    #[tokio::test]
    async fn specs_for_context_filters_allowed_tools() {
        let security = Arc::new(SecurityPolicy::default());
        let mut ctx = ExecutionContext::test_default(security);
        ctx.allowed_tools = Some(std::collections::HashSet::from(["test_tool".to_string()]));

        let mut registry = ToolRegistry::new(vec![]);
        registry.register(Box::new(TestTool));

        let specs = registry.specs_for_context(&ctx);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "test_tool");
    }

    #[tokio::test]
    async fn execute_with_security_middleware_blocks_readonly() {
        let security = Arc::new(SecurityPolicy::default());
        let mut ctx = ExecutionContext::test_default(security);
        ctx.autonomy_level = crate::security::AutonomyLevel::ReadOnly;

        let mut registry = ToolRegistry::new(vec![Arc::new(SecurityMiddleware)]);
        registry.register(Box::new(TestTool));

        let result = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|msg| msg.contains("read-only"))
        );
    }

    #[tokio::test]
    async fn execute_require_approval_without_broker_returns_error() {
        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security);
        let mut registry = ToolRegistry::new(vec![Arc::new(RequireApprovalMiddleware)]);
        registry.register(Box::new(TestTool));

        let error = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("requires approval"));
    }

    #[tokio::test]
    async fn execute_require_approval_with_broker_approved_executes_tool() {
        let security = Arc::new(SecurityPolicy::default());
        let mut ctx = ExecutionContext::test_default(security);
        ctx.approval_broker = Some(Arc::new(CountingBroker {
            calls: Arc::new(AtomicUsize::new(0)),
            decision: ApprovalDecision::Approved,
            last_request: Arc::new(Mutex::new(None)),
        }));

        let mut registry = ToolRegistry::new(vec![Arc::new(RequireApprovalMiddleware)]);
        registry.register(Box::new(TestTool));

        let result = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "ok");
    }

    #[tokio::test]
    async fn execute_require_approval_with_broker_denied_returns_tool_result_error() {
        let security = Arc::new(SecurityPolicy::default());
        let mut ctx = ExecutionContext::test_default(security);
        ctx.approval_broker = Some(Arc::new(CountingBroker {
            calls: Arc::new(AtomicUsize::new(0)),
            decision: ApprovalDecision::Denied {
                reason: "operator rejected".to_string(),
            },
            last_request: Arc::new(Mutex::new(None)),
        }));

        let mut registry = ToolRegistry::new(vec![Arc::new(RequireApprovalMiddleware)]);
        registry.register(Box::new(TestTool));

        let result = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|msg| msg.contains("operator rejected"))
        );
    }

    #[tokio::test]
    async fn approval_request_mapping_uses_intent_action_kind_entity_and_risk() {
        let security = Arc::new(SecurityPolicy::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let last_request = Arc::new(Mutex::new(None));
        let mut ctx = ExecutionContext::test_default(security);
        ctx.approval_broker = Some(Arc::new(CountingBroker {
            calls: Arc::clone(&calls),
            decision: ApprovalDecision::Approved,
            last_request: Arc::clone(&last_request),
        }));

        let mut registry = ToolRegistry::new(vec![Arc::new(RequireApprovalMiddleware)]);
        registry.register(Box::new(TestTool));

        let _ = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let request = last_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap();
        assert_eq!(request.tool_name, "shell");
        assert_eq!(request.entity_id.as_str(), "discord:user-1");
        assert_eq!(request.channel, "discord");
        assert_eq!(request.risk_level, RiskLevel::High);
        assert_eq!(request.args_summary, "cargo test");
    }

    #[tokio::test]
    async fn execute_with_grant_stores_permission_and_future_call_skips_broker() {
        let security = Arc::new(SecurityPolicy::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let temp_dir = TempDir::new().unwrap();
        let permission_store = Arc::new(PermissionStore::load(temp_dir.path()));
        let mut ctx = ExecutionContext::test_default(security);
        ctx.permission_store = Some(Arc::clone(&permission_store));
        ctx.approval_broker = Some(Arc::new(CountingBroker {
            calls: Arc::clone(&calls),
            decision: ApprovalDecision::ApprovedWithGrant(PermissionGrant {
                tool: "test_tool".to_string(),
                pattern: "*".to_string(),
                scope: GrantScope::Session,
            }),
            last_request: Arc::new(Mutex::new(None)),
        }));

        let mut registry = ToolRegistry::new(vec![Arc::new(SecurityMiddleware)]);
        registry.register(Box::new(TestTool));

        let first = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();
        assert!(first.success);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(permission_store.is_granted(
            "test_tool",
            "{}",
            ctx.entity_id.as_str(),
            &ctx.tenant_context,
        ));

        let second = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();
        assert!(second.success);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn execute_retries_transient_tool_errors_and_returns_success() {
        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new(vec![]);
        registry.register(Box::new(FlakyTool {
            calls: Arc::clone(&calls),
            fail_count: 1,
        }));

        let result = registry
            .execute("flaky_tool", json!({}), &ctx)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output, "ok on attempt 2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn execute_converts_retry_exhaustion_into_failed_tool_result() {
        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new(vec![]);
        registry.register(Box::new(FlakyTool {
            calls: Arc::clone(&calls),
            fail_count: usize::MAX,
        }));

        let result = registry
            .execute("flaky_tool", json!({}), &ctx)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("failed after 3 attempt(s)"))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn execute_records_successful_tool_runs_to_memory_when_available() {
        let security = Arc::new(SecurityPolicy::default());
        let temp_dir = TempDir::new().unwrap();
        let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp_dir.path()));
        let mut ctx = ExecutionContext::test_default(security);
        ctx.memory = Some(memory.clone());
        ctx.session_id = Some("session-1".to_string());

        let mut registry = ToolRegistry::new(vec![]);
        registry.register(Box::new(TestTool));

        let result = registry
            .execute("test_tool", json!({}), &ctx)
            .await
            .unwrap();

        assert!(result.success);
        let memory_dir = temp_dir.path().join("memory");
        let persisted = std::fs::read_dir(&memory_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
            .any(|content| content.contains("tool.execution.test_tool"));
        assert!(persisted);
    }
}
