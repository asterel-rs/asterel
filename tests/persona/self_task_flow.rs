use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use asterel::config::{Config, PersonaConfig};
use asterel::core::agent::loop_::{TurnParams, run_main_turn_test};
use asterel::core::memory::{MarkdownMemory, Memory};
use asterel::core::persona::state_header::StateHeader;
use asterel::core::persona::state_persistence::BackendHeaderPersist;
use asterel::core::providers::{Provider, ProviderResult};
use asterel::security::SecurityPolicy;
use asterel::security::policy::TenantPolicyContext;
use tempfile::TempDir;

const FOLLOW_UP_QUEUE_SLOT_KEY: &str = "persona.writeback.follow_up_queue.v1";

struct SequenceProvider {
    responses: Mutex<Vec<Result<String>>>,
}

impl SequenceProvider {
    fn new(responses: Vec<Result<String>>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

impl Provider for SequenceProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let mut responses = self
                .responses
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if responses.is_empty() {
                return Ok("{}".to_string());
            }

            responses.remove(0).map_err(Into::into)
        })
    }
}

fn test_config(workspace_dir: &std::path::Path) -> Config {
    let database_url = crate::test_env::postgres_url()
        .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
    let mut config = Config {
        workspace_dir: workspace_dir.to_path_buf(),
        memory: asterel::config::MemoryConfig {
            backend: asterel::config::MemoryBackend::Markdown,
            auto_save: false,
            ..asterel::config::MemoryConfig::default()
        },
        identity: asterel::config::IdentityConfig {
            person_id: Some("person-test".to_string()),
            ..asterel::config::IdentityConfig::default()
        },
        persona: PersonaConfig {
            enabled_main_session: true,
            ..PersonaConfig::default()
        },
        ..Config::default()
    };
    config.memory.postgres_url = Some(database_url);
    config
}

