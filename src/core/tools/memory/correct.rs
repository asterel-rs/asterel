//! Memory correct tool — amends a belief slot after confirming the prior value.
//!
//! # What it does
//!
//! `memory_correct` applies an optimistic-locking correction: the caller must
//! supply the expected `old_value` (or a substring of it). The tool resolves
//! the current slot, checks that `old_value` appears in the live value
//! (case-insensitive, trimmed), and only then appends a `FactUpdated` event
//! with the new value. If the check fails — because the slot no longer holds
//! the expected content — the write is rejected and the slot is left unchanged.
//!
//! # Security surface
//!
//! Like `memory_store`, every write passes through
//! `enforce_tool_memory_write_policy` before the backend is touched.
//! Entity scope is also checked via `policy_context::enforce_entity_scope`
//! to prevent cross-tenant writes when tenant mode is active.
//!
//! The correction is recorded with `SourceKind::Manual` and a provenance
//! reference of `"tool.memory_correct:<reason>"` to maintain a clear audit
//! trail of why the slot value changed.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::core::memory::{
    BeliefSlot, Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};
use crate::security::writeback_guard::enforce_tool_memory_write_policy;

/// Tool that amends a belief slot using an optimistic-locking check on the prior value.
pub struct MemoryCorrectTool {
    memory: Arc<dyn Memory>,
}

struct CorrectionArgs<'a> {
    entity_id: &'a str,
    slot_key: &'a str,
    old_value: &'a str,
    new_value: &'a str,
    reason: &'a str,
}

impl MemoryCorrectTool {
    /// Create a new memory-correct tool backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// Return `true` when `needle` (trimmed, lowercased) appears anywhere in
    /// `haystack` (trimmed, lowercased). An empty needle always returns `false`
    /// so that a caller supplying `old_value: ""` cannot accidentally match
    /// every slot.
    fn normalized_contains(haystack: &str, needle: &str) -> bool {
        let haystack = haystack.trim().to_lowercase();
        let needle = needle.trim().to_lowercase();
        !needle.is_empty() && haystack.contains(&needle)
    }

    fn parse_args(args: &serde_json::Value) -> anyhow::Result<CorrectionArgs<'_>> {
        Ok(CorrectionArgs {
            entity_id: args
                .get("entity_id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'entity_id' parameter"))?,
            slot_key: args
                .get("slot_key")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'slot_key' parameter"))?,
            old_value: args
                .get("old_value")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'old_value' parameter"))?,
            new_value: args
                .get("new_value")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'new_value' parameter"))?,
            reason: args
                .get("reason")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'reason' parameter"))?,
        })
    }

    fn failure(message: impl Into<String>) -> ToolResult {
        ToolResult::failure(message)
    }

    async fn resolve_current_slot(
        &self,
        entity_id: &str,
        slot_key: &str,
    ) -> Result<BeliefSlot, ToolResult> {
        match self.memory.resolve_slot(entity_id, slot_key).await {
            Ok(Some(slot)) => Ok(slot),
            Ok(None) => Err(Self::failure(format!(
                "Memory correction failed: no current value found for [{entity_id}:{slot_key}]"
            ))),
            Err(error) => Err(Self::failure(format!("Memory correction failed: {error}"))),
        }
    }

    fn build_correction_input(args: &CorrectionArgs<'_>) -> anyhow::Result<MemoryEventInput> {
        let input = MemoryEventInput::new(
            args.entity_id,
            args.slot_key,
            MemoryEventType::FactUpdated,
            args.new_value,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_source_kind(SourceKind::Manual)
        .with_source_ref("tool.memory_correct")
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            format!("tool.memory_correct:{}", args.reason),
        ));

        enforce_tool_memory_write_policy(&input)?;
        Ok(input)
    }
}

