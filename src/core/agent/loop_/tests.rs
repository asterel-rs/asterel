//! Unit tests for the agent turn loop.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tempfile::TempDir;
use verify_repair::VERIFY_REPAIR_ESCALATION_SLOT_KEY;

use super::*;
use crate::config::PersonaConfig;
use crate::contracts::provider::ProviderCapabilities;
use crate::core::memory::MarkdownMemory;
use crate::core::persona::continuity_gate::ROLLBACK_DRILL_SLOT_KEY;
use crate::core::persona::embodied_state::EMBODIED_STATE_SLOT_KEY;
use crate::core::persona::metacognition::CALIBRATION_SNAPSHOT_SLOT_KEY;
use crate::core::persona::state_header::StateHeader;
use crate::core::persona::state_persistence::BackendHeaderPersist;
use crate::core::persona::style_profile::StyleProfileState;
use crate::core::providers::ProviderResult;
use crate::core::providers::reliable::ReliableProvider;
use crate::core::subagents::NoopSkillMetadataProvider;
use crate::security::SecurityPolicy;

struct MockProvider {
    calls: Arc<AtomicUsize>,
    responses: Vec<String>,
    fail_on_call: Option<usize>,
}

impl Provider for MockProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let call_number = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_on_call == Some(call_number) {
                return Err(anyhow::anyhow!("mock failure on call {call_number}").into());
            }

            Ok(self
                .responses
                .get(call_number - 1)
                .cloned()
                .unwrap_or_else(|| "{}".to_string()))
        })
    }
}

fn sample_state() -> StateHeader {
    StateHeader {
        identity_principles_hash: "identity-v1-abcd1234".to_string(),
        safety_posture: "strict".to_string(),
        current_objective: "Ship two-call main-session loop".to_string(),
        open_loops: vec!["Wire persona reflect stage".to_string()],
        next_actions: vec!["Add strict payload parsing".to_string()],
        commitments: vec!["Preserve answer path on call-2 failure".to_string()],
        recent_context_summary: "Task 4 integrates answer + reflect/writeback calls.".to_string(),
        last_updated_at: Utc::now().to_rfc3339(),
    }
}

fn shifted_timestamp(base: &str, minutes: i64) -> String {
    chrono::DateTime::parse_from_rfc3339(base).map_or_else(
        |_| Utc::now().to_rfc3339(),
        |dt| (dt.with_timezone(&Utc) + ChronoDuration::minutes(minutes.max(0))).to_rfc3339(),
    )
}

fn build_reflect_payload(previous: &StateHeader) -> String {
    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": shifted_timestamp(&previous.last_updated_at, 5)
        },
        "memory_append": ["persona loop writeback accepted"]
    })
    .to_string()
}

fn build_reflect_payload_with_user_inferences(
    previous: &StateHeader,
    user_inferences: &[(&str, &str)],
) -> String {
    let user_inferences_json: Vec<_> = user_inferences
        .iter()
        .map(|(slot_key, value)| json!({"slot_key": slot_key, "value": value}))
        .collect();

    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": shifted_timestamp(&previous.last_updated_at, 5)
        },
        "memory_append": ["persona loop writeback accepted"],
        "user_inferences": user_inferences_json
    })
    .to_string()
}

fn build_reflect_payload_with_memory_inferences(
    previous: &StateHeader,
    memory_inferences: &[(&str, &str)],
) -> String {
    let memory_inferences_json: Vec<_> = memory_inferences
        .iter()
        .map(|(slot_key, value)| json!({"slot_key": slot_key, "value": value}))
        .collect();

    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": shifted_timestamp(&previous.last_updated_at, 5)
        },
        "memory_append": ["persona loop writeback accepted"],
        "memory_inferences": memory_inferences_json
    })
    .to_string()
}

fn build_reflect_payload_with_over_horizon_self_task(previous: &StateHeader) -> String {
    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": shifted_timestamp(&previous.last_updated_at, 5)
        },
        "memory_append": ["persona loop writeback accepted"],
        "self_tasks": [
            {
                "title": "Long horizon task",
                "instructions": "This should be pruned before guard validation",
                "expires_at": shifted_timestamp(&previous.last_updated_at, 60 * 100)
            }
        ]
    })
    .to_string()
}

fn build_reflect_payload_with_style(
    previous: &StateHeader,
    last_updated_at: &str,
    formality: u8,
    verbosity: u8,
    temperature: f64,
) -> String {
    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": last_updated_at
        },
        "memory_append": ["persona loop writeback accepted"],
        "style_profile": {
            "formality": formality,
            "verbosity": verbosity,
            "temperature": temperature
        }
    })
    .to_string()
}