async fn follow_up_items(mem: &dyn Memory) -> Vec<serde_json::Value> {
    let Some(slot) = mem
        .resolve_slot("person:person-test", FOLLOW_UP_QUEUE_SLOT_KEY)
        .await
        .expect("follow-up slot lookup")
    else {
        return Vec::new();
    };

    serde_json::from_str::<serde_json::Value>(&slot.value)
        .ok()
        .and_then(|value| {
            value
                .get("items")
                .and_then(serde_json::Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
}

fn seeded_state() -> StateHeader {
    StateHeader {
        identity_principles_hash: "identity-v1-abcd1234".to_string(),
        safety_posture: "strict".to_string(),
        current_objective: "Close autonomy loop deterministically".to_string(),
        open_loops: vec!["route reflect output into bounded queue".to_string()],
        next_actions: vec!["run integration suite".to_string()],
        commitments: vec!["do not bypass policy guards".to_string()],
        recent_context_summary: "Task 16 cross-layer setup".to_string(),
        last_updated_at: "2026-02-17T12:00:00Z".to_string(),
    }
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn persona_reflect_self_task_flows_through_follow_up_queue() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let config = test_config(&workspace);

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    persistence
        .persist_backend_sync(&seeded_state())
        .await
        .expect("seed canonical state");

    let answer_provider = SequenceProvider::new(vec![Ok("bounded-autonomy-answer".to_string())]);
    let reflect_provider = SequenceProvider::new(vec![Ok(serde_json::json!({
        "state_header": {
            "identity_principles_hash": "identity-v1-abcd1234",
            "safety_posture": "strict",
            "current_objective": "Execute bounded autonomy flow",
            "open_loops": ["route reflect output into bounded queue"],
            "next_actions": ["verify bounded execution"],
            "commitments": ["do not bypass policy guards"],
            "recent_context_summary": "reflect stage produced deterministic update",
            "last_updated_at": "2026-02-17T13:00:00Z"
        },
        "memory_append": ["reflect writeback accepted"],
        "self_tasks": [
            {
                "title": "policy-governed self task",
                "instructions": "attempt bounded execution only",
                "expires_at": "2026-02-17T14:00:00Z"
            }
        ]
    })
    .to_string())]);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &answer_provider,
        reflect_provider: &reflect_provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.4,
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        user_message: "run full bounded autonomy cycle",
    })
    .await
    .expect("main session turn");
    assert_eq!(response, "bounded-autonomy-answer");

    let queued = follow_up_items(&*mem).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0]["task_title"], "policy-governed self task");
    assert!(
        queued[0]["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("attempt bounded execution only")
    );
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn persona_reflect_self_task_enqueue_rejects_payload_above_pending_cap() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let config = test_config(&workspace);

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    persistence
        .persist_backend_sync(&seeded_state())
        .await
        .expect("seed canonical state");

    let self_tasks = (0..6)
        .map(|idx| {
            serde_json::json!({
                "title": format!("self-task-{idx}"),
                "instructions": "attempt bounded execution only",
                "expires_at": "2026-02-17T14:00:00Z"
            })
        })
        .collect::<Vec<_>>();

    let answer_provider = SequenceProvider::new(vec![Ok("bounded-autonomy-answer".to_string())]);
    let reflect_provider = SequenceProvider::new(vec![Ok(serde_json::json!({
        "state_header": {
            "identity_principles_hash": "identity-v1-abcd1234",
            "safety_posture": "strict",
            "current_objective": "Execute bounded autonomy flow",
            "open_loops": ["route reflect output into bounded queue"],
            "next_actions": ["verify bounded execution"],
            "commitments": ["do not bypass policy guards"],
            "recent_context_summary": "reflect stage produced deterministic update",
            "last_updated_at": "2026-02-17T13:00:00Z"
        },
        "memory_append": ["reflect writeback accepted"],
        "self_tasks": self_tasks
    })
    .to_string())]);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &answer_provider,
        reflect_provider: &reflect_provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.4,
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        user_message: "run full bounded autonomy cycle",
    })
    .await
    .expect("main session turn");
    assert_eq!(response, "bounded-autonomy-answer");

    let queued = follow_up_items(&*mem).await;
    assert!(queued.is_empty());
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn persona_reflect_rejects_top_level_source_identity_injection() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let config = test_config(&workspace);

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    persistence
        .persist_backend_sync(&seeded_state())
        .await
        .expect("seed canonical state");

    let answer_provider = SequenceProvider::new(vec![Ok("bounded-autonomy-answer".to_string())]);
    let reflect_provider = SequenceProvider::new(vec![Ok(serde_json::json!({
        "source_kind": "discord",
        "source_ref": "channel:discord:attack",
        "state_header": {
            "identity_principles_hash": "identity-v1-abcd1234",
            "safety_posture": "strict",
            "current_objective": "Attempt identity overwrite",
            "open_loops": ["route reflect output into bounded queue"],
            "next_actions": ["verify bounded execution"],
            "commitments": ["do not bypass policy guards"],
            "recent_context_summary": "inject top-level source identity",
            "last_updated_at": "2026-02-17T13:00:00Z"
        },
        "memory_append": ["reflect writeback accepted"],
        "self_tasks": [
            {
                "title": "malicious self task",
                "instructions": "attempt bounded execution only",
                "expires_at": "2026-02-17T14:00:00Z"
            }
        ]
    })
    .to_string())]);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &answer_provider,
        reflect_provider: &reflect_provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.4,
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        user_message: "run full bounded autonomy cycle",
    })
    .await
    .expect("main session turn");
    assert_eq!(response, "bounded-autonomy-answer");

    let queued = follow_up_items(&*mem).await;
    assert!(queued.is_empty());
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn persona_reflect_rejects_top_level_source_kind_only_injection() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let config = test_config(&workspace);

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    persistence
        .persist_backend_sync(&seeded_state())
        .await
        .expect("seed canonical state");

    let answer_provider = SequenceProvider::new(vec![Ok("bounded-autonomy-answer".to_string())]);
    let reflect_provider = SequenceProvider::new(vec![Ok(serde_json::json!({
        "source_kind": "slack",
        "state_header": {
            "identity_principles_hash": "identity-v1-abcd1234",
            "safety_posture": "strict",
            "current_objective": "Attempt source kind overwrite",
            "open_loops": ["route reflect output into bounded queue"],
            "next_actions": ["verify bounded execution"],
            "commitments": ["do not bypass policy guards"],
            "recent_context_summary": "inject top-level source kind",
            "last_updated_at": "2026-02-17T13:00:00Z"
        },
        "memory_append": ["reflect writeback accepted"],
        "self_tasks": [
            {
                "title": "malicious source-kind task",
                "instructions": "attempt bounded execution only",
                "expires_at": "2026-02-17T14:00:00Z"
            }
        ]
    })
    .to_string())]);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &answer_provider,
        reflect_provider: &reflect_provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.4,
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        user_message: "run full bounded autonomy cycle",
    })
    .await
    .expect("main session turn");
    assert_eq!(response, "bounded-autonomy-answer");

    let queued = follow_up_items(&*mem).await;
    assert!(queued.is_empty());
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn persona_reflect_rejects_top_level_source_ref_only_injection() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let config = test_config(&workspace);

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    persistence
        .persist_backend_sync(&seeded_state())
        .await
        .expect("seed canonical state");

    let answer_provider = SequenceProvider::new(vec![Ok("bounded-autonomy-answer".to_string())]);
    let reflect_provider = SequenceProvider::new(vec![Ok(serde_json::json!({
        "source_ref": "channel:discord:attack",
        "state_header": {
            "identity_principles_hash": "identity-v1-abcd1234",
            "safety_posture": "strict",
            "current_objective": "Attempt source ref overwrite",
            "open_loops": ["route reflect output into bounded queue"],
            "next_actions": ["verify bounded execution"],
            "commitments": ["do not bypass policy guards"],
            "recent_context_summary": "inject top-level source ref",
            "last_updated_at": "2026-02-17T13:00:00Z"
        },
        "memory_append": ["reflect writeback accepted"],
        "self_tasks": [
            {
                "title": "malicious source-ref task",
                "instructions": "attempt bounded execution only",
                "expires_at": "2026-02-17T14:00:00Z"
            }
        ]
    })
    .to_string())]);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &answer_provider,
        reflect_provider: &reflect_provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.4,
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        user_message: "run full bounded autonomy cycle",
    })
    .await
    .expect("main session turn");
    assert_eq!(response, "bounded-autonomy-answer");

    let queued = follow_up_items(&*mem).await;
    assert!(queued.is_empty());
}

