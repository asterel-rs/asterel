//! Approval decision model and risk summarization helpers.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::contracts::ids::EntityId;
use crate::security::governance::RiskLevel;
use crate::security::scrub::scrub_secrets;

/// Approval request payload presented to a broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Stable identifier for the action intent.
    pub intent_id: String,
    /// Tool name being requested.
    pub tool_name: String,
    /// Sanitized, human-readable argument summary.
    pub args_summary: String,
    /// Computed risk level for this request.
    pub risk_level: RiskLevel,
    /// Entity (user/tenant) requesting the action.
    pub entity_id: EntityId,
    /// Origin channel name.
    pub channel: String,
}

/// Decision returned by an approval broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Request approved for immediate execution.
    Approved,
    /// Request denied with rationale.
    Denied { reason: String },
    /// Request approved and converted into a reusable grant.
    ApprovedWithGrant(PermissionGrant),
}

impl ApprovalDecision {
    /// Maps this approval decision to a canonical governance autonomy verdict.
    #[must_use]
    pub fn autonomy_verdict(&self) -> crate::security::governance::AutonomyVerdict {
        match self {
            Self::Approved | Self::ApprovedWithGrant(_) => {
                crate::security::governance::AutonomyVerdict::Allow
            }
            Self::Denied { .. } => crate::security::governance::AutonomyVerdict::Deny,
        }
    }
}

/// Persistable permission grant produced by approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGrant {
    /// Tool this grant applies to.
    pub tool: String,
    /// Argument/path pattern allowed by this grant.
    pub pattern: String,
    /// Lifetime of the grant.
    pub scope: GrantScope,
}

/// Lifetime scope for a permission grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantScope {
    /// Valid only for the current session.
    Session,
    /// Persisted across sessions.
    Permanent,
}

/// Async approval broker interface.
pub trait ApprovalBroker: Send + Sync {
    /// Request approval for a tool invocation.
    fn request_approval<'a>(
        &'a self,
        request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ApprovalDecision>> + Send + 'a>>;
}

/// Broker that denies all requests with a fixed reason.
pub struct AutoDenyBroker {
    /// Rejection reason to return for all requests.
    pub reason: String,
}

impl ApprovalBroker for AutoDenyBroker {
    fn request_approval<'a>(
        &'a self,
        _request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ApprovalDecision>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ApprovalDecision::Denied {
                reason: self.reason.clone(),
            })
        })
    }
}

/// Format an approval request as human-readable notification text.
#[must_use]
pub fn format_approval_text(request: &ApprovalRequest) -> String {
    format!(
        "Tool approval required\nID: {}\nTool: {}\nArgs: {}\nRisk: {:?}\nEntity: {}\n\
         Reply with: approve {}  /  deny {}",
        request.intent_id,
        request.tool_name,
        request.args_summary,
        request.risk_level,
        request.entity_id,
        request.intent_id,
        request.intent_id,
    )
}

/// Classify a tool name into an approval risk tier.
#[must_use]
pub fn classify_risk(tool_name: &str) -> RiskLevel {
    match tool_name {
        "shell" | "composio" => RiskLevel::High,
        "codespace" => RiskLevel::Medium,
        name if name.starts_with("mcp_") => RiskLevel::High,
        "file_write" | "memory_forget" | "memory_governance" | "delegate" | "subagent_spawn"
        | "subagent_output" | "subagent_cancel" => RiskLevel::Medium,
        _ => RiskLevel::Low,
    }
}

/// Sensitive path prefixes/suffixes that upgrade `file_read` to Medium risk.
const SENSITIVE_PATH_PATTERNS: &[&str] = &[
    "/etc/shadow",
    "/etc/passwd",
    "/etc/sudoers",
    ".env",
    ".ssh/",
    "id_rsa",
    "id_ed25519",
    ".gnupg/",
    "credentials",
    ".netrc",
    ".pgpass",
    "config.toml", // asterel config may contain secrets
];

/// Classify risk with argument awareness.
///
/// For `file_read`, inspects the `path` argument to upgrade risk to Medium
/// when the target is a sensitive file (e.g., `/etc/shadow`, `.env`).
#[must_use]
pub fn classify_risk_args(tool_name: &str, args: &serde_json::Value) -> RiskLevel {
    let base = classify_risk(tool_name);
    if tool_name == "file_read"
        && base == RiskLevel::Low
        && let Some(path) = args.get("path").and_then(serde_json::Value::as_str)
    {
        let lower = path.to_lowercase();
        for pattern in SENSITIVE_PATH_PATTERNS {
            if lower.contains(pattern) {
                return RiskLevel::Medium;
            }
        }
    }
    base
}