fn build_reflect_payload_discontinuous(previous: &StateHeader) -> String {
    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Rewrite behavior model for abrupt persona reset",
            "open_loops": ["Drop prior loops and rewrite goals immediately"],
            "next_actions": ["Ignore prior continuity assumptions and restart state"],
            "commitments": ["Discard previous commitments"],
            "recent_context_summary": "Large discontinuity introduced for test coverage.",
            "last_updated_at": shifted_timestamp(&previous.last_updated_at, 5)
        },
        "memory_append": ["discontinuous candidate for continuity gate test"]
    })
    .to_string()
}

fn build_reflect_payload_with_same_timestamp(previous: &StateHeader) -> String {
    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": previous.last_updated_at.clone()
        },
        "memory_append": ["persona loop writeback accepted"]
    })
    .to_string()
}

fn build_reflect_payload_with_long_memory_append(previous: &StateHeader) -> String {
    json!({
        "state_header": {
            "identity_principles_hash": previous.identity_principles_hash,
            "safety_posture": previous.safety_posture,
            "current_objective": "Confirm two provider calls per turn",
            "open_loops": previous.open_loops.clone(),
            "next_actions": ["Run targeted persona loop tests"],
            "commitments": previous.commitments.clone(),
            "recent_context_summary": previous.recent_context_summary.clone(),
            "last_updated_at": shifted_timestamp(&previous.last_updated_at, 5)
        },
        "memory_append": ["x".repeat(400)]
    })
    .to_string()
}

fn test_config(workspace_dir: &std::path::Path) -> Config {
    Config {
        workspace_dir: workspace_dir.to_path_buf(),
        memory: crate::config::MemoryConfig {
            auto_save: false,
            ..crate::config::MemoryConfig::default()
        },
        persona: PersonaConfig {
            enabled_main_session: true,
            ..PersonaConfig::default()
        },
        ..Config::default()
    }
}

fn noop_observer() -> Arc<dyn Observer> {
    Arc::new(NoopObserver)
}

fn main_turn_params<'a>(
    config: &Config,
    answer_provider: &'a dyn Provider,
    reflect_provider: &'a dyn Provider,
    system_prompt: &'a str,
    model_name: &'a str,
    temperature: f64,
) -> MainSessionTurnParams<'a> {
    MainSessionTurnParams {
        answer_provider,
        reflect_provider,
        augmentor_provider: None,
        stream_sink: None,
        interactive_input_tx: None,
        approval_broker: None,
        execution_audit_sink: None,
        person_id: "person-test",
        system_prompt,
        model_name,
        temperature,
        registry: Arc::new(crate::core::tools::ToolRegistry::new(vec![])),
        max_tool_iterations: config.autonomy.max_tool_loop_iterations,
        loop_detection: config.tools.loop_detection.clone(),
        rate_limiter: Arc::new(crate::security::EntityRateLimiter::new(
            config.autonomy.max_actions_per_hour,
            config.autonomy.max_actions_per_entity_per_hour,
        )),
        permission_store: Arc::new(crate::security::PermissionStore::load(
            &config.workspace_dir,
        )),
        subagent_manager: Arc::new(crate::core::subagents::SubagentOrchestrator::new()),
        skill_metadata_provider: Arc::new(NoopSkillMetadataProvider::new()),
    }
}

fn main_turn_params_with_augmentor<'a>(
    config: &Config,
    answer_provider: &'a dyn Provider,
    reflect_provider: &'a dyn Provider,
    augmentor_provider: Arc<dyn Provider>,
    system_prompt: &'a str,
    model_name: &'a str,
    temperature: f64,
) -> MainSessionTurnParams<'a> {
    MainSessionTurnParams {
        answer_provider,
        reflect_provider,
        augmentor_provider: Some(augmentor_provider),
        stream_sink: None,
        interactive_input_tx: None,
        approval_broker: None,
        execution_audit_sink: None,
        person_id: "person-test",
        system_prompt,
        model_name,
        temperature,
        registry: Arc::new(crate::core::tools::ToolRegistry::new(vec![])),
        max_tool_iterations: config.autonomy.max_tool_loop_iterations,
        loop_detection: config.tools.loop_detection.clone(),
        rate_limiter: Arc::new(crate::security::EntityRateLimiter::new(
            config.autonomy.max_actions_per_hour,
            config.autonomy.max_actions_per_entity_per_hour,
        )),
        permission_store: Arc::new(crate::security::PermissionStore::load(
            &config.workspace_dir,
        )),
        subagent_manager: Arc::new(crate::core::subagents::SubagentOrchestrator::new()),
        skill_metadata_provider: Arc::new(NoopSkillMetadataProvider::new()),
    }
}

