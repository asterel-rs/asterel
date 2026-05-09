use std::sync::Arc;

use asterel::core::memory::{
    MarkdownMemory, Memory, MemoryEventInput, MemoryEventType, MemorySource, PrivacyLevel,
};
use asterel::core::tools::{ExecutionContext, MemoryGovernanceTool, Tool};
use asterel::security::policy::TenantPolicyContext;
use asterel::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use tempfile::TempDir;

fn fixture() -> (TempDir, Arc<dyn Memory>, Arc<SecurityPolicy>) {
    let temp = TempDir::new().expect("temp dir should be created");
    let memory = MarkdownMemory::new(temp.path());
    let security = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: temp.path().to_path_buf(),
        ..SecurityPolicy::default()
    };
    (temp, Arc::new(memory), Arc::new(security))
}

async fn seed_slot(
    memory: &dyn Memory,
    entity_id: &str,
    slot_key: &str,
    value: &str,
    privacy: PrivacyLevel,
) {
    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                slot_key,
                MemoryEventType::FactAdded,
                value,
                MemorySource::ExplicitUser,
                privacy,
            )
            .with_confidence(0.95)
            .with_importance(0.7),
        )
        .await
        .expect("seed slot should be inserted");
}

#[tokio::test]
async fn memory_governance_delete_denied_is_audited() {
    let (_temp, memory, security) = fixture();
    seed_slot(
        memory.as_ref(),
        "tenant-alpha:user-3",
        "profile.region",
        "eu-west",
        PrivacyLevel::Private,
    )
    .await;

    let tool = MemoryGovernanceTool::new(memory);
    let mut ctx = ExecutionContext::from_security(security);
    ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");
    let denied = tool
        .execute(
            json!({
                "action": "delete",
                "actor": "compliance-bot",
                "entity_id": "tenant-beta:user-3",
                "slot_key": "profile.region",
                "mode": "hard",
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": "tenant-alpha"
                }
            }),
            &ctx,
        )
        .await
        .expect("governance tool should return deny result");

    assert!(!denied.success);
    assert_eq!(
        denied.error,
        Some("blocked by security policy: tenant recall scope mismatch".to_string())
    );

    let payload: serde_json::Value =
        serde_json::from_str(&denied.output).expect("deny output should be json");
    let audit_path = payload["audit_record_path"]
        .as_str()
        .expect("audit path should be present");
    let audit_lines = tokio::fs::read_to_string(audit_path)
        .await
        .expect("audit file should exist");
    let latest_record: serde_json::Value = serde_json::from_str(
        audit_lines
            .lines()
            .last()
            .expect("audit should contain at least one record"),
    )
    .expect("latest audit line should be valid json");
    assert_eq!(latest_record["actor"], "compliance-bot");
    assert_eq!(latest_record["action"], "delete");
    assert_eq!(latest_record["outcome"], "denied");
    assert_eq!(
        latest_record["message"],
        "blocked by security policy: tenant recall scope mismatch"
    );
}
