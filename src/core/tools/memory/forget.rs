//! Memory forget tool — removes or tombstones a belief slot.
//!
//! # What it does
//!
//! `memory_forget` delegates to `Memory::forget_slot` with one of three modes:
//!
//! * `soft` (default) — asks the backend to mark the slot as deleted while
//!   keeping history.
//! * `hard` — asks the backend to purge slot events when that backend supports
//!   physical deletion.
//! * `tombstone` — asks the backend to write a permanent deletion marker so the
//!   slot cannot be recreated under the same key.
//!
//! A missing slot, or a backend outcome that reports no applied mutation, is not
//! propagated as an error; the tool returns a successful no-op message so the
//! agent can continue instead of crashing. Append-only backends such as
//! `MarkdownMemory` may therefore report no applied mutation for `hard` or
//! `tombstone` requests.
//!
//! # Security surface
//!
//! Entity scope is validated by `policy_context::effective_tenant_policy_context`
//! before deletion proceeds. In tenant mode, an agent is prohibited from
//! forgetting slots that belong to other tenants. Failed scope checks surface
//! as a non-success `ToolResult` rather than a hard error.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::core::memory::{ForgetMode, Memory, ensure_forget_mode_supported};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that applies soft, hard, or tombstone deletion to a belief slot.
pub struct MemoryForgetTool {
    memory: Arc<dyn Memory>,
}

impl MemoryForgetTool {
    /// Create a new memory-forget tool backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl Tool for MemoryForgetTool {
    fn name(&self) -> &'static str {
        "memory_forget"
    }

    fn description(&self) -> &'static str {
        "Apply soft/hard/tombstone forgetting on an entity slot."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "slot_key": {
                    "type": "string",
                    "description": "Slot key to forget"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity id owning the slot"
                },
                "mode": {
                    "type": "string",
                    "enum": ["soft", "hard", "tombstone"],
                    "description": "Deletion lifecycle mode"
                },
                "reason": {
                    "type": "string",
                    "description": "Deletion reason for audit"
                },
                "policy_context": {
                    "type": "object",
                    "description": "Optional tenant policy context to validate forget scope",
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
            "required": ["entity_id", "slot_key"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let key = args
                .get("slot_key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'slot_key' parameter"))?;

            let entity_id = args
                .get("entity_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'entity_id' parameter"))?;
            let reason = args
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("user_requested");

            let policy_context =
                super::policy_context::effective_tenant_policy_context(&args, ctx)?;
            if let Err(error) = policy_context.enforce_recall_scope(entity_id) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to forget memory: {error}")),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            let mode = match args.get("mode").and_then(|v| v.as_str()) {
                Some("hard") => ForgetMode::Hard,
                Some("tombstone") => ForgetMode::Tombstone,
                _ => ForgetMode::Soft,
            };

            if let Err(error) = ensure_forget_mode_supported(self.memory.as_ref(), mode) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to forget memory: {error}")),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            match self.memory.forget_slot(entity_id, key, mode, reason).await {
                Ok(outcome) if outcome.was_applied => {
                    if let Some(log) = &ctx.memory_access_log {
                        log.record_access(
                            entity_id,
                            crate::core::memory::governance::MemoryAccessType::Delete,
                            Some(key),
                        );
                        log.record_transition(
                            crate::core::memory::governance::MemoryState::Active,
                            crate::core::memory::governance::MemoryState::Deleted,
                            entity_id,
                        );
                    }
                    Ok(ToolResult {
                        success: true,
                        output: format!("Forgot slot: {key}"),
                        error: None,

                        attachments: Vec::new(),
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    })
                }
                Ok(_) => Ok(ToolResult {
                    success: true,
                    output: format!("No memory found with key: {key}"),
                    error: None,

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to forget memory: {e}")),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{
        MarkdownMemory, MemoryEventInput, MemoryEventType, MemorySource, PrivacyLevel,
    };
    use crate::core::tools::middleware::ExecutionContext;

    fn test_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        (tmp, Arc::new(mem))
    }

    #[test]
    fn name_and_schema() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem);
        assert_eq!(tool.name(), "memory_forget");
        assert!(tool.parameters_schema()["properties"]["slot_key"].is_object());
    }

    #[cfg(feature = "postgres")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn forget_existing() {
        use std::time::Duration;

        use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
        use crate::core::memory::postgres::{PostgresConnectOptions, PostgresMemory};

        struct StubEmbedding;

        impl EmbeddingProvider for StubEmbedding {
            fn name(&self) -> &'static str {
                "stub_forget_test"
            }
            fn dimensions(&self) -> usize {
                3
            }
            fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
                Box::pin(async move { Ok(texts.iter().map(|_| vec![0.0, 0.0, 0.0]).collect()) })
            }
        }

        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let database_url = std::env::var("ASTEREL_POSTGRES_URL").expect("postgres url must be set");
        let mem: Arc<dyn Memory> = Arc::new(
            PostgresMemory::connect_with_options(
                &database_url,
                Arc::new(StubEmbedding),
                PostgresConnectOptions {
                    cache_max: 16,
                    graph_retrieval_fusion_enabled: false,
                    graph_retrieval_weight: 0.0,
                    max_connections: 4,
                    min_connections: 1,
                    connect_timeout: Duration::from_secs(5),
                    idle_timeout: Duration::from_secs(30),
                    vector_weight: 0.7,
                    keyword_weight: 0.3,
                    max_lifetime: Duration::from_secs(60),
                    hnsw_ef_search: 0,
                },
            )
            .await
            .expect("connect postgres memory"),
        );

        let entity_id = format!("person:forget-{}", uuid::Uuid::new_v4().simple());

        mem.append_event(
            MemoryEventInput::new(
                &entity_id,
                "temp",
                MemoryEventType::FactAdded,
                "temporary",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.6),
        )
        .await
        .unwrap();

        let tool = MemoryForgetTool::new(mem.clone());
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({"entity_id": &entity_id, "slot_key": "temp", "mode": "hard"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.success,
            "forget tool should succeed: {:?}",
            result.error
        );
        assert!(
            result.output.contains("Forgot"),
            "expected output to contain 'Forgot': {}",
            result.output
        );

        assert!(
            mem.resolve_slot(&entity_id, "temp")
                .await
                .unwrap()
                .is_none(),
            "slot should be gone after hard forget"
        );
    }

    #[tokio::test]
    async fn forget_nonexistent() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(json!({"entity_id": "default", "slot_key": "nope"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No memory found"));
    }

    #[tokio::test]
    async fn forget_missing_key() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool.execute(json!({"entity_id": "default"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn forget_hard_mode_checks_backend_capability_before_delete() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));

        let result = tool
            .execute(
                json!({"entity_id": "default", "slot_key": "temp", "mode": "hard"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("does not support forget mode 'hard'")),
            "expected capability error, got {:?}",
            result.error
        );
    }
}
