//! MCP-compatible memory tool definitions.
//!
//! Exposes `Asterel` memory as read-only MCP tools for external
//! consumers. Write access is intentionally omitted — external agents
//! can read but not modify the agent's memory.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::core::memory::identifier::normalize_identifier;
use crate::core::memory::{Memory, RecallQuery};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};
use crate::security::scrub::sanitize_api_error;

const DEFAULT_RECALL_LIMIT: usize = 10;
const MAX_RECALL_LIMIT: usize = 20;
const DEFAULT_GRAPH_LIMIT: usize = 10;
const MAX_GRAPH_LIMIT: usize = 50;
const MCP_OUTPUT_TAINT_LABEL: &str = "external:mcp";
const MCP_MEMORY_READ_TAINT_LABEL: &str = "memory:read";

pub struct McpMemoryRecallTool {
    memory: Arc<dyn Memory>,
}

pub struct McpMemoryLookupTool {
    memory: Arc<dyn Memory>,
}

pub struct McpMemoryGraphQueryTool {
    memory: Arc<dyn Memory>,
}

impl McpMemoryRecallTool {
    #[must_use]
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl McpMemoryLookupTool {
    #[must_use]
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl McpMemoryGraphQueryTool {
    #[must_use]
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

fn parse_required_string(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(std::string::ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("Missing '{key}' parameter"))
}

fn parse_bounded_limit(
    args: &Value,
    key: &str,
    default: usize,
    maximum: usize,
) -> anyhow::Result<usize> {
    let Some(raw) = args.get(key) else {
        return Ok(default);
    };

    let value = raw
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Invalid '{key}' parameter: expected integer"))?;
    let parsed = usize::try_from(value).unwrap_or(usize::MAX);
    Ok(parsed.min(maximum))
}

fn scoped_query(
    entity_id: &str,
    query: impl Into<String>,
    limit: usize,
    ctx: &ExecutionContext,
) -> anyhow::Result<RecallQuery> {
    let entity_id = scoped_mcp_memory_entity_id(entity_id, ctx);
    let request =
        RecallQuery::new(&entity_id, query, limit).with_policy_context(ctx.tenant_context.clone());
    request.enforce_policy()?;
    Ok(request)
}

fn scoped_mcp_memory_entity_id(entity_id: &str, ctx: &ExecutionContext) -> String {
    let requested = entity_id.trim();
    if requested == "default" {
        return requested.to_string();
    }
    ctx.tenant_context.scope_entity_id(requested)
}

fn normalize_mcp_slot_key(raw: &str) -> anyhow::Result<String> {
    let normalized = normalize_identifier(raw, true);
    anyhow::ensure!(!normalized.is_empty(), "slot_key must not be empty");
    anyhow::ensure!(normalized.len() <= 256, "slot_key must be <= 256 chars");
    anyhow::ensure!(
        is_valid_mcp_slot_key_pattern(&normalized),
        "slot_key must match taxonomy pattern"
    );
    Ok(normalized)
}

fn is_valid_mcp_slot_key_pattern(slot_key: &str) -> bool {
    let mut chars = slot_key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/'))
}

fn json_success(output: String) -> ToolResult {
    ToolResult {
        success: true,
        output,
        error: None,
        attachments: Vec::new(),
        taint_labels: mcp_memory_taint_labels(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}

fn json_failure(error: &dyn std::fmt::Display) -> ToolResult {
    ToolResult {
        success: false,
        output: String::new(),
        error: Some(sanitize_api_error(&error.to_string())),
        attachments: Vec::new(),
        taint_labels: mcp_memory_taint_labels(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}

fn mcp_memory_taint_labels() -> Vec<String> {
    vec![
        MCP_OUTPUT_TAINT_LABEL.to_string(),
        MCP_MEMORY_READ_TAINT_LABEL.to_string(),
    ]
}

fn slot_to_graph_entry(slot: &crate::core::memory::BeliefSlot) -> Value {
    json!({
        "entity": slot.entity_id,
        "slot": slot.slot_key,
        "value": slot.value,
        "confidence": slot.confidence,
        "importance": slot.importance,
        "score": 0.0,
        "source": slot.source,
        "occurred_at": slot.updated_at,
    })
}

impl Tool for McpMemoryRecallTool {
    fn name(&self) -> &'static str {
        "memory/recall"
    }

    fn description(&self) -> &'static str {
        "Recall read-only memory entries scoped to one entity."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Free-text query used to search memory for the entity"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity id to scope recall against"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of recall entries to return",
                    "default": DEFAULT_RECALL_LIMIT,
                    "maximum": MAX_RECALL_LIMIT
                }
            },
            "required": ["query", "entity_id"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let query = parse_required_string(&args, "query")?;
            let entity_id = parse_required_string(&args, "entity_id")?;
            let limit =
                parse_bounded_limit(&args, "limit", DEFAULT_RECALL_LIMIT, MAX_RECALL_LIMIT)?;
            let request = scoped_query(&entity_id, query, limit, ctx)?;

            let result = match self.memory.recall_scoped(request).await {
                Ok(entries) => serde_json::to_string(&entries).map_or_else(
                    |error| {
                        let error = anyhow::Error::new(error);
                        json_failure(&error)
                    },
                    json_success,
                ),
                Err(error) => json_failure(&error),
            };

            Ok(result)
        })
    }
}

impl Tool for McpMemoryLookupTool {
    fn name(&self) -> &'static str {
        "memory/lookup"
    }

    fn description(&self) -> &'static str {
        "Resolve one read-only belief slot for a scoped entity."
    }

