//! Memory store tool — appends a new immutable event to a belief slot.
//!
//! # What it does
//!
//! `memory_store` writes a single `MemoryEventInput` into the underlying
//! `Memory` backend. Each call is append-only: no existing event is mutated.
//! The slot's current value is derived at read time by replaying the event log.
//!
//! # Parameters
//!
//! Required: `entity_id`, `slot_key`, `value`. Optional: `layer`, `source`,
//! `confidence`, `importance`, `provenance`, `source_ref`, `privacy_level`.
//!
//! # Security surface
//!
//! Before touching the backend the tool calls
//! `enforce_tool_memory_write_policy`, which rejects writes whose
//! `privacy_level` is `secret` under the default security policy. Callers
//! cannot bypass this gate by omitting the field — the default privacy level
//! is `Private`, so `secret` must be stated explicitly and is always blocked.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};
use crate::security::writeback_guard::enforce_tool_memory_write_policy;

/// Tool that appends an immutable memory event to the agent's belief store.
///
/// Backed by the `Memory` trait so any storage backend (e.g. `MarkdownMemory`,
/// a future `PostgresMemory`) can be injected at construction time.
pub struct MemoryStoreTool {
    memory: Arc<dyn Memory>,
}

impl MemoryStoreTool {
    /// Create a new memory-store tool backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    fn parse_layer(args: &serde_json::Value) -> anyhow::Result<MemoryLayer> {
        let Some(layer) = args.get("layer") else {
            return Ok(MemoryLayer::Working);
        };

        let Some(layer) = layer.as_str() else {
            anyhow::bail!("Invalid 'layer' parameter: expected string");
        };

        match layer {
            "working" => Ok(MemoryLayer::Working),
            "episodic" => Ok(MemoryLayer::Episodic),
            "semantic" => Ok(MemoryLayer::Semantic),
            "procedural" => Ok(MemoryLayer::Procedural),
            "identity" => Ok(MemoryLayer::Identity),
            other => anyhow::bail!(
                "Invalid 'layer' parameter: got '{other}', must be one of working, episodic, semantic, procedural, identity"
            ),
        }
    }

    fn parse_provenance(args: &serde_json::Value) -> anyhow::Result<Option<MemoryProvenance>> {
        let Some(raw_provenance) = args.get("provenance") else {
            return Ok(None);
        };

        let Some(provenance) = raw_provenance.as_object() else {
            anyhow::bail!("Invalid 'provenance' parameter: expected object");
        };

        let source_class = provenance.get("source_class").ok_or_else(|| {
            anyhow::anyhow!("Invalid 'provenance.source_class' parameter: missing required field")
        })?;

        let Some(source_class) = source_class.as_str() else {
            anyhow::bail!("Invalid 'provenance.source_class' parameter: expected string");
        };

        let source_class = match source_class {
            "explicit_user" => MemorySource::ExplicitUser,
            "tool_verified" => MemorySource::ToolVerified,
            "system" => MemorySource::System,
            "inferred" => MemorySource::Inferred,
            other => anyhow::bail!(
                "Invalid 'provenance.source_class' parameter: got '{other}', must be one of explicit_user, tool_verified, system, inferred"
            ),
        };

        let reference = provenance.get("reference").ok_or_else(|| {
            anyhow::anyhow!("Invalid 'provenance.reference' parameter: missing required field")
        })?;

        let Some(reference) = reference.as_str() else {
            anyhow::bail!("Invalid 'provenance.reference' parameter: expected string");
        };

        if reference.trim().is_empty() {
            anyhow::bail!("Invalid 'provenance.reference' parameter: must not be empty");
        }

        let evidence_uri = match provenance.get("evidence_uri") {
            Some(serde_json::Value::Null) | None => None,
            Some(value) => {
                let Some(uri) = value.as_str() else {
                    anyhow::bail!("Invalid 'provenance.evidence_uri' parameter: expected string");
                };
                if uri.trim().is_empty() {
                    anyhow::bail!("Invalid 'provenance.evidence_uri' parameter: must not be empty");
                }
                Some(uri.to_string())
            }
        };

        Ok(Some(MemoryProvenance {
            source_class,
            reference: reference.to_string(),
            evidence_uri,
        }))
    }
}