#[tokio::test]
async fn persona_loop_two_calls_per_turn() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MockProvider {
        calls: calls.clone(),
        responses: vec![
            "answer-call-output".to_string(),
            build_reflect_payload(&initial),
        ],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.4),
        "How do we wire Task 4?",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "answer-call-output");
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let updated = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        updated.current_objective,
        "Confirm two provider calls per turn"
    );
}

#[tokio::test]
async fn llm_user_model_uses_auxiliary_augmentor_provider_when_enabled() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_llm_user_model = true;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_temperatures = Arc::new(Mutex::new(Vec::new()));
    let answer_messages = Arc::new(Mutex::new(Vec::new()));
    let answer_provider = TemperatureCaptureProvider {
        temperatures: answer_temperatures,
        messages: answer_messages.clone(),
        response: "answer-with-user-model".to_string(),
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload(&initial)],
        fail_on_call: None,
    };
    let augmentor_calls = Arc::new(AtomicUsize::new(0));
    let augmentor_provider: Arc<dyn Provider> = Arc::new(MockProvider {
        calls: augmentor_calls.clone(),
        responses: vec![
            r#"{"beliefs_about_agent":"The agent can inspect local code","likely_next_question":"Can you fix it now?"}"#
                .to_string(),
        ],
        fail_on_call: None,
    });
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params_with_augmentor(
            &config,
            &answer_provider,
            &reflect_provider,
            augmentor_provider,
            "system",
            "test-model",
            0.4,
        ),
        "Please review this patch.",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "answer-with-user-model");
    assert_eq!(augmentor_calls.load(Ordering::SeqCst), 1);

    let seen_messages = answer_messages
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen_messages.len(), 1);
    assert!(seen_messages[0].contains("User believes: The agent can inspect local code"));
    assert!(seen_messages[0].contains("Likely follow-up: Can you fix it now?"));
}

#[tokio::test]
async fn persona_loop_call2_failure_preserves_answer() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MockProvider {
        calls: calls.clone(),
        responses: vec!["answer-survives-call2-failure".to_string()],
        fail_on_call: Some(2),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.4),
        "Keep answer path stable",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "answer-survives-call2-failure");
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(persisted, initial);
}

struct AlwaysFailProvider {
    calls: Arc<AtomicUsize>,
}

impl Provider for AlwaysFailProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!("transient reflect failure").into())
        })
    }
}

struct TemperatureCaptureProvider {
    temperatures: Arc<Mutex<Vec<f64>>>,
    messages: Arc<Mutex<Vec<String>>>,
    response: String,
}

struct StreamingResponseProvider {
    response: String,
}

struct MessageFailProvider {
    calls: Arc<AtomicUsize>,
    message: &'static str,
}

impl Provider for TemperatureCaptureProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        message: &'a str,
        _model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        let captured_message = message.to_string();
        Box::pin(async move {
            self.temperatures
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(temperature);
            self.messages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(captured_message);
            Ok(self.response.clone())
        })
    }
}

impl Provider for StreamingResponseProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok(self.response.clone()) })
    }

    fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            ..ProviderCapabilities::default()
        }
    }
}

impl Provider for MessageFailProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!(self.message).into())
        })
    }
}

#[tokio::test]
async fn persona_reflect_no_retry() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_calls = Arc::new(AtomicUsize::new(0));
    let answer_provider = ReliableProvider::new(
        vec![(
            "primary".to_string(),
            Box::new(MockProvider {
                calls: answer_calls.clone(),
                responses: vec![
                    "unused-first-attempt".to_string(),
                    "answer-with-reliable-configured".to_string(),
                ],
                fail_on_call: Some(1),
            }),
        )],
        3,
        1,
    );

    let reflect_calls = Arc::new(AtomicUsize::new(0));
    let reflect_provider = AlwaysFailProvider {
        calls: reflect_calls.clone(),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.2,
        ),
        "verify reflect retry suppression",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "answer-with-reliable-configured");
    assert_eq!(answer_calls.load(Ordering::SeqCst), 2);
    assert_eq!(reflect_calls.load(Ordering::SeqCst), 1);

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(persisted, initial);
}

