//! Memory lookup tool — point resolution of a single belief slot.
//!
//! # What it does
//!
//! `memory_lookup` resolves the current value of one `(entity_id, slot_key)`
//! pair by replaying the event log and returning the winning slot. Unlike
//! `memory_recall`, which performs ranked full-text search across all slots,
//! this is an exact-key lookup with O(events) read complexity.
//!
//! A missing slot is not an error — the tool returns `success: true` with
//! a "No value found" message so the agent can distinguish "slot unknown"
//! from "backend failure".
//!
//! # Security surface
//!
//! Entity scope is enforced via `policy_context::enforce_entity_scope` before
//! the backend is queried, which prevents cross-tenant lookups when tenant
//! mode is active on the `ExecutionContext`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::core::memory::Memory;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that resolves the current value of one entity memory slot by exact key.
pub struct MemoryLookupTool {
    memory: Arc<dyn Memory>,
}

impl MemoryLookupTool {
    /// Create a new memory-lookup tool backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl Tool for MemoryLookupTool {
    fn name(&self) -> &'static str {
        "memory_lookup"
    }

    fn description(&self) -> &'static str {
        "Resolve the current value for one entity memory slot."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Entity id owning the slot"
                },
                "slot_key": {
                    "type": "string",
                    "description": "Slot key to resolve"
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
            let entity_id = args
                .get("entity_id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'entity_id' parameter"))?;
            let slot_key = args
                .get("slot_key")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'slot_key' parameter"))?;

            super::policy_context::enforce_entity_scope(entity_id, &ctx.tenant_context)?;

            match self.memory.resolve_slot(entity_id, slot_key).await {
                Ok(Some(slot)) => {
                    if let Some(log) = &ctx.memory_access_log {
                        log.record_access(
                            entity_id,
                            crate::core::memory::governance::MemoryAccessType::Read,
                            Some(slot_key),
                        );
                    }
                    Ok(ToolResult {
                        success: true,
                        output: format!(
                            "Resolved slot [{}:{}] = {}",
                            slot.entity_id, slot.slot_key, slot.value
                        ),
                        error: None,
                        attachments: Vec::new(),
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    })
                }
                Ok(None) => Ok(ToolResult {
                    success: true,
                    output: format!("No value found for slot [{entity_id}:{slot_key}]"),
                    error: None,
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
                Err(error) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Memory lookup failed: {error}")),
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
        let tool = MemoryLookupTool::new(mem);
        assert_eq!(tool.name(), "memory_lookup");
        assert!(tool.parameters_schema()["properties"]["entity_id"].is_object());
        assert!(tool.parameters_schema()["properties"]["slot_key"].is_object());
    }

    #[tokio::test]
    async fn lookup_returns_existing_slot() {
        let (_tmp, mem) = test_mem();
        mem.append_event(MemoryEventInput::new(
            "default",
            "favorite.language",
            MemoryEventType::FactAdded,
            "Rust",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .unwrap();

        let tool = MemoryLookupTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({"entity_id": "default", "slot_key": "favorite.language"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("favorite.language"));
        assert!(result.output.contains("Rust"));
    }

    #[tokio::test]
    async fn lookup_reports_missing_slot() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryLookupTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(json!({"entity_id": "default", "slot_key": "missing"}), &ctx)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("No value found"));
    }

    #[tokio::test]
    async fn lookup_missing_entity_id_is_rejected() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryLookupTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool.execute(json!({"slot_key": "x"}), &ctx).await;
        assert!(result.is_err());
        assert_eq!(
            result
                .expect_err("missing entity id should fail")
                .to_string(),
            "Missing 'entity_id' parameter"
        );
    }
}