impl Tool for MemoryStoreTool {
    fn name(&self) -> &'static str {
        "memory_store"
    }

    fn description(&self) -> &'static str {
        "Append one immutable memory event for an entity slot."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Entity identifier"
                },
                "slot_key": {
                    "type": "string",
                    "description": "Slot key"
                },
                "value": {
                    "type": "string",
                    "description": "Slot value to persist"
                },
                "event_type": {
                    "type": "string",
                    "description": "Event type (e.g. preference_set, fact_updated)"
                },
                "layer": {
                    "type": "string",
                    "enum": ["working", "episodic", "semantic", "procedural", "identity"],
                    "description": "Memory layer (defaults to working)"
                },
                "source": {
                    "type": "string",
                    "enum": ["explicit_user", "tool_verified", "system", "inferred"],
                    "description": "Event source"
                },
                "confidence": {
                    "type": "number",
                    "description": "Confidence score 0..1 (defaults by source class when omitted)"
                },
                "importance": {
                    "type": "number",
                    "description": "Importance score 0..1"
                },
                "provenance": {
                    "type": "object",
                    "description": "Optional provenance source reference envelope",
                    "properties": {
                        "source_class": {
                            "type": "string",
                            "enum": ["explicit_user", "tool_verified", "system", "inferred"]
                        },
                        "reference": {
                            "type": "string",
                            "description": "Stable source reference (ticket, event id, trace id, etc.)"
                        },
                        "evidence_uri": {
                            "type": "string",
                            "description": "Optional supporting URI"
                        }
                    },
                    "required": ["source_class", "reference"]
                },
                "source_ref": {
                    "type": "string",
                    "description": "Optional write reference for policy traceability"
                },
                "privacy_level": {
                    "type": "string",
                    "enum": ["public", "private", "secret"],
                    "description": "Privacy label"
                }
            },
            "required": ["entity_id", "slot_key", "value"]
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
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'entity_id' parameter"))?;

            super::policy_context::enforce_entity_scope(entity_id, &ctx.tenant_context)?;

            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'value' parameter"))?;

            let slot_key = args
                .get("slot_key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'slot_key' parameter"))?
                .to_string();

            let event_type = args
                .get("event_type")
                .and_then(|v| v.as_str())
                .unwrap_or("fact_added")
                .parse::<MemoryEventType>()?;

            let source = match args.get("source").and_then(|v| v.as_str()) {
                Some("explicit_user") => MemorySource::ExplicitUser,
                Some("tool_verified") => MemorySource::ToolVerified,
                Some("inferred") => MemorySource::Inferred,
                _ => MemorySource::System,
            };

            let layer = Self::parse_layer(&args)?;

            let privacy_level = match args.get("privacy_level").and_then(|v| v.as_str()) {
                Some("public") => PrivacyLevel::Public,
                Some("secret") => PrivacyLevel::Secret,
                _ => PrivacyLevel::Private,
            };

            let confidence = args
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
                .map(|value| value.clamp(0.0, 1.0));

            let importance = args
                .get("importance")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);

            let provenance = Self::parse_provenance(&args)?;

            let mut input = MemoryEventInput::new(
                entity_id,
                &slot_key,
                event_type,
                value,
                source,
                privacy_level,
            )
            .with_layer(layer)
            .with_importance(importance);

            if let Some(confidence) = confidence {
                input = input.with_confidence(confidence);
            }

            let source_ref = args
                .get("source_ref")
                .and_then(|v| v.as_str())
                .map_or_else(|| "tool.memory_store".to_string(), ToString::to_string);

            input = input
                .with_source_kind(SourceKind::Manual)
                .with_source_ref(source_ref);

            if let Some(provenance) = provenance {
                input = input.with_provenance(provenance);
            } else {
                input = input.with_provenance(MemoryProvenance::source_reference(
                    source,
                    "tool.memory_store",
                ));
            }

            enforce_tool_memory_write_policy(&input)?;

            match self.memory.append_event(input).await {
                Ok(event) => {
                    if let Some(log) = &ctx.memory_access_log {
                        log.record_access(
                            entity_id,
                            crate::core::memory::governance::MemoryAccessType::Write,
                            Some(&slot_key),
                        );
                    }
                    Ok(ToolResult {
                        success: true,
                        output: format!("Stored memory event: {}", event.event_id),
                        error: None,

                        attachments: Vec::new(),
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    })
                }
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to store memory: {e}")),

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
    use crate::core::memory::MarkdownMemory;
    use crate::core::tools::middleware::ExecutionContext;
    use crate::security::policy::TenantPolicyContext;

    fn test_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = MarkdownMemory::new(tmp.path());
        (tmp, Arc::new(mem))
    }

    #[test]
    fn name_and_schema() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem);
        assert_eq!(tool.name(), "memory_store");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["entity_id"].is_object());
        assert!(schema["properties"]["slot_key"].is_object());
        assert!(schema["properties"]["value"].is_object());
    }

    #[tokio::test]
    async fn store_core() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem.clone());
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({"entity_id": "lang", "slot_key": "note", "value": "Prefers Rust"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Stored memory event"));

        let slot = mem.resolve_slot("lang", "note").await.unwrap();
        assert!(slot.is_some());
        assert_eq!(slot.unwrap().value, "Prefers Rust");
    }

    #[tokio::test]
    async fn store_with_category() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem.clone());
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({"entity_id": "note", "slot_key": "daily", "value": "Fixed bug"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn store_missing_entity_id_is_rejected() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(json!({"slot_key": "x", "value": "no key"}), &ctx)
            .await;
        assert!(result.is_err());
        assert_eq!(
            result
                .expect_err("missing entity id should fail")
                .to_string(),
            "Missing 'entity_id' parameter"
        );
    }

    #[tokio::test]
    async fn store_missing_content() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(json!({"entity_id": "no_content", "slot_key": "x"}), &ctx)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn store_rejects_secret_privacy_by_policy() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "entity_id": "policy",
                    "slot_key": "note",
                    "value": "x",
                    "privacy_level": "secret"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn store_rejects_foreign_tenant_entity() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem.clone());
        let mut ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");

        let result = tool
            .execute(
                json!({
                    "entity_id": "tenant-beta:person:user-1",
                    "slot_key": "note",
                    "value": "foreign write"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(
            mem.resolve_slot("tenant-beta:person:user-1", "note")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn store_rejects_default_entity_in_tenant_mode() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem.clone());
        let mut ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");

        let result = tool
            .execute(
                json!({"entity_id": "default", "slot_key": "note", "value": "default write"}),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(mem.resolve_slot("default", "note").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn store_accepts_same_tenant_entity() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryStoreTool::new(mem.clone());
        let mut ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()));
        ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");

        let result = tool
            .execute(
                json!({
                    "entity_id": "tenant-alpha:person:user-1",
                    "slot_key": "note",
                    "value": "same tenant write"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.success);
        let slot = mem
            .resolve_slot("tenant-alpha:person:user-1", "note")
            .await
            .unwrap()
            .expect("same-tenant write should persist");
        assert_eq!(slot.value, "same tenant write");
    }
}