#[tokio::test]
async fn persona_reflect_retries_and_persists_on_transient_failure() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["answer-with-transient-reflect-recovery".to_string()],
        fail_on_call: None,
    };

    let reflect_calls = Arc::new(AtomicUsize::new(0));
    let reflect_provider = ReliableProvider::new(
        vec![(
            "primary".to_string(),
            Box::new(MockProvider {
                calls: reflect_calls.clone(),
                responses: vec![
                    "unused-first-attempt".to_string(),
                    build_reflect_payload(&initial),
                ],
                fail_on_call: Some(1),
            }),
        )],
        2,
        1,
    );
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.2,
        ),
        "recover reflect writeback after one transient provider failure",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "answer-with-transient-reflect-recovery");
    assert_eq!(reflect_calls.load(Ordering::SeqCst), 2);

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        persisted.current_objective,
        "Confirm two provider calls per turn"
    );
}

#[tokio::test]
async fn persona_budget_counter_stable() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_calls = Arc::new(AtomicUsize::new(0));
    let answer_provider = MockProvider {
        calls: answer_calls.clone(),
        responses: vec![
            "turn-1-answer".to_string(),
            "turn-2-answer".to_string(),
            "turn-3-answer".to_string(),
        ],
        fail_on_call: None,
    };

    let reflect_calls = Arc::new(AtomicUsize::new(0));
    let reflect_provider = ReliableProvider::new(
        vec![(
            "primary".to_string(),
            Box::new(MockProvider {
                calls: reflect_calls.clone(),
                responses: vec![
                    build_reflect_payload(&initial),
                    build_reflect_payload(&initial),
                    build_reflect_payload(&initial),
                ],
                fail_on_call: None,
            }),
        )],
        3,
        1,
    );
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    for turn in 0..3 {
        let observer = noop_observer();
        let turn_params = main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.2,
        );
        let ctx = TurnPipelineContext {
            config: &config,
            security: &security,
            mem: mem.clone(),
            params: &turn_params,
            observer: &observer,
        };
        let outcome = execute_main_session_turn_with_accounting(
            &ctx,
            &format!("turn-{turn}-message"),
            &RuntimeMemoryWriteContext::main_session_person("person-test"),
            TurnExecutionSettings {
                conversation_history: &[],
                thinking_level: crate::core::providers::ThinkingLevel::Off,
                show_reasoning: false,
                ephemeral: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            outcome.accounting.budget_limit,
            PERSONA_PER_TURN_CALL_BUDGET
        );
        assert_eq!(outcome.accounting.answer_calls, 1);
        assert_eq!(outcome.accounting.reflect_calls, 1);
        assert_eq!(outcome.accounting.total_calls(), 2);
    }

    assert_eq!(answer_calls.load(Ordering::SeqCst), 3);
    assert_eq!(reflect_calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn persona_loop_policy_blocks_when_action_limit_is_exhausted() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.autonomy.max_actions_per_hour = 0;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MockProvider {
        calls: calls.clone(),
        responses: vec!["should-not-run".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let err = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "blocked by policy",
        &noop_observer(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("action limit exceeded"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn autonomy_temperature_clamped() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;
    config.autonomy.level = crate::security::AutonomyLevel::Full;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let temperatures = Arc::new(Mutex::new(Vec::new()));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let provider = TemperatureCaptureProvider {
        temperatures: temperatures.clone(),
        messages,
        response: "clamped-temp-response".to_string(),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 1.9),
        "clamp this temperature",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "clamped-temp-response");
    let seen = temperatures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen, vec![1.0]);
}

#[tokio::test]
async fn main_session_turn_applies_response_finalization_before_display() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["いい質問です。原因は接続順です。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "原因は接続順です。");
}

#[tokio::test]
async fn main_session_turn_trims_outline_scaffold_leadin() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["以下に簡潔に説明します。原因は接続順です。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "原因は接続順です。");
}

#[tokio::test]
async fn main_session_turn_trims_templated_wrap_up() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["原因は接続順です。以上です。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "原因は接続順です。");
}

#[tokio::test]
async fn main_session_turn_skips_response_finalization_when_disabled() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;
    config.persona.enable_response_finalization = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["いい質問です。原因は接続順です。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "いい質問です。原因は接続順です。");
}

#[tokio::test]
async fn main_session_turn_collapses_unneeded_bullets_for_explanations() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["- 原因は接続順です。\n- 依存は壊れていません。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "原因は接続順です。依存は壊れていません。");
}

#[tokio::test]
async fn main_session_turn_trims_outline_scaffolding_for_explanations() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["結論から言うと、原因は接続順です。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "原因は接続順です。");
}

#[tokio::test]
async fn main_session_report_turn_keeps_bullets_unchanged() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["- テストは通りました。\n- 変更はありません。".to_string()],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "結果を教えて",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "- テストは通りました。\n- 変更はありません。");
}