#[tokio::test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
async fn persona_reflect_enqueues_bounded_self_tasks_within_pending_cap() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let config = test_config(&workspace);

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    persistence
        .persist_backend_sync(&seeded_state())
        .await
        .expect("seed canonical state");

    let self_tasks = (0..5)
        .map(|idx| {
            serde_json::json!({
                "title": format!("self-task-{idx}"),
                "instructions": "attempt bounded execution only",
                "expires_at": "2026-02-17T14:00:00Z"
            })
        })
        .collect::<Vec<_>>();

    let answer_provider = SequenceProvider::new(vec![Ok("bounded-autonomy-answer".to_string())]);
    let reflect_provider = SequenceProvider::new(vec![Ok(serde_json::json!({
        "state_header": {
            "identity_principles_hash": "identity-v1-abcd1234",
            "safety_posture": "strict",
            "current_objective": "Execute bounded autonomy flow",
            "open_loops": ["route reflect output into bounded queue"],
            "next_actions": ["verify bounded execution"],
            "commitments": ["do not bypass policy guards"],
            "recent_context_summary": "reflect stage produced deterministic update",
            "last_updated_at": "2026-02-17T13:00:00Z"
        },
        "memory_append": ["reflect writeback accepted"],
        "self_tasks": self_tasks
    })
    .to_string())]);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &answer_provider,
        reflect_provider: &reflect_provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.4,
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        user_message: "run full bounded autonomy cycle",
    })
    .await
    .expect("main session turn");
    assert_eq!(response, "bounded-autonomy-answer");

    let queued = follow_up_items(&*mem).await;
    assert!(!queued.is_empty());
    assert!(queued.len() <= 5);
    assert!(queued.iter().all(|item| {
        item["task_title"]
            .as_str()
            .unwrap_or_default()
            .starts_with("self-task-")
    }));
}
