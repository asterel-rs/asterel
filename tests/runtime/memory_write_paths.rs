use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use asterel::config::Config;
use asterel::core::agent::loop_::{TurnParams, run_main_turn_policy_test};
use asterel::core::memory::{MarkdownMemory, Memory};
use asterel::core::providers::{Provider, ProviderResult};
use asterel::security::SecurityPolicy;
use asterel::security::policy::TenantPolicyContext;
use asterel::transport::channels::build_system_prompt;
use tempfile::TempDir;

struct FixedResponseProvider {
    response: String,
}

impl Provider for FixedResponseProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok(self.response.clone()) })
    }
}

#[tokio::test]
async fn memory_autosave_includes_layer_provenance() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = Config {
        workspace_dir: workspace.clone(),
        memory: asterel::config::MemoryConfig {
            backend: asterel::config::MemoryBackend::Markdown,
            auto_save: true,
            ..asterel::config::MemoryConfig::default()
        },
        persona: asterel::config::PersonaConfig {
            enabled_main_session: false,
            ..asterel::config::PersonaConfig::default()
        },
        ..Config::default()
    };

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let provider = FixedResponseProvider {
        response: "INFERRED_CLAIM inference.preference.language => User prefers Rust\nCONTRADICTION_EVENT contradiction.preference.language => Earlier note said Python".to_string(),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    let entity_id = "tenant-alpha:user-42";

    let response = run_main_turn_policy_test(TurnParams {
        config: &config,
        security: &security,
        mem,
        answer_provider: &provider,
        reflect_provider: &provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.3,
        entity_id,
        policy_context: TenantPolicyContext::enabled("tenant-alpha"),
        user_message: "capture autosave metadata",
    })
    .await
    .unwrap();
    assert!(!response.contains("INFERRED_CLAIM"));
    assert!(!response.contains("CONTRADICTION_EVENT"));
}

#[test]
fn prompt_no_daily_memory_injection() {
    let ws = TempDir::new().unwrap();
    std::fs::write(ws.path().join("SOUL.md"), "# Soul\nBe helpful.").unwrap();
    std::fs::write(ws.path().join("CHARACTER.md"), "# Voice\nWarm and direct.").unwrap();
    std::fs::write(ws.path().join("USER.md"), "# User\nName: Runtime Test").unwrap();
    std::fs::write(
        ws.path().join("AGENTS.md"),
        "# Agents\nFollow instructions.",
    )
    .unwrap();
    std::fs::write(ws.path().join("TOOLS.md"), "# Tools\nUse tools.").unwrap();
    std::fs::write(ws.path().join("HEARTBEAT.md"), "# Heartbeat\nStable.").unwrap();
    std::fs::write(ws.path().join("MEMORY.md"), "# Memory\nCurated memory.").unwrap();

    let memory_dir = ws.path().join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    std::fs::write(
        memory_dir.join(format!("{today}.md")),
        "# Daily\nSome note.",
    )
    .unwrap();

    let prompt = build_system_prompt(ws.path(), "model", &[], &[], None);
    assert!(!prompt.contains("Daily Notes"));
    assert!(!prompt.contains("Some note"));
}