#[tokio::test]
async fn main_session_streaming_turn_keeps_original_text() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = StreamingResponseProvider {
        response: "いい質問です。原因は接続順です。".to_string(),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.1),
        "説明して",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "いい質問です。原因は接続順です。");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn persona_style_profile_adapts_temperature_and_injects_guidance() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let temperatures = Arc::new(Mutex::new(Vec::new()));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let answer_provider = TemperatureCaptureProvider {
        temperatures: temperatures.clone(),
        messages: messages.clone(),
        response: "style-aware-response".to_string(),
    };
    let reflect_at_1 = shifted_timestamp(&initial.last_updated_at, 5);
    let reflect_at_2 = shifted_timestamp(&initial.last_updated_at, 10);
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![
            build_reflect_payload_with_style(&initial, &reflect_at_1, 20, 20, 0.3),
            build_reflect_payload_with_style(&initial, &reflect_at_2, 95, 95, 0.9),
            build_reflect_payload(&initial),
        ],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    for turn in 0..3 {
        let response = run_main_turn(
            &config,
            &security,
            mem.clone(),
            &main_turn_params(
                &config,
                &answer_provider,
                &reflect_provider,
                "system",
                "test-model",
                0.4,
            ),
            &format!("style adaptation turn {turn}"),
            &noop_observer(),
        )
        .await
        .unwrap();
        assert_eq!(response, "style-aware-response");
    }

    let seen_temperatures = temperatures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen_temperatures.len(), 3);
    assert!(
        seen_temperatures[0] <= 0.4,
        "first turn should not exceed requested baseline temperature"
    );
    assert!(
        seen_temperatures[1] <= 0.3,
        "second turn should reflect adapted style profile target (plus optional cooling deltas)"
    );
    assert!(
        seen_temperatures[2] <= 0.45,
        "third turn should stay within bounded style profile cap"
    );
    assert!(
        seen_temperatures[2] >= seen_temperatures[1],
        "third turn should recover temperature relative to second turn after clamped update"
    );

    let seen_messages = messages
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen_messages
            .iter()
            .all(|message| message.contains("[Response Baseline]")),
        "every turn should include the shared response baseline block"
    );
    assert!(
        seen_messages
            .iter()
            .all(|message| message.contains("[Decision Core]")),
        "every turn should include the compact decision core block"
    );
    assert!(
        !seen_messages[0].contains("[Style profile guidance]"),
        "first turn should not include style guidance before first reflect update"
    );
    assert!(
        seen_messages[1].contains("[Style profile guidance]"),
        "second turn should include style guidance after first reflect update"
    );
    let runtime_pos = seen_messages[1]
        .find("[Runtime metadata]")
        .expect("second turn should include runtime metadata");
    let baseline_pos = seen_messages[1]
        .find("[Response Baseline]")
        .expect("second turn should include shared response baseline");
    assert!(
        baseline_pos > runtime_pos,
        "response baseline should sit closer to the live user turn than runtime context"
    );
    assert!(
        seen_messages[2].contains("temperature=0.45"),
        "third turn should include bounded temperature after clamped update"
    );

    let style_slot = mem
        .resolve_slot("person:person-test", "persona/person-test/style_profile/v1")
        .await
        .unwrap()
        .expect("style profile slot should exist");
    let parsed: StyleProfileState = serde_json::from_str(&style_slot.value).unwrap();
    assert_eq!(parsed.formality, 35);
    assert_eq!(parsed.verbosity, 35);
    assert!((parsed.temperature - 0.45).abs() < f64::EPSILON);
}

#[tokio::test]
async fn conversation_turn_skips_style_profile_guidance_even_with_saved_profile() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let temperatures = Arc::new(Mutex::new(Vec::new()));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let answer_provider = TemperatureCaptureProvider {
        temperatures,
        messages: messages.clone(),
        response: "small-talk-response".to_string(),
    };
    let reflect_at_1 = shifted_timestamp(&initial.last_updated_at, 5);
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![
            build_reflect_payload_with_style(&initial, &reflect_at_1, 20, 20, 0.3),
            build_reflect_payload(&initial),
        ],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let first = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.4,
        ),
        "seed style profile",
        &noop_observer(),
    )
    .await
    .unwrap();
    assert_eq!(first, "small-talk-response");

    let second = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.4,
        ),
        "今日はちょっと眠い",
        &noop_observer(),
    )
    .await
    .unwrap();
    assert_eq!(second, "small-talk-response");

    let seen_messages = messages
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen_messages.len(), 2);
    assert!(
        seen_messages[1].contains("mode=conversation"),
        "small talk should still use conversation mode guidance"
    );
    assert!(
        seen_messages[1].contains("[Decision Core]"),
        "small talk should include the compact decision core block"
    );
    assert!(
        !seen_messages[1].contains("[Style profile guidance]"),
        "small talk should not be burdened with saved style profile guidance"
    );
}

