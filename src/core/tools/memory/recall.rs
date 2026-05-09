//! Memory recall tool — hybrid-ranked full-text search across belief slots.
//!
//! # What it does
//!
//! `memory_recall` issues a `RecallQuery` against the memory backend scoped to
//! a specific `entity_id`. Results use the active backend's ranking path
//! (`PostgreSQL` combines richer retrieval signals; `MarkdownMemory` uses its
//! projection keyword score) and are capped at `limit` entries (default 5,
//! maximum 20).
//!
//! # Security surface
//!
//! When `tenant_mode_enabled` is set on the `ExecutionContext`, the query is
//! restricted to the execution context's `tenant_id`. Attempts to query the
//! reserved `"default"` entity scope are blocked and surfaced as a
//! non-success `ToolResult` rather than propagated as a hard error, so the
//! agent can recover gracefully. Errors whose message begins with
//! `SECURITY_POLICY_BLOCK_PREFIX` are downgraded from `Err` to a failed
//! `ToolResult` for the same reason.

use std::fmt::Write;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::contracts::memory_error::MemoryError;
use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::core::memory::{Memory, RecallQuery};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{
    Tool, ToolResult, ToolResultCompactionTarget, ToolResultTextField,
};

const DEFAULT_RECALL_LIMIT: usize = 5;
const MAX_RECALL_LIMIT: usize = 20;

/// Tool that searches the agent's belief store using backend-provided ranking.
///
/// Results are returned in descending relevance order, formatted as
/// `[entity_id:slot_key] value [score%]` lines for easy agent parsing.
pub struct MemoryRecallTool {
    memory: Arc<dyn Memory>,
}

impl MemoryRecallTool {
    /// Create a new memory-recall tool backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    fn build_recall_request(
        args: &serde_json::Value,
        ctx: &ExecutionContext,
    ) -> anyhow::Result<RecallQuery> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

        let limit = parse_recall_limit(args)?;

        let entity_id = args
            .get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'entity_id' parameter"))?;

        let policy_context = super::policy_context::effective_tenant_policy_context(args, ctx)?;
        let request = RecallQuery::new(entity_id, query, limit).with_policy_context(policy_context);
        request.enforce_policy()?;
        Ok(request)
    }
}

fn parse_recall_limit(args: &serde_json::Value) -> anyhow::Result<usize> {
    let Some(raw_limit) = args.get("limit") else {
        return Ok(DEFAULT_RECALL_LIMIT);
    };

    let Some(limit) = raw_limit.as_u64() else {
        anyhow::bail!("Invalid 'limit' parameter: expected non-negative integer");
    };

    if limit > MAX_RECALL_LIMIT as u64 {
        anyhow::bail!("Invalid 'limit' parameter: must be {MAX_RECALL_LIMIT} or fewer");
    }

    usize::try_from(limit)
        .map_err(|_| anyhow::anyhow!("Invalid 'limit' parameter: too large for this platform"))
}

impl Tool for MemoryRecallTool {
    fn name(&self) -> &'static str {
        "memory_recall"
    }

    fn description(&self) -> &'static str {
        "Recall entity-scoped memory using hybrid ranking."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or phrase to search for in memory"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity id to scope recall"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_RECALL_LIMIT,
                    "description": "Max results to return (default: 5, maximum: 20)"
                },
                "policy_context": {
                    "type": "object",
                    "description": "Optional tenant policy context to enforce recall scope",
                    "properties": {
                        "tenant_mode_enabled": {
                            "type": "boolean"
                        },
                        "tenant_id": {
                            "type": ["string", "null"]
                        }
                    },
                    "additionalProperties": false
                }
            },
            "required": ["entity_id", "query"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let request = match Self::build_recall_request(&args, ctx) {
                Ok(request) => request,
                Err(error) => {
                    let error_text = memory_recall_error_text(&error);
                    if error_text.starts_with(SECURITY_POLICY_BLOCK_PREFIX.trim_end()) {
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some(format!("Memory recall failed: {error_text}")),

                            attachments: Vec::new(),
                            taint_labels: Vec::new(),
                            semantic:
                                crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                        });
                    }
                    return Err(error);
                }
            };

            match self.memory.recall_scoped(request).await {
                Ok(entries) if entries.is_empty() => {
                    if let Some(log) = &ctx.memory_access_log {
                        log.record_access(
                            ctx.entity_id.as_str(),
                            crate::core::memory::governance::MemoryAccessType::Search,
                            None,
                        );
                    }
                    Ok(
                        ToolResult::success("No memories found matching that query.")
                            .with_output_kind("memory.recall")
                            .with_compaction_target(ToolResultCompactionTarget::Output)
                            .with_source_fields([ToolResultTextField::Output]),
                    )
                }
                Ok(entries) => {
                    if let Some(log) = &ctx.memory_access_log {
                        log.record_access(
                            ctx.entity_id.as_str(),
                            crate::core::memory::governance::MemoryAccessType::Search,
                            None,
                        );
                    }
                    let mut output = format!("Found {} memories:\n", entries.len());
                    for entry in &entries {
                        let score = format!(" [{:.0}%]", entry.score * 100.0);
                        let _ = writeln!(
                            output,
                            "- [{}:{}] {}{score}",
                            entry.entity_id, entry.slot_key, entry.value
                        );
                    }
                    Ok(ToolResult::success(output)
                        .with_output_kind("memory.recall")
                        .with_compaction_target(ToolResultCompactionTarget::Output)
                        .with_source_fields([ToolResultTextField::Output]))
                }
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Memory recall failed: {e}")),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
            }
        })
    }
}

