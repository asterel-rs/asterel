use std::sync::Arc;

use asterel::core::tools::{ActionIntent, ActionOperator, NoopOperator};
use asterel::security::SecurityPolicy;
use tempfile::TempDir;

#[tokio::test]
async fn noop_operator_never_executes_external_action() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy {
        workspace_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });

    let intent = ActionIntent::new("notify", "slack", serde_json::json!({"text": "hello"}));
    let verdict = intent.policy_verdict(&security);
    let operator = NoopOperator::new(security);
    let result = operator.apply(&intent, Some(&verdict)).await.unwrap();

    assert!(!result.executed);
    assert!(
        result
            .message
            .contains("external_action_execution is disabled")
    );
    assert!(
        result
            .audit_record_path
            .as_deref()
            .is_some_and(|path| path.contains("action_intents"))
    );
}

#[tokio::test]
async fn action_intent_requires_policy_verdict() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy {
        workspace_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });

    let intent = ActionIntent::new("notify", "slack", serde_json::json!({"text": "hello"}));
    let operator = NoopOperator::new(security);
    let err = operator.apply(&intent, None).await.unwrap_err();

    assert!(err.to_string().contains("policy verdict required"));
}