#[tokio::test]
async fn calibration_gate_blocks_reflect_writeback_when_error_exceeds_threshold() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.calibration_gate_min_samples = 1;
    config.persona.calibration_gate_mean_error_max = 0.01;
    config.persona.calibration_gate_p95_error_max = 0.01;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_calls = Arc::new(AtomicUsize::new(0));
    let answer_provider = MockProvider {
        calls: answer_calls.clone(),
        responses: vec!["tiny".to_string()],
        fail_on_call: None,
    };
    let reflect_calls = Arc::new(AtomicUsize::new(0));
    let reflect_provider = MockProvider {
        calls: reflect_calls.clone(),
        responses: vec![build_reflect_payload(&initial)],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "please provide a very short answer to trigger low observed success heuristic",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "tiny");
    assert_eq!(answer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(reflect_calls.load(Ordering::SeqCst), 0);

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(persisted, initial);

    let snapshot = mem
        .resolve_slot("person:person-test", CALIBRATION_SNAPSHOT_SLOT_KEY)
        .await
        .unwrap()
        .expect("calibration snapshot should be persisted");
    let snapshot_json: serde_json::Value = serde_json::from_str(&snapshot.value).unwrap();
    assert_eq!(
        snapshot_json
            .get("gate_status")
            .and_then(serde_json::Value::as_str),
        Some("blocked")
    );
}

#[tokio::test]
async fn continuity_gate_blocks_discontinuous_reflect_writeback() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_calibration_gate = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_calls = Arc::new(AtomicUsize::new(0));
    let answer_provider = MockProvider {
        calls: answer_calls.clone(),
        responses: vec!["continuity-gate-answer".to_string()],
        fail_on_call: None,
    };
    let reflect_calls = Arc::new(AtomicUsize::new(0));
    let reflect_provider = MockProvider {
        calls: reflect_calls.clone(),
        responses: vec![build_reflect_payload_discontinuous(&initial)],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "attempt discontinuous writeback",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "continuity-gate-answer");
    assert_eq!(answer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(reflect_calls.load(Ordering::SeqCst), 1);

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(persisted, initial);

    let drill = mem
        .resolve_slot("person:person-test", ROLLBACK_DRILL_SLOT_KEY)
        .await
        .unwrap()
        .expect("rollback drill slot should exist");
    let drill_json: serde_json::Value = serde_json::from_str(&drill.value).unwrap();
    assert_eq!(
        drill_json.get("status").and_then(serde_json::Value::as_str),
        Some("skipped_no_record")
    );
    assert_eq!(
        drill_json
            .get("trigger")
            .and_then(serde_json::Value::as_str),
        Some("continuity_gate_blocked")
    );
}

#[tokio::test]
async fn reflect_writeback_user_inferences_respects_slot_filtering() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["reflect-writeback-ok".to_string()],
        fail_on_call: None,
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload_with_user_inferences(
            &initial,
            &[
                ("user.preference.response_style", "concise"),
                ("profile.invalid_prefix", "system-learned"),
            ],
        )],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "derive user inferences",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "reflect-writeback-ok");

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        persisted.current_objective,
        "Confirm two provider calls per turn"
    );

    let valid = mem
        .resolve_slot("user:person-test", "user.preference.response_style")
        .await
        .unwrap()
        .expect("valid user inference should be persisted");
    assert_eq!(valid.value, "concise");

    let invalid = mem
        .resolve_slot("user:person-test", "profile.invalid_prefix")
        .await
        .unwrap();
    assert!(
        invalid.is_none(),
        "invalid user inference should be filtered out"
    );
}

#[tokio::test]
async fn reflect_writeback_memory_inferences_persist() {
    let temp = TempDir::new().unwrap();
    let config = test_config(temp.path());

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["reflect-writeback-ok".to_string()],
        fail_on_call: None,
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload_with_memory_inferences(
            &initial,
            &[("language.current", "ja")],
        )],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "respond in Japanese",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "reflect-writeback-ok");

    let inferred = mem
        .resolve_slot("person:person-test", "inferred.language.current")
        .await
        .unwrap()
        .expect("memory inference should be persisted");
    assert_eq!(inferred.value, "ja");
}

