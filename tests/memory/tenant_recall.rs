use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use asterel::config::Config;
use asterel::core::agent::loop_::{
    ContextBudget, TurnParams, build_context_for_integration, run_main_turn_policy_test,
};
use asterel::core::memory::{Memory, MemoryCategory, MemoryReader, RecallQuery};
use asterel::core::providers::{Provider, ProviderResult};
use asterel::core::tools::{ExecutionContext, MemoryRecallTool, Tool};
use asterel::security::SecurityPolicy;
use asterel::security::policy::{
    TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR, TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR,
    TenantPolicyContext,
};
use serde_json::json;

use super::memory_harness::{append_test_event, markdown_fixture};

struct CaptureProvider {
    reply: String,
    last_message: Arc<Mutex<Option<String>>>,
}

impl CaptureProvider {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
            last_message: Arc::new(Mutex::new(None)),
        }
    }

    // Debug helper: inspect the last message seen by CaptureProvider in failing tests.
    #[allow(dead_code)]
    fn captured_message(&self) -> Option<String> {
        self.last_message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Provider for CaptureProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            *self
                .last_message
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.to_string());
            Ok(self.reply.clone())
        })
    }
}

#[tokio::test]
async fn tenant_mode_disables_default_fallback() {
    let (_tmp_dir, memory) = markdown_fixture();

    append_test_event(
        &memory,
        "tenant-alpha:user-002",
        "profile.timezone",
        "Timezone is UTC",
        MemoryCategory::Core,
    )
    .await;

    let err = memory
        .recall_scoped(
            RecallQuery::new("default", "timezone", 5)
                .with_policy_context(TenantPolicyContext::enabled("tenant-alpha")),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("memory policy violation: {TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR}"),
        "tenant mode must reject default-scope fallback"
    );
}