    fn parameters_schema(&self) -> Value {
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
            "required": ["entity_id", "slot_key"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let entity_id = parse_required_string(&args, "entity_id")?;
            let slot_key = normalize_mcp_slot_key(&parse_required_string(&args, "slot_key")?)?;
            let request = scoped_query(&entity_id, &slot_key, 1, ctx)?;

            let result = match self
                .memory
                .resolve_slot(request.entity_id.as_str(), &slot_key)
                .await
            {
                Ok(Some(slot)) => serde_json::to_string(&slot).map_or_else(
                    |error| {
                        let error = anyhow::Error::new(error);
                        json_failure(&error)
                    },
                    json_success,
                ),
                Ok(None) => serde_json::to_string(&json!({
                    "found": false,
                    "slot": Value::Null,
                }))
                .map_or_else(
                    |error| {
                        let error = anyhow::Error::new(error);
                        json_failure(&error)
                    },
                    json_success,
                ),
                Err(error) => json_failure(&error),
            };

            Ok(result)
        })
    }
}

impl Tool for McpMemoryGraphQueryTool {
    fn name(&self) -> &'static str {
        "memory/graph_query"
    }

    fn description(&self) -> &'static str {
        "Return graph-shaped read-only memory metadata for one entity."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Entity id whose graph-shaped memory view should be returned"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of graph-shaped entries to return",
                    "default": DEFAULT_GRAPH_LIMIT,
                    "maximum": MAX_GRAPH_LIMIT
                }
            },
            "required": ["entity_id"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let entity_id = parse_required_string(&args, "entity_id")?;
            let max_results =
                parse_bounded_limit(&args, "max_results", DEFAULT_GRAPH_LIMIT, MAX_GRAPH_LIMIT)?;
            let request = scoped_query(&entity_id, "", max_results, ctx)?;

            let result = match self.memory.list_slots(request.entity_id.as_str()).await {
                Ok(mut slots) => {
                    slots.truncate(request.limit);
                    let graph_entries: Vec<Value> = slots.iter().map(slot_to_graph_entry).collect();
                    serde_json::to_string(&graph_entries).map_or_else(
                        |error| {
                            let error = anyhow::Error::new(error);
                            json_failure(&error)
                        },
                        json_success,
                    )
                }
                Err(error) => json_failure(&error),
            };

            Ok(result)
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
    use crate::security::SecurityPolicy;
    use crate::security::policy::TenantPolicyContext;

    fn test_context() -> ExecutionContext {
        ExecutionContext::test_default(Arc::new(SecurityPolicy::default()))
    }

    fn tenant_context(tenant_id: &str) -> ExecutionContext {
        let mut ctx = test_context();
        ctx.tenant_context = TenantPolicyContext::enabled(tenant_id);
        ctx
    }

    fn seeded_memory() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
        (tmp, memory)
    }

    async fn seed_fact(memory: &Arc<dyn Memory>, entity_id: &str, slot_key: &str, value: &str) {
        memory
            .append_event(
                MemoryEventInput::new(
                    entity_id,
                    slot_key,
                    MemoryEventType::FactAdded,
                    value,
                    MemorySource::ExplicitUser,
                    PrivacyLevel::Private,
                )
                .with_confidence(0.9)
                .with_importance(0.7),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mcp_recall_returns_results() {
        let (_tmp, memory) = seeded_memory();
        seed_fact(&memory, "default", "favorite.language", "Rust").await;
        seed_fact(&memory, "default", "timezone", "UTC").await;

        let tool = McpMemoryRecallTool::new(memory);
        let result = tool
            .execute(
                json!({"entity_id": "default", "query": "Rust", "limit": 5}),
                &test_context(),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.taint_labels, mcp_memory_taint_labels());
        let entries: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["slot_key"], "favorite.language");
        assert_eq!(entries[0]["value"], "Rust");
    }

    #[tokio::test]
    async fn mcp_recall_limits_results() {
        let (_tmp, memory) = seeded_memory();
        for index in 0..30 {
            seed_fact(
                &memory,
                "default",
                &format!("fact.{index}"),
                &format!("default memory fact {index}"),
            )
            .await;
        }

        let tool = McpMemoryRecallTool::new(memory);
        let result = tool
            .execute(
                json!({"entity_id": "default", "query": "default", "limit": 99}),
                &test_context(),
            )
            .await
            .unwrap();

        assert!(result.success);
        let entries: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(entries.len(), 20);
    }

    #[tokio::test]
    async fn mcp_lookup_finds_existing_slot() {
        let (_tmp, memory) = seeded_memory();
        seed_fact(&memory, "default", "profile.name", "Asteron").await;

        let tool = McpMemoryLookupTool::new(memory);
        let result = tool
            .execute(
                json!({"entity_id": "default", "slot_key": "profile.name"}),
                &test_context(),
            )
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.taint_labels, mcp_memory_taint_labels());
        let slot: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(slot["slot_key"], "profile.name");
        assert_eq!(slot["value"], "Asteron");
    }

    #[tokio::test]
    async fn mcp_lookup_uses_scoped_entity_from_policy_check() {
        let (_tmp, memory) = seeded_memory();
        seed_fact(
            &memory,
            "tenant-a:person:alice",
            "profile.name",
            "Tenant Alice",
        )
        .await;
        seed_fact(&memory, "person:alice", "profile.name", "Unscoped Alice").await;

        let tool = McpMemoryLookupTool::new(memory);
        let result = tool
            .execute(
                json!({"entity_id": "person:alice", "slot_key": "profile.name"}),
                &tenant_context("tenant-a"),
            )
            .await
            .unwrap();

        assert!(result.success);
        let slot: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(slot["entity_id"], "tenant-a:person:alice");
        assert_eq!(slot["value"], "Tenant Alice");
    }

    #[tokio::test]
    async fn mcp_lookup_normalizes_slot_key_before_resolve() {
        let (_tmp, memory) = seeded_memory();
        seed_fact(&memory, "default", "profile name", "normalized").await;

        let tool = McpMemoryLookupTool::new(memory);
        let result = tool
            .execute(
                json!({"entity_id": "default", "slot_key": " profile name "}),
                &test_context(),
            )
            .await
            .unwrap();

        assert!(result.success);
        let slot: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(slot["slot_key"], "profile_name");
        assert_eq!(slot["value"], "normalized");
    }

    #[tokio::test]
    async fn mcp_lookup_returns_not_found_for_missing() {
        let (_tmp, memory) = seeded_memory();
        let tool = McpMemoryLookupTool::new(memory);

        let result = tool
            .execute(
                json!({"entity_id": "default", "slot_key": "missing.slot"}),
                &test_context(),
            )
            .await
            .unwrap();

        assert!(result.success);
        let payload: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(payload["found"], false);
        assert_eq!(payload["slot"], Value::Null);
    }

    #[tokio::test]
    async fn mcp_graph_query_returns_graph_shaped_entries() {
        let (_tmp, memory) = seeded_memory();
        seed_fact(&memory, "graph-owner", "favorite.project", "Asterel").await;

        let tool = McpMemoryGraphQueryTool::new(memory);
        let result = tool
            .execute(json!({"entity_id": "graph-owner"}), &test_context())
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.taint_labels, mcp_memory_taint_labels());
        let entries: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["entity"], "graph-owner");
        assert_eq!(entries[0]["slot"], "favorite.project");
        assert_eq!(entries[0]["value"], "Asterel");
        assert!(entries[0]["confidence"].is_number());
    }

    #[test]
    fn mcp_memory_failure_sanitizes_error_and_carries_taint() {
        let error = anyhow::anyhow!("backend failed with api_key=sk-leaked-mcp-memory-token");
        let result = json_failure(&error);

        assert!(!result.success);
        assert_eq!(result.taint_labels, mcp_memory_taint_labels());
        let message = result.error.as_deref().expect("error message");
        assert!(message.contains("[REDACTED]"));
        assert!(!message.contains("sk-leaked-mcp-memory-token"));
    }
}