#[tokio::test]
async fn reflect_writeback_prunes_out_of_horizon_self_tasks_without_rejecting_state() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_calibration_gate = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["reflect-writeback-ok".to_string()],
        fail_on_call: None,
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload_with_over_horizon_self_task(&initial)],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "keep the writeback even if the self task deadline is too far out",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "reflect-writeback-ok");

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        persisted.current_objective,
        "Confirm two provider calls per turn"
    );
}

#[tokio::test]
async fn rollback_drill_records_pass_after_successful_reflect_writeback() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_calibration_gate = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["reflect-writeback-ok".to_string()],
        fail_on_call: None,
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload(&initial)],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "complete a normal reflect writeback",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "reflect-writeback-ok");

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        persisted.current_objective,
        "Confirm two provider calls per turn"
    );

    let drill = mem
        .resolve_slot("person:person-test", ROLLBACK_DRILL_SLOT_KEY)
        .await
        .unwrap()
        .expect("rollback drill slot should exist");
    let drill_json: serde_json::Value = serde_json::from_str(&drill.value).unwrap();
    assert_eq!(
        drill_json.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );
    assert_eq!(
        drill_json
            .get("trigger")
            .and_then(serde_json::Value::as_str),
        Some("post_writeback")
    );
}

#[tokio::test]
async fn reflect_writeback_normalizes_same_timestamp_and_keeps_rollback_drill_passing() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_calibration_gate = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["reflect-writeback-ok".to_string()],
        fail_on_call: None,
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload_with_same_timestamp(&initial)],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "repair equal reflect timestamp before persistence",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "reflect-writeback-ok");

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        persisted.current_objective,
        "Confirm two provider calls per turn"
    );
    assert_ne!(persisted.last_updated_at, initial.last_updated_at);

    let drill = mem
        .resolve_slot("person:person-test", ROLLBACK_DRILL_SLOT_KEY)
        .await
        .unwrap()
        .expect("rollback drill slot should exist");
    let drill_json: serde_json::Value = serde_json::from_str(&drill.value).unwrap();
    assert_eq!(
        drill_json.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );
}

#[tokio::test]
async fn reflect_writeback_truncates_overlong_memory_append_instead_of_rejecting() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_calibration_gate = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let answer_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec!["reflect-writeback-ok".to_string()],
        fail_on_call: None,
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![build_reflect_payload_with_long_memory_append(&initial)],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.3,
        ),
        "truncate memory_append before guard validation",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "reflect-writeback-ok");

    let persisted = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(
        persisted.current_objective,
        "Confirm two provider calls per turn"
    );

    let drill = mem
        .resolve_slot("person:person-test", ROLLBACK_DRILL_SLOT_KEY)
        .await
        .unwrap()
        .expect("rollback drill slot should exist");
    let drill_json: serde_json::Value = serde_json::from_str(&drill.value).unwrap();
    assert_eq!(
        drill_json.get("status").and_then(serde_json::Value::as_str),
        Some("passed")
    );
}

#[tokio::test]
async fn embodied_state_modulation_reduces_next_turn_temperature_after_high_error() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enable_embodied_state_policy_modulation = true;
    config.persona.embodied_temperature_delta_max = 0.10;
    config.persona.calibration_gate_min_samples = 1;
    config.persona.calibration_gate_mean_error_max = 0.05;
    config.persona.calibration_gate_p95_error_max = 0.05;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "person-test",
    );
    let initial = sample_state();
    persistence.persist_backend_sync(&initial).await.unwrap();

    let temperatures = Arc::new(Mutex::new(Vec::new()));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let answer_provider = TemperatureCaptureProvider {
        temperatures: temperatures.clone(),
        messages,
        response: "tiny".to_string(),
    };
    let reflect_provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![
            build_reflect_payload(&initial),
            build_reflect_payload(&initial),
        ],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let first = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.4,
        ),
        "turn one",
        &noop_observer(),
    )
    .await
    .unwrap();
    assert_eq!(first, "tiny");

    let second = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(
            &config,
            &answer_provider,
            &reflect_provider,
            "system",
            "test-model",
            0.4,
        ),
        "turn two",
        &noop_observer(),
    )
    .await
    .unwrap();
    assert_eq!(second, "tiny");

    let seen_temperatures = temperatures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen_temperatures.len(), 2);
    assert!(
        seen_temperatures[0] <= 0.4,
        "first turn should not exceed requested baseline temperature"
    );
    assert!(seen_temperatures[1] < seen_temperatures[0]);

    let embodied_slot = mem
        .resolve_slot("person:person-test", EMBODIED_STATE_SLOT_KEY)
        .await
        .unwrap()
        .expect("embodied-state snapshot should be persisted");
    let embodied_json: serde_json::Value = serde_json::from_str(&embodied_slot.value).unwrap();
    let delta = embodied_json
        .get("applied_temperature_delta")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    assert!(delta <= 0.0);
    assert!(
        embodied_json
            .get("modulation_reason")
            .and_then(serde_json::Value::as_str)
            .is_some()
    );
}