#[tokio::test]
async fn tenant_recall_all_entrypoints_allow_same_tenant() {
    let (_tmp_dir, memory) = markdown_fixture();
    let memory: Arc<dyn Memory> = Arc::new(memory);

    append_test_event(
        memory.as_ref(),
        "tenant-alpha:user-003",
        "profile.language",
        "Primary language is Rust",
        MemoryCategory::Core,
    )
    .await;

    let policy_context = TenantPolicyContext::enabled("tenant-alpha");

    let direct = memory
        .recall_scoped(
            RecallQuery::new("tenant-alpha:user-003", "language", 5)
                .with_policy_context(policy_context.clone()),
        )
        .await
        .unwrap();
    assert_eq!(direct.len(), 1);

    let tool = MemoryRecallTool::new(memory.clone());
    let ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
    let tool_result = tool
        .execute(
            json!({
                "entity_id": "tenant-alpha:user-003",
                "query": "language",
                "limit": 5,
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": "tenant-alpha"
                }
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(tool_result.success);
    assert!(tool_result.output.contains("Primary language is Rust"));

    let loop_context = build_context_for_integration(
        memory.as_ref(),
        "tenant-alpha:user-003",
        "language",
        policy_context,
        ContextBudget::default(),
    )
    .await;
    assert!(loop_context.is_ok());
}

#[tokio::test]
async fn tenant_recall_all_entrypoints_block_cross_scope() {
    let (_tmp_dir, memory) = markdown_fixture();
    let memory: Arc<dyn Memory> = Arc::new(memory);

    append_test_event(
        memory.as_ref(),
        "tenant-beta:user-004",
        "profile.timezone",
        "Timezone is UTC",
        MemoryCategory::Core,
    )
    .await;

    let policy_context = TenantPolicyContext::enabled("tenant-alpha");

    let direct_err = memory
        .recall_scoped(
            RecallQuery::new("tenant-beta:user-004", "timezone", 5)
                .with_policy_context(policy_context.clone()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        direct_err.to_string(),
        format!("memory policy violation: {TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR}")
    );

    let tool = MemoryRecallTool::new(memory.clone());
    let mut ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
    ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");
    let tool_result = tool
        .execute(
            json!({
                "entity_id": "tenant-beta:user-004",
                "query": "timezone",
                "limit": 5,
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": "tenant-alpha"
                }
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!tool_result.success);
    assert_eq!(
        tool_result.error,
        Some(format!(
            "Memory recall failed: {TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR}"
        ))
    );

    let loop_err = build_context_for_integration(
        memory.as_ref(),
        "tenant-beta:user-004",
        "timezone",
        policy_context,
        ContextBudget::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        loop_err.to_string(),
        format!("memory policy violation: {TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR}")
    );
}

#[tokio::test]
async fn tenant_recall_e2e_cross_tenant_block() {
    let (_tmp_dir, memory) = markdown_fixture();
    let memory: Arc<dyn Memory> = Arc::new(memory);

    append_test_event(
        memory.as_ref(),
        "tenant-beta:user-006",
        "profile.timezone",
        "Timezone is UTC",
        MemoryCategory::Core,
    )
    .await;

    let tenant_alpha_context = TenantPolicyContext::enabled("tenant-alpha");

    let tool = MemoryRecallTool::new(memory.clone());
    let mut ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
    ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");
    let cross_tool_result = tool
        .execute(
            json!({
                "entity_id": "tenant-beta:user-006",
                "query": "timezone",
                "limit": 5,
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": "tenant-alpha"
                }
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!cross_tool_result.success);
    assert_eq!(
        cross_tool_result.error,
        Some(format!(
            "Memory recall failed: {TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR}"
        ))
    );

    let default_tool_result = tool
        .execute(
            json!({
                "entity_id": "default",
                "query": "timezone",
                "limit": 5,
                "policy_context": {
                    "tenant_mode_enabled": true,
                    "tenant_id": "tenant-alpha"
                }
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!default_tool_result.success);
    assert_eq!(
        default_tool_result.error,
        Some(format!(
            "Memory recall failed: {TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR}"
        ))
    );

    let loop_cross_err = build_context_for_integration(
        memory.as_ref(),
        "tenant-beta:user-006",
        "timezone",
        tenant_alpha_context.clone(),
        ContextBudget::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        loop_cross_err.to_string(),
        format!("memory policy violation: {TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR}")
    );

    let loop_default_err = build_context_for_integration(
        memory.as_ref(),
        "default",
        "timezone",
        tenant_alpha_context.clone(),
        ContextBudget::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        loop_default_err.to_string(),
        format!("memory policy violation: {TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR}")
    );

    let mut config = Config::default();
    config.memory.auto_save = false;
    config.persona.enabled_main_session = false;
    config.autonomy.verify_repair_max_attempts = 1;
    config.autonomy.verify_repair_max_repair_depth = 0;
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    let provider = CaptureProvider::new("ok");

    let runtime_cross_err = run_main_turn_policy_test(TurnParams {
        config: &config,
        security: &security,
        mem: memory.clone(),
        answer_provider: &provider,
        reflect_provider: &provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.0,
        entity_id: "tenant-beta:user-006",
        policy_context: tenant_alpha_context.clone(),
        user_message: "timezone",
    })
    .await
    .unwrap_err();
    assert_eq!(
        runtime_cross_err.to_string(),
        format!(
            "verify/repair escalated: reason=max_attempts_reached attempts=1 repair_depth=0 max_attempts=1 max_repair_depth=0 failure_class=transient_failure last_error={TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR}"
        )
    );

    let runtime_default_err = run_main_turn_policy_test(TurnParams {
        config: &config,
        security: &security,
        mem: memory,
        answer_provider: &provider,
        reflect_provider: &provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.0,
        entity_id: "default",
        policy_context: tenant_alpha_context,
        user_message: "timezone",
    })
    .await
    .unwrap_err();
    assert_eq!(
        runtime_default_err.to_string(),
        format!(
            "verify/repair escalated: reason=max_attempts_reached attempts=1 repair_depth=0 max_attempts=1 max_repair_depth=0 failure_class=transient_failure last_error={TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR}"
        )
    );
}