/// Summarize tool arguments for approval UI/logging.
///
/// Sensitive patterns are scrubbed before returning.
#[must_use]
pub fn summarize_args(tool_name: &str, args: &serde_json::Value) -> String {
    let raw = match tool_name {
        "shell" => args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(unknown)")
            .to_string(),
        "file_write" => {
            let path = args
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            let len = args
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map_or(0, str::len);
            format!("write {len} bytes to {path}")
        }
        "file_read" => args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?")
            .to_string(),
        _ => serde_json::to_string(args).unwrap_or_else(|_| "(args unavailable)".to_string()),
    };

    scrub_secrets(&raw).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalBroker, ApprovalDecision, ApprovalRequest, AutoDenyBroker, GrantScope,
        PermissionGrant, RiskLevel, classify_risk, classify_risk_args, summarize_args,
    };

    #[test]
    fn classify_risk_shell_is_high() {
        assert_eq!(classify_risk("shell"), RiskLevel::High);
    }

    #[test]
    fn classify_risk_file_read_is_low() {
        assert_eq!(classify_risk("file_read"), RiskLevel::Low);
    }

    #[test]
    fn classify_risk_file_write_is_medium() {
        assert_eq!(classify_risk("file_write"), RiskLevel::Medium);
    }

    #[tokio::test]
    async fn auto_deny_broker_denies_all_requests() {
        let broker = AutoDenyBroker {
            reason: "non-interactive context".to_string(),
        };
        let request = ApprovalRequest {
            intent_id: "intent-1".to_string(),
            tool_name: "shell".to_string(),
            args_summary: "ls".to_string(),
            risk_level: RiskLevel::High,
            entity_id: "entity-1".into(),
            channel: "email".to_string(),
        };

        let decision = broker
            .request_approval(&request)
            .await
            .expect("auto deny broker should not fail");

        assert_eq!(
            decision,
            ApprovalDecision::Denied {
                reason: "non-interactive context".to_string()
            }
        );
    }

    #[test]
    fn permission_grant_round_trip_serde() {
        let grant = PermissionGrant {
            tool: "file_write".to_string(),
            pattern: "notes/*.md".to_string(),
            scope: GrantScope::Session,
        };

        let json = serde_json::to_string(&grant).expect("serialize grant");
        let decoded: PermissionGrant = serde_json::from_str(&json).expect("deserialize grant");

        assert_eq!(grant, decoded);
    }

    #[test]
    fn summarize_args_shell_command() {
        let summary = summarize_args("shell", &serde_json::json!({ "command": "ls" }));
        assert_eq!(summary, "ls");
    }

    #[test]
    fn summarize_args_file_write_details() {
        let summary = summarize_args(
            "file_write",
            &serde_json::json!({ "path": "foo.txt", "content": "hello" }),
        );
        assert_eq!(summary, "write 5 bytes to foo.txt");
    }

    #[test]
    fn approval_decision_approved_equality() {
        assert_eq!(ApprovalDecision::Approved, ApprovalDecision::Approved);
    }

    #[test]
    fn classify_risk_composio_is_high() {
        assert_eq!(classify_risk("composio"), RiskLevel::High);
    }

    #[test]
    fn classify_risk_mcp_tools_are_high() {
        assert_eq!(classify_risk("mcp_filesystem_read"), RiskLevel::High);
        assert_eq!(classify_risk("mcp_github_search"), RiskLevel::High);
    }

    #[test]
    fn classify_risk_with_args_file_read_sensitive_paths() {
        let env_args = serde_json::json!({ "path": "/app/.env" });
        assert_eq!(
            classify_risk_args("file_read", &env_args),
            RiskLevel::Medium
        );

        let shadow_args = serde_json::json!({ "path": "/etc/shadow" });
        assert_eq!(
            classify_risk_args("file_read", &shadow_args),
            RiskLevel::Medium
        );

        let ssh_args = serde_json::json!({ "path": "/home/user/.ssh/id_rsa" });
        assert_eq!(
            classify_risk_args("file_read", &ssh_args),
            RiskLevel::Medium
        );
    }

    #[test]
    fn classify_risk_with_args_file_read_normal_paths() {
        let safe_args = serde_json::json!({ "path": "/tmp/notes.txt" });
        assert_eq!(classify_risk_args("file_read", &safe_args), RiskLevel::Low);
    }

    #[test]
    fn classify_risk_with_args_non_file_read_unchanged() {
        let args = serde_json::json!({ "command": "ls" });
        assert_eq!(classify_risk_args("shell", &args), RiskLevel::High);
    }
}