impl Tool for MemoryCorrectTool {
    fn name(&self) -> &'static str {
        "memory_correct"
    }

    fn description(&self) -> &'static str {
        "Correct a slot after confirming the prior value still matches."
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
                    "description": "Slot key to correct"
                },
                "old_value": {
                    "type": "string",
                    "description": "Expected prior value or substring"
                },
                "new_value": {
                    "type": "string",
                    "description": "Corrected value to write"
                },
                "reason": {
                    "type": "string",
                    "description": "Reason for the correction"
                }
            },
            "required": ["entity_id", "slot_key", "old_value", "new_value", "reason"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let args = Self::parse_args(&args)?;
            super::policy_context::enforce_entity_scope(args.entity_id, &ctx.tenant_context)?;

            let current_slot = match self
                .resolve_current_slot(args.entity_id, args.slot_key)
                .await
            {
                Ok(slot) => slot,
                Err(result) => return Ok(result),
            };
            if !Self::normalized_contains(&current_slot.value, args.old_value) {
                return Ok(Self::failure(format!(
                    "Memory correction failed: current value did not match 'old_value' for [{}:{}]",
                    args.entity_id, args.slot_key
                )));
            }

            match self
                .memory
                .append_event(Self::build_correction_input(&args)?)
                .await
            {
                Ok(event) => {
                    if let Some(log) = &ctx.memory_access_log {
                        log.record_access(
                            args.entity_id,
                            crate::core::memory::governance::MemoryAccessType::Write,
                            Some(args.slot_key),
                        );
                    }
                    Ok(ToolResult::success(format!(
                        "Corrected slot [{}:{}] and recorded event {}",
                        args.entity_id, args.slot_key, event.event_id
                    )))
                }
                Err(error) => Ok(Self::failure(format!("Memory correction failed: {error}"))),
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
        let tool = MemoryCorrectTool::new(mem);
        assert_eq!(tool.name(), "memory_correct");
        assert!(tool.parameters_schema()["properties"]["old_value"].is_object());
        assert!(tool.parameters_schema()["properties"]["new_value"].is_object());
    }

    #[tokio::test]
    async fn correct_updates_slot_when_old_value_matches() {
        let (_tmp, mem) = test_mem();
        mem.append_event(MemoryEventInput::new(
            "default",
            "profile.timezone",
            MemoryEventType::FactAdded,
            "User lives in PST timezone",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .unwrap();

        let tool = MemoryCorrectTool::new(mem.clone());
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "entity_id": "default",
                    "slot_key": "profile.timezone",
                    "old_value": "PST",
                    "new_value": "User lives in PDT timezone",
                    "reason": "DST correction"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.success);
        assert!(
            result
                .output
                .contains("Corrected slot [default:profile.timezone]")
        );

        let slot = mem
            .resolve_slot("default", "profile.timezone")
            .await
            .unwrap()
            .expect("slot should exist");
        assert_eq!(slot.value, "User lives in PDT timezone");
    }

    #[tokio::test]
    async fn correct_rejects_when_old_value_does_not_match() {
        let (_tmp, mem) = test_mem();
        mem.append_event(MemoryEventInput::new(
            "default",
            "profile.city",
            MemoryEventType::FactAdded,
            "Lives in Osaka",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .unwrap();

        let tool = MemoryCorrectTool::new(mem.clone());
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "entity_id": "default",
                    "slot_key": "profile.city",
                    "old_value": "Tokyo",
                    "new_value": "Lives in Kyoto",
                    "reason": "user corrected prior statement"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(
            result.error,
            Some(
                "Memory correction failed: current value did not match 'old_value' for [default:profile.city]"
                    .to_string()
            )
        );

        let slot = mem
            .resolve_slot("default", "profile.city")
            .await
            .unwrap()
            .expect("slot should exist");
        assert_eq!(slot.value, "Lives in Osaka");
    }

    #[tokio::test]
    async fn correct_rejects_missing_slot() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryCorrectTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "entity_id": "default",
                    "slot_key": "missing",
                    "old_value": "before",
                    "new_value": "after",
                    "reason": "cleanup"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(
            result.error,
            Some(
                "Memory correction failed: no current value found for [default:missing]"
                    .to_string()
            )
        );
    }
}