fn memory_recall_error_text(error: &anyhow::Error) -> String {
    if let Some(MemoryError::Policy(message)) = error.downcast_ref::<MemoryError>() {
        return message.clone();
    }
    error.to_string()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::contracts::strings::verdicts::TENANT_DEFAULT_RECALL_FORBIDDEN;
    use crate::core::memory::{
        MarkdownMemory, MemoryEventInput, MemoryEventType, MemorySource, PrivacyLevel,
    };
    use crate::core::tools::middleware::ExecutionContext;
    use crate::core::tools::traits::{ToolResultCompactionTarget, ToolResultTextField};

    fn seeded_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        (tmp, Arc::new(mem))
    }

    #[tokio::test]
    async fn recall_empty() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(json!({"entity_id": "default", "query": "anything"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No memories found"));
    }

    #[tokio::test]
    async fn recall_finds_match() {
        let (_tmp, mem) = seeded_mem();
        mem.append_event(
            MemoryEventInput::new(
                "default",
                "lang",
                MemoryEventType::FactAdded,
                "User prefers Rust",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.6),
        )
        .await
        .unwrap();
        mem.append_event(
            MemoryEventInput::new(
                "default",
                "tz",
                MemoryEventType::FactAdded,
                "Timezone is EST",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.6),
        )
        .await
        .unwrap();

        let tool = MemoryRecallTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(json!({"entity_id": "default", "query": "Rust"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Rust"));
        assert!(result.output.contains("Found 1"));
        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("memory.recall")
        );
        assert_eq!(
            result.semantic.compaction_target,
            ToolResultCompactionTarget::Output
        );
        assert_eq!(
            result
                .semantic
                .source_fields
                .iter()
                .map(|field| field.field)
                .collect::<Vec<_>>(),
            vec![ToolResultTextField::Output]
        );
        assert!(result.semantic.stats.is_some());
    }

    #[tokio::test]
    async fn recall_respects_limit() {
        let (_tmp, mem) = seeded_mem();
        for i in 0..10 {
            mem.append_event(
                MemoryEventInput::new(
                    "default",
                    format!("k{i}"),
                    MemoryEventType::FactAdded,
                    format!("Rust fact {i}"),
                    MemorySource::ExplicitUser,
                    PrivacyLevel::Private,
                )
                .with_confidence(0.95)
                .with_importance(0.6),
            )
            .await
            .unwrap();
        }

        let tool = MemoryRecallTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({"entity_id": "default", "query": "Rust", "limit": 3}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        let item_lines = result
            .output
            .lines()
            .filter(|line| line.starts_with("- "))
            .count();
        assert_eq!(item_lines, 3);
    }

    #[test]
    fn recall_limit_defaults_and_accepts_configured_max() {
        assert_eq!(
            parse_recall_limit(&json!({})).unwrap(),
            DEFAULT_RECALL_LIMIT
        );
        assert_eq!(
            parse_recall_limit(&json!({"limit": MAX_RECALL_LIMIT})).unwrap(),
            MAX_RECALL_LIMIT
        );
    }

    #[test]
    fn recall_limit_rejects_pathological_values() {
        let err = parse_recall_limit(&json!({"limit": u64::MAX}))
            .expect_err("pathological limit should be rejected")
            .to_string();

        assert!(err.contains("must be 20 or fewer"));
    }

    #[test]
    fn recall_limit_rejects_non_integer_values() {
        let err = parse_recall_limit(&json!({"limit": "20"}))
            .expect_err("string limit should be rejected")
            .to_string();

        assert!(err.contains("expected non-negative integer"));
    }

    #[tokio::test]
    async fn recall_missing_query() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn name_and_schema() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        assert_eq!(tool.name(), "memory_recall");
        assert!(tool.parameters_schema()["properties"]["query"].is_object());
    }

    #[tokio::test]
    async fn recall_rejects_default_scope_when_tenant_mode_enabled() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        let mut ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        ctx.tenant_context = crate::security::policy::TenantPolicyContext::enabled("tenant-alpha");

        let result = tool
            .execute(
                json!({
                    "entity_id": "default",
                    "query": "anything"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(
            result.error,
            Some(format!(
                "Memory recall failed: {TENANT_DEFAULT_RECALL_FORBIDDEN}"
            ))
        );
    }
}
