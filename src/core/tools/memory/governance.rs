//! Memory governance tool — data-sovereignty actions with mandatory audit logging.
//!
//! # What it does
//!
//! `memory_governance` provides four privileged actions for compliance workflows:
//!
//! * `inspect` — returns slot metadata (and optionally values) for an entity.
//! * `export` — extracts a scoped subset of slots as structured JSON.
//! * `delete` — requests backend deletion of a named slot via the configured
//!   `ForgetMode`; exact physical deletion semantics are backend-dependent.
//! * `verify_integrity` — runs the backend's integrity checker and reports any
//!   inconsistencies between the event log and the deletion ledger.
//!
//! Every call, whether allowed or denied, appends a JSONL record to
//! `<workspace>/memory_governance/<date>.jsonl` with the actor, action,
//! scope, outcome, and timestamp. This log is append-only and is never read
//! back by this module — it exists solely for external audit consumers.
//!
//! # Security surface
//!
//! The tool enforces tenant scope via `policy_context::effective_tenant_policy_context`
//! before executing any action. Denied requests are written to the audit log
//! before the `ToolResult` is returned, so refusals are recorded even when
//! the backend is never touched. `verify_integrity` is exempt from tenant-scope
//! enforcement because it operates on the backend as a whole, not on any
//! specific entity's data.
//!
//! `include_sensitive: true` must be explicitly set to receive `Private` or
//! `Secret` field values in inspect and export responses; by default only
//! `Public` values are included and sensitive fields are replaced with
//! `{ "value_redacted": true }`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::core::memory::{
    BeliefSlot, ForgetMode, Memory, PrivacyLevel, ensure_forget_mode_supported,
};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

enum GovernanceAction {
    Inspect,
    Export,
    Delete,
    VerifyIntegrity,
}

impl GovernanceAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Export => "export",
            Self::Delete => "delete",
            Self::VerifyIntegrity => "verify_integrity",
        }
    }
}

/// Tool that exposes data-sovereignty governance actions on the memory backend.
///
/// All actions are recorded in the daily audit log before this function
/// returns, regardless of success or denial.
pub struct MemoryGovernanceTool {
    memory: Arc<dyn Memory>,
}

