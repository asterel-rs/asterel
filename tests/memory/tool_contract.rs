use std::sync::Arc;

use asterel::core::memory::{
    MarkdownMemory, Memory, MemoryEventInput, MemoryEventType, MemorySource, PrivacyLevel,
};
use asterel::core::tools::{
    ExecutionContext, MemoryForgetTool, MemoryRecallTool, MemoryStoreTool, Tool,
};
use asterel::security::SecurityPolicy;
use asterel::security::policy::TenantPolicyContext;
use serde_json::json;
use tempfile::TempDir;

fn markdown_memory() -> (TempDir, Arc<dyn Memory>) {
    let temp = TempDir::new().expect("temp dir should be created");
    let memory = MarkdownMemory::new(temp.path());
    (temp, Arc::new(memory))
}

#[tokio::test]
async fn memory_tool_schema_contract() {
    let (_temp, memory) = markdown_memory();
    let ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));

    let store = MemoryStoreTool::new(memory.clone());
    let store_result = store
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-100",
                "slot_key": "profile.language",
                "value": "Rust"
            }),
            &ctx,
        )
        .await
        .expect("store payload should execute");
    assert!(store_result.success);

    let recall = MemoryRecallTool::new(memory.clone());
    let recall_result = recall
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-100",
                "query": "Rust"
            }),
            &ctx,
        )
        .await
        .expect("recall payload should execute");
    assert!(recall_result.success);
    assert!(recall_result.output.contains("Rust"));

    memory
        .append_event(
            MemoryEventInput::new(
                "tenant-alpha:user-100",
                "sample.slot",
                MemoryEventType::FactAdded,
                "sample value",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_importance(0.5),
        )
        .await
        .expect("seed event should be inserted");

    let forget = MemoryForgetTool::new(memory);
    let forget_result = forget
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-100",
                "slot_key": "sample.slot"
            }),
            &ctx,
        )
        .await
        .expect("forget payload should execute");
    assert!(forget_result.success);

    let missing_entity_id = store
        .execute(
            json!({"slot_key": "profile.locale", "value": "en-US"}),
            &ctx,
        )
        .await;
    assert!(missing_entity_id.is_err());
    assert_eq!(
        missing_entity_id
            .expect_err("missing entity_id should fail")
            .to_string(),
        "Missing 'entity_id' parameter"
    );

    let missing_slot_key = forget
        .execute(json!({"entity_id": "tenant-alpha:user-100"}), &ctx)
        .await;
    assert!(missing_slot_key.is_err());
    assert_eq!(
        missing_slot_key
            .expect_err("missing slot key should fail")
            .to_string(),
        "Missing 'slot_key' parameter"
    );

    let missing_query = recall
        .execute(json!({"entity_id": "tenant-alpha:user-100"}), &ctx)
        .await;
    assert!(missing_query.is_err());
    assert_eq!(
        missing_query
            .expect_err("missing query should fail")
            .to_string(),
        "Missing 'query' parameter"
    );

    let missing_recall_entity = recall.execute(json!({"query": "Rust"}), &ctx).await;
    assert!(missing_recall_entity.is_err());
    assert_eq!(
        missing_recall_entity
            .expect_err("missing recall entity should fail")
            .to_string(),
        "Missing 'entity_id' parameter"
    );
}

#[tokio::test]
async fn memory_tool_policy_context_validation() {
    let (_temp, memory) = markdown_memory();
    let mut ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));

    let recall = MemoryRecallTool::new(memory.clone());

    // Test 1: Invalid policy_context shape in args is rejected
    let invalid_recall_context = recall
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-200",
                "query": "anything",
                "policy_context": "tenant-alpha"
            }),
            &ctx,
        )
        .await;
    assert!(invalid_recall_context.is_err());
    assert!(
        invalid_recall_context
            .expect_err("invalid policy_context shape should fail")
            .to_string()
            .contains("Invalid 'policy_context' parameter")
    );

    // Test 2: Invalid policy_context.tenant_mode_enabled type is rejected
    let invalid_recall_flag = recall
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-200",
                "query": "anything",
                "policy_context": {
                    "tenant_mode_enabled": "yes",
                    "tenant_id": "tenant-alpha"
                }
            }),
            &ctx,
        )
        .await;
    assert!(invalid_recall_flag.is_err());
    assert!(
        invalid_recall_flag
            .expect_err("invalid policy_context tenant_mode_enabled should fail")
            .to_string()
            .contains("Invalid 'policy_context' parameter")
    );

    // Test 3: Cross-tenant blocking works through ctx.tenant_context
    ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");
    let cross_tenant_blocked = recall
        .execute(
            json!({
                "entity_id": "tenant-beta:user-200",
                "query": "anything"
            }),
            &ctx,
        )
        .await
        .expect("cross-tenant recall should return error result");
    assert!(
        !cross_tenant_blocked.success,
        "cross-tenant recall should be blocked"
    );
    assert!(
        cross_tenant_blocked.error.is_some(),
        "error should be present"
    );

    // Test 3b: policy_context cannot override execution tenant scope
    let mismatched_policy_override = recall
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-200",
                "query": "anything",
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": "tenant-beta"
                }
            }),
            &ctx,
        )
        .await;
    assert!(mismatched_policy_override.is_err());
    assert!(
        mismatched_policy_override
            .expect_err("tenant mismatch override should fail")
            .to_string()
            .contains("tenant_id mismatch")
    );

    let store = MemoryStoreTool::new(memory.clone());
    let invalid_provenance = store
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-200",
                "slot_key": "profile.timezone",
                "value": "UTC",
                "provenance": {
                    "source_class": "invalid",
                    "reference": "ticket:11"
                }
            }),
            &ctx,
        )
        .await;
    assert!(invalid_provenance.is_err());
    assert_eq!(
        invalid_provenance
            .expect_err("invalid provenance source should fail")
            .to_string(),
        "Invalid 'provenance.source_class' parameter: got 'invalid', must be one of explicit_user, tool_verified, system, inferred"
    );

    let empty_provenance_reference = store
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-200",
                "slot_key": "profile.timezone",
                "value": "UTC",
                "provenance": {
                    "source_class": "system",
                    "reference": "   "
                }
            }),
            &ctx,
        )
        .await;
    assert!(empty_provenance_reference.is_err());
    assert_eq!(
        empty_provenance_reference
            .expect_err("empty provenance reference should fail")
            .to_string(),
        "Invalid 'provenance.reference' parameter: must not be empty"
    );

    let forget = MemoryForgetTool::new(memory);
    // Test 4: Invalid policy_context in forget args is rejected
    let invalid_forget_context = forget
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-200",
                "slot_key": "profile.timezone",
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": 123
                }
            }),
            &ctx,
        )
        .await;
    assert!(invalid_forget_context.is_err());
    assert!(
        invalid_forget_context
            .expect_err("invalid policy_context in forget should fail")
            .to_string()
            .contains("Invalid 'policy_context' parameter")
    );
}
