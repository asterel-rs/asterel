use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use asterel::config::PersonaConfig;
use asterel::core::agent::loop_::{TurnParams, run_main_turn_test};
use asterel::core::memory::{MarkdownMemory, Memory};
use asterel::core::providers::ProviderResult;
use asterel::core::providers::traits::Provider;
use asterel::security::SecurityPolicy;
use tempfile::TempDir;

struct PlannerAwareProvider {
    calls: Arc<AtomicUsize>,
    direct_response: String,
    plan_response: String,
}

impl Provider for PlannerAwareProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        let captured_message = message.to_string();
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if captured_message.contains("Build a DAG plan")
                || captured_message.contains("You are the planning controller")
            {
                Ok(self.plan_response.clone())
            } else {
                Ok(self.direct_response.clone())
            }
        })
    }
}

fn test_config(workspace_dir: &std::path::Path) -> asterel::config::Config {
    asterel::config::Config {
        workspace_dir: workspace_dir.to_path_buf(),
        memory: asterel::config::MemoryConfig {
            auto_save: false,
            ..asterel::config::MemoryConfig::default()
        },
        persona: PersonaConfig {
            enabled_main_session: false,
            ..PersonaConfig::default()
        },
        ..asterel::config::Config::default()
    }
}

fn fenced_json(val: &serde_json::Value) -> String {
    format!("```json\n{val}\n```")
}

#[tokio::test]
async fn plan_shaped_request_stays_on_direct_companion_path() {
    let temp = TempDir::new().expect("temp dir");
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = PlannerAwareProvider {
        calls: calls.clone(),
        direct_response: "direct companion answer".to_string(),
        plan_response: fenced_json(&serde_json::json!({
            "id": "plan-cutover",
            "description": "planner bypass",
            "steps": [
                {
                    "id": "A",
                    "description": "first",
                    "action": {"kind": "checkpoint", "label": "a"},
                    "depends_on": []
                },
                {
                    "id": "B",
                    "description": "second",
                    "action": {"kind": "checkpoint", "label": "b"},
                    "depends_on": ["A"]
                },
                {
                    "id": "C",
                    "description": "third",
                    "action": {"kind": "checkpoint", "label": "done"},
                    "depends_on": ["B"]
                }
            ]
        })),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem,
        answer_provider: &provider,
        reflect_provider: &provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.2,
        entity_id: "person:test",
        policy_context: asterel::security::policy::TenantPolicyContext::disabled(),
        user_message: "1. Gather context\n2. Build a plan\n3. Execute it\n4. Report results",
    })
    .await
    .expect("direct companion response should succeed");

    assert_eq!(response, "direct companion answer");
    assert!(!response.contains("Plan draft created"));
    assert!(!response.contains("Approve: /plan approve"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