impl MemoryGovernanceTool {
    /// Create a new memory-governance tool backed by the given memory.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    fn parse_action(args: &serde_json::Value) -> anyhow::Result<GovernanceAction> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;
        match action {
            "inspect" => Ok(GovernanceAction::Inspect),
            "export" => Ok(GovernanceAction::Export),
            "delete" => Ok(GovernanceAction::Delete),
            "verify_integrity" => Ok(GovernanceAction::VerifyIntegrity),
            other => {
                anyhow::bail!(
                    "Invalid 'action' parameter: got '{other}', must be one of inspect, export, delete, verify_integrity"
                )
            }
        }
    }

    fn parse_mode(args: &serde_json::Value) -> ForgetMode {
        match args.get("mode").and_then(|v| v.as_str()) {
            Some("hard") => ForgetMode::Hard,
            Some("tombstone") => ForgetMode::Tombstone,
            _ => ForgetMode::Soft,
        }
    }

    fn parse_actor(args: &serde_json::Value) -> anyhow::Result<String> {
        let actor = args
            .get("actor")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'actor' parameter"))?
            .trim();
        if actor.is_empty() {
            anyhow::bail!("Invalid 'actor' parameter: must not be empty");
        }
        Ok(actor.to_string())
    }

    fn parse_entity_id(args: &serde_json::Value) -> anyhow::Result<String> {
        let entity_id = args
            .get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'entity_id' parameter"))?
            .trim();
        if entity_id.is_empty() {
            anyhow::bail!("Invalid 'entity_id' parameter: must not be empty");
        }
        Ok(entity_id.to_string())
    }

    fn parse_entity_id_for_action(
        action: &GovernanceAction,
        args: &serde_json::Value,
    ) -> anyhow::Result<String> {
        if matches!(action, GovernanceAction::VerifyIntegrity) {
            let optional = args
                .get("entity_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            return Ok(optional.unwrap_or_else(|| "_integrity".to_string()));
        }
        Self::parse_entity_id(args)
    }

    fn parse_scope_keys(args: &serde_json::Value) -> anyhow::Result<Vec<String>> {
        let mut keys = Vec::new();

        if let Some(slot_key) = args.get("slot_key") {
            let Some(slot_key) = slot_key.as_str() else {
                anyhow::bail!("Invalid 'slot_key' parameter: expected string");
            };
            let slot_key = slot_key.trim();
            if slot_key.is_empty() {
                anyhow::bail!("Invalid 'slot_key' parameter: must not be empty");
            }
            keys.push(slot_key.to_string());
        }

        if let Some(raw_slot_keys) = args.get("slot_keys") {
            let Some(raw_slot_keys) = raw_slot_keys.as_array() else {
                anyhow::bail!("Invalid 'slot_keys' parameter: expected array of strings");
            };
            for value in raw_slot_keys {
                let Some(slot_key) = value.as_str() else {
                    anyhow::bail!("Invalid 'slot_keys' parameter: expected array of strings");
                };
                let slot_key = slot_key.trim();
                if slot_key.is_empty() {
                    anyhow::bail!("Invalid 'slot_keys' parameter: must not contain empty values");
                }
                keys.push(slot_key.to_string());
            }
        }

        keys.sort_unstable();
        keys.dedup();
        Ok(keys)
    }

    fn parse_include_sensitive(args: &serde_json::Value) -> bool {
        args.get("include_sensitive")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    fn redact_slot(slot: &BeliefSlot, include_sensitive: bool) -> serde_json::Value {
        let can_include_value =
            include_sensitive || matches!(slot.privacy_level, PrivacyLevel::Public);

        if can_include_value {
            json!({
                "slot_key": slot.slot_key,
                "privacy_level": slot.privacy_level,
                "value": slot.value,
                "confidence": slot.confidence,
                "importance": slot.importance,
                "updated_at": slot.updated_at,
            })
        } else {
            json!({
                "slot_key": slot.slot_key,
                "privacy_level": slot.privacy_level,
                "value_redacted": true,
                "confidence": slot.confidence,
                "importance": slot.importance,
                "updated_at": slot.updated_at,
            })
        }
    }

    fn audit_path(workspace_dir: &Path) -> PathBuf {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        workspace_dir
            .join("memory_governance")
            .join(format!("{date}.jsonl"))
    }

    async fn append_audit_record(
        &self,
        actor: &str,
        action: &GovernanceAction,
        entity_id: &str,
        scope_keys: &[String],
        status: (&str, &str),
        workspace_dir: &Path,
    ) -> anyhow::Result<String> {
        let (outcome, message) = status;
        let path = Self::audit_path(workspace_dir);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        let record = json!({
            "timestamp": Utc::now().to_rfc3339(),
            "actor": actor,
            "action": action.as_str(),
            "scope": {
                "entity_id": entity_id,
                "slot_keys": scope_keys,
            },
            "outcome": outcome,
            "message": message,
        });

        file.write_all(record.to_string().as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(path.to_string_lossy().into_owned())
    }

    async fn run_inspect(
        &self,
        entity_id: &str,
        scope_keys: &[String],
        include_sensitive: bool,
    ) -> anyhow::Result<serde_json::Value> {
        let event_count = self.memory.count_events(Some(entity_id)).await?;
        if scope_keys.is_empty() {
            return Ok(json!({
                "entity_id": entity_id,
                "event_count": event_count,
                "inspected_slots": [],
            }));
        }

        let mut inspected_slots = Vec::new();
        for slot_key in scope_keys {
            let slot = self.memory.resolve_slot(entity_id, slot_key).await?;
            if let Some(slot) = slot {
                inspected_slots.push(Self::redact_slot(&slot, include_sensitive));
            } else {
                inspected_slots.push(json!({
                    "slot_key": slot_key,
                    "status": "not_found",
                }));
            }
        }

        Ok(json!({
            "entity_id": entity_id,
            "event_count": event_count,
            "inspected_slots": inspected_slots,
        }))
    }

    async fn run_export(
        &self,
        entity_id: &str,
        scope_keys: &[String],
        include_sensitive: bool,
    ) -> anyhow::Result<serde_json::Value> {
        if scope_keys.is_empty() {
            anyhow::bail!("Missing scope for export: provide 'slot_key' or 'slot_keys'");
        }

        let mut entries = Vec::new();
        let mut missing_slot_keys = Vec::new();
        for slot_key in scope_keys {
            match self.memory.resolve_slot(entity_id, slot_key).await? {
                Some(slot) => entries.push(Self::redact_slot(&slot, include_sensitive)),
                None => missing_slot_keys.push(slot_key.clone()),
            }
        }

        Ok(json!({
            "entity_id": entity_id,
            "scope": {
                "slot_keys": scope_keys,
            },
            "entry_count": entries.len(),
            "entries": entries,
            "missing_slot_keys": missing_slot_keys,
            "sensitive_fields_included": include_sensitive,
        }))
    }

    async fn run_delete(
        &self,
        entity_id: &str,
        scope_keys: &[String],
        mode: ForgetMode,
        reason: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let Some(slot_key) = scope_keys.first() else {
            anyhow::bail!("Missing scope for delete: provide 'slot_key'");
        };
        ensure_forget_mode_supported(self.memory.as_ref(), mode)?;
        let outcome = self
            .memory
            .forget_slot(entity_id, slot_key, mode, reason)
            .await?;
        Ok(serde_json::to_value(outcome)?)
    }

    async fn run_action_payload(
        &self,
        action: &GovernanceAction,
        args: &serde_json::Value,
        entity_id: &str,
        scope_keys: &[String],
        include_sensitive: bool,
    ) -> anyhow::Result<serde_json::Value> {
        match action {
            GovernanceAction::Inspect => {
                self.run_inspect(entity_id, scope_keys, include_sensitive)
                    .await
            }
            GovernanceAction::Export => {
                self.run_export(entity_id, scope_keys, include_sensitive)
                    .await
            }
            GovernanceAction::Delete => {
                let mode = Self::parse_mode(args);
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("governance_request");
                self.run_delete(entity_id, scope_keys, mode, reason).await
            }
            GovernanceAction::VerifyIntegrity => self
                .memory
                .verify_integrity()
                .await
                .map(|report| {
                    json!({
                        "backend": report.backend,
                        "verified": report.is_verified,
                        "checked_memory_events": report.checked_memory_events,
                        "checked_deletion_ledger": report.checked_deletion_ledger,
                        "issues": report.issues,
                    })
                })
                .map_err(Into::into),
        }
    }

    async fn failed_action_result(
        &self,
        actor: &str,
        action: &GovernanceAction,
        entity_id: &str,
        scope_keys: &[String],
        error: anyhow::Error,
        workspace_dir: &Path,
    ) -> anyhow::Result<ToolResult> {
        let error_message = error.to_string();
        let audit_record_path = self
            .append_audit_record(
                actor,
                action,
                entity_id,
                scope_keys,
                ("failed", &error_message),
                workspace_dir,
            )
            .await?;
        Ok(ToolResult {
            success: false,
            output: json!({
                "audit_record_path": audit_record_path,
                "action": action.as_str(),
                "entity_id": entity_id,
                "scope": { "slot_keys": scope_keys },
            })
            .to_string(),
            error: Some(error_message),

            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        })
    }
}

impl Tool for MemoryGovernanceTool {
    fn name(&self) -> &'static str {
        "memory_governance"
    }

    fn description(&self) -> &'static str {
        "Run governance inspect/export/delete/verify_integrity actions on memory with audit logging."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["inspect", "export", "delete", "verify_integrity"],
                    "description": "Governance action type"
                },
                "actor": {
                    "type": "string",
                    "description": "Actor identifier for audit records"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity id scope"
                },
                "slot_key": {
                    "type": "string",
                    "description": "Single slot scope key"
                },
                "slot_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Scoped slot keys for inspect/export"
                },
                "mode": {
                    "type": "string",
                    "enum": ["soft", "hard", "tombstone"],
                    "description": "Delete mode for action=delete"
                },
                "reason": {
                    "type": "string",
                    "description": "Delete reason for action=delete"
                },
                "include_sensitive": {
                    "type": "boolean",
                    "description": "Include private/secret values in inspect/export responses"
                },
                "policy_context": {
                    "type": "object",
                    "description": "Optional tenant policy context to validate governance scope",
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
            "required": ["action", "actor"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let action = Self::parse_action(&args)?;
            let actor = Self::parse_actor(&args)?;
            let entity_id = Self::parse_entity_id_for_action(&action, &args)?;
            let scope_keys = Self::parse_scope_keys(&args)?;
            let include_sensitive = Self::parse_include_sensitive(&args);
            let policy_context =
                super::policy_context::effective_tenant_policy_context(&args, ctx)?;

            if !matches!(action, GovernanceAction::VerifyIntegrity)
                && let Err(error) = policy_context.enforce_recall_scope(&entity_id)
            {
                let audit_record_path = self
                    .append_audit_record(
                        &actor,
                        &action,
                        &entity_id,
                        &scope_keys,
                        ("denied", error),
                        &ctx.workspace_dir,
                    )
                    .await?;
                return Ok(ToolResult {
                    success: false,
                    output: json!({
                        "audit_record_path": audit_record_path,
                        "action": action.as_str(),
                        "entity_id": entity_id,
                        "scope": { "slot_keys": scope_keys },
                    })
                    .to_string(),
                    error: Some(error.to_string()),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            let payload = match self
                .run_action_payload(&action, &args, &entity_id, &scope_keys, include_sensitive)
                .await
            {
                Ok(payload) => payload,
                Err(error) => {
                    return self
                        .failed_action_result(
                            &actor,
                            &action,
                            &entity_id,
                            &scope_keys,
                            error,
                            &ctx.workspace_dir,
                        )
                        .await;
                }
            };

            let audit_record_path = self
                .append_audit_record(
                    &actor,
                    &action,
                    &entity_id,
                    &scope_keys,
                    ("allowed", "governance action completed"),
                    &ctx.workspace_dir,
                )
                .await?;

            let output = json!({
                "action": action.as_str(),
                "entity_id": entity_id,
                "scope": { "slot_keys": scope_keys },
                "result": payload,
                "audit_record_path": audit_record_path,
            });

            Ok(ToolResult {
                success: true,
                output: output.to_string(),
                error: None,

                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parse_action_accepts_supported_values() {
        let inspect = MemoryGovernanceTool::parse_action(&json!({"action": "inspect"})).unwrap();
        assert!(matches!(inspect, GovernanceAction::Inspect));

        let export = MemoryGovernanceTool::parse_action(&json!({"action": "export"})).unwrap();
        assert!(matches!(export, GovernanceAction::Export));

        let delete = MemoryGovernanceTool::parse_action(&json!({"action": "delete"})).unwrap();
        assert!(matches!(delete, GovernanceAction::Delete));

        let verify =
            MemoryGovernanceTool::parse_action(&json!({"action": "verify_integrity"})).unwrap();
        assert!(matches!(verify, GovernanceAction::VerifyIntegrity));
    }

    #[test]
    fn parse_action_rejects_invalid_empty_and_case_mismatches() {
        let invalid = MemoryGovernanceTool::parse_action(&json!({"action": "invalid"}))
            .err()
            .map(|error| error.to_string())
            .unwrap();
        assert_eq!(
            invalid,
            "Invalid 'action' parameter: got 'invalid', must be one of inspect, export, delete, verify_integrity"
        );

        let empty = MemoryGovernanceTool::parse_action(&json!({"action": ""}))
            .err()
            .map(|error| error.to_string())
            .unwrap();
        assert_eq!(
            empty,
            "Invalid 'action' parameter: got '', must be one of inspect, export, delete, verify_integrity"
        );

        let case_mismatch = MemoryGovernanceTool::parse_action(&json!({"action": "Inspect"}))
            .err()
            .map(|error| error.to_string())
            .unwrap();
        assert_eq!(
            case_mismatch,
            "Invalid 'action' parameter: got 'Inspect', must be one of inspect, export, delete, verify_integrity"
        );
    }

    #[test]
    fn parse_mode_maps_known_values_and_defaults_for_unknown() {
        assert_eq!(
            MemoryGovernanceTool::parse_mode(&json!({"mode": "soft"})),
            ForgetMode::Soft
        );
        assert_eq!(
            MemoryGovernanceTool::parse_mode(&json!({"mode": "hard"})),
            ForgetMode::Hard
        );
        assert_eq!(
            MemoryGovernanceTool::parse_mode(&json!({"mode": "tombstone"})),
            ForgetMode::Tombstone
        );
        assert_eq!(
            MemoryGovernanceTool::parse_mode(&json!({"mode": "invalid"})),
            ForgetMode::Soft
        );
    }

    #[test]
    fn parse_actor_validates_presence_and_non_empty_values() {
        let actor = MemoryGovernanceTool::parse_actor(&json!({"actor": "governance-bot"})).unwrap();
        assert_eq!(actor, "governance-bot");

        let empty = MemoryGovernanceTool::parse_actor(&json!({"actor": ""}))
            .unwrap_err()
            .to_string();
        assert_eq!(empty, "Invalid 'actor' parameter: must not be empty");

        let whitespace = MemoryGovernanceTool::parse_actor(&json!({"actor": "   "}))
            .unwrap_err()
            .to_string();
        assert_eq!(whitespace, "Invalid 'actor' parameter: must not be empty");
    }

    #[test]
    fn parse_entity_id_validates_presence_and_non_empty_values() {
        let entity_id =
            MemoryGovernanceTool::parse_entity_id(&json!({"entity_id": "tenant-a:user-1"}))
                .unwrap();
        assert_eq!(entity_id, "tenant-a:user-1");

        let empty = MemoryGovernanceTool::parse_entity_id(&json!({"entity_id": ""}))
            .unwrap_err()
            .to_string();
        assert_eq!(empty, "Invalid 'entity_id' parameter: must not be empty");
    }

    #[tokio::test]
    async fn delete_unsupported_forget_mode_records_failed_audit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        let memory = Arc::new(crate::core::memory::MarkdownMemory::new(&memory_dir));
        let tool = MemoryGovernanceTool::new(memory);
        let ctx =
            ExecutionContext::test_default(Arc::new(crate::security::SecurityPolicy::default()))
                .with_workspace(tmp.path().to_path_buf());

        let result = tool
            .execute(
                json!({
                    "action": "delete",
                    "actor": "operator",
                    "entity_id": "person:test",
                    "slot_key": "memory.slot",
                    "mode": "hard"
                }),
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
            "expected unsupported capability error, got {:?}",
            result.error
        );
        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        let audit_path = output["audit_record_path"].as_str().unwrap();
        let audit = std::fs::read_to_string(audit_path).unwrap();
        assert!(audit.contains("\"outcome\":\"failed\""));
        assert!(audit.contains("does not support forget mode"));
    }
}