#[tokio::test]
async fn verify_repair_recovers_within_attempt_cap() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;
    config.autonomy.verify_repair_max_attempts = 3;
    config.autonomy.verify_repair_max_repair_depth = 2;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MockProvider {
        calls: calls.clone(),
        responses: vec![
            "unused-first-attempt".to_string(),
            "recovered-on-second-attempt".to_string(),
        ],
        fail_on_call: Some(1),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.2),
        "recover after one failure",
        &noop_observer(),
    )
    .await
    .unwrap();

    assert_eq!(response, "recovered-on-second-attempt");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn verify_repair_stops_at_max_attempts() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;
    config.autonomy.verify_repair_max_attempts = 3;
    config.autonomy.verify_repair_max_repair_depth = 2;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MessageFailProvider {
        calls: calls.clone(),
        message: "deterministic transient failure",
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let err = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.2),
        "always fail",
        &noop_observer(),
    )
    .await
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("reason=max_attempts_reached"));
    assert!(message.contains("attempts=3"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn verify_repair_emits_escalation_event() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;
    config.autonomy.verify_repair_max_attempts = 2;
    config.autonomy.verify_repair_max_repair_depth = 1;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MessageFailProvider {
        calls: calls.clone(),
        message: "deterministic retry failure",
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let err = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.2),
        "escalate and emit event",
        &noop_observer(),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("reason=max_attempts_reached"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let escalation = mem
        .resolve_slot("person:person-test", VERIFY_REPAIR_ESCALATION_SLOT_KEY)
        .await
        .unwrap()
        .expect("escalation event should be written");

    assert!(
        escalation
            .value
            .contains("\"reason\":\"max_attempts_reached\"")
    );
    assert!(escalation.value.contains("\"attempts\":2"));
    assert!(
        escalation
            .value
            .contains("\"failure_class\":\"transient_failure\"")
    );
}

#[tokio::test]
async fn verify_repair_retries_still_enforce_policy_limits() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.persona.enabled_main_session = false;
    config.autonomy.max_actions_per_hour = 2;
    config.autonomy.verify_repair_max_attempts = 5;
    config.autonomy.verify_repair_max_repair_depth = 4;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = MessageFailProvider {
        calls: calls.clone(),
        message: "retry until policy blocks",
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let err = run_main_turn(
        &config,
        &security,
        mem,
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.2),
        "policy must gate every retry",
        &noop_observer(),
    )
    .await
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("reason=non_retryable_failure"));
    assert!(message.contains("failure_class=policy_limit"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn post_turn_inference_hook_appends_tagged_events() {
    let temp = TempDir::new().unwrap();
    let mut config = test_config(temp.path());
    config.memory.auto_save = true;
    config.persona.enabled_main_session = false;

    let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let provider = MockProvider {
        calls: Arc::new(AtomicUsize::new(0)),
        responses: vec![
            "INFERRED_CLAIM inference.preference.language => User prefers Rust\nCONTRADICTION_EVENT contradiction.preference.language => Earlier note said Python".to_string(),
        ],
        fail_on_call: None,
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let response = run_main_turn(
        &config,
        &security,
        mem.clone(),
        &main_turn_params(&config, &provider, &provider, "system", "test-model", 0.3),
        "derive inferences",
        &noop_observer(),
    )
    .await
    .unwrap();
    assert!(!response.contains("INFERRED_CLAIM"));
    assert!(!response.contains("CONTRADICTION_EVENT"));

    let inferred = mem
        .resolve_slot("person:person-test", "inference.preference.language")
        .await
        .unwrap()
        .expect("inferred claim should persist");
    assert_eq!(inferred.source, MemorySource::Inferred);

    let contradiction = mem
        .resolve_slot("person:person-test", "contradiction.preference.language")
        .await
        .unwrap()
        .expect("contradiction event should be represented as event");
    assert_eq!(contradiction.source, MemorySource::System);
}
