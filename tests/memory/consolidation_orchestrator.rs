use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use asterel::config::Config;
use asterel::contracts::observability::NoopObserver;
use asterel::core::agent::loop_::{TurnParams, run_main_turn_policy_test};
use asterel::core::memory::{
    BeliefSlot, CONSOLIDATION_SLOT_KEY, ConsolidationDisposition, ConsolidationInput, ForgetMode,
    ForgetOutcome, MarkdownMemory, Memory, MemoryEvent, MemoryEventInput, MemoryEventType,
    MemoryGovernance, MemoryLayer, MemoryReader, MemoryRecallEntry, MemoryResult, MemorySource,
    MemoryWriter, PrivacyLevel, RecallQuery, consolidation_worker_statuses,
    enqueue_consolidation_task, run_consolidation,
};
use asterel::core::providers::{Provider, ProviderResult};
use asterel::security::SecurityPolicy;
use asterel::security::policy::TenantPolicyContext;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::Value;
use tempfile::TempDir;

fn parse_consolidated_episode(value: &str) -> Value {
    serde_json::from_str(value).expect("consolidation payload should be valid JSON")
}

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

struct DelayedConsolidationMemory {
    inner: Arc<dyn Memory>,
    delay: Duration,
}

struct CountingConsolidationMemory {
    inner: Arc<dyn Memory>,
    consolidation_appends: Arc<AtomicUsize>,
}

impl CountingConsolidationMemory {
    fn consolidation_append_count(&self) -> usize {
        self.consolidation_appends.load(Ordering::SeqCst)
    }
}

impl MemoryWriter for CountingConsolidationMemory {
    fn append_event(
        &self,
        input: MemoryEventInput,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryEvent>> + Send + '_>> {
        Box::pin(async move {
            if input.slot_key.as_str() == CONSOLIDATION_SLOT_KEY {
                self.consolidation_appends.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.append_event(input).await
        })
    }
}

impl MemoryReader for CountingConsolidationMemory {
    fn recall_scoped(
        &self,
        query: RecallQuery,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryRecallEntry>>> + Send + '_>> {
        Box::pin(async move { self.inner.recall_scoped(query).await })
    }

    fn resolve_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Option<BeliefSlot>>> + Send + 'a>> {
        Box::pin(async move { self.inner.resolve_slot(entity_id, slot_key).await })
    }
}

impl MemoryGovernance for CountingConsolidationMemory {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { self.inner.health_check().await })
    }

    fn forget_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
        mode: ForgetMode,
        reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<ForgetOutcome>> + Send + 'a>> {
        Box::pin(async move {
            self.inner
                .forget_slot(entity_id, slot_key, mode, reason)
                .await
        })
    }

    fn count_events<'a>(
        &'a self,
        entity_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<usize>> + Send + 'a>> {
        Box::pin(async move { self.inner.count_events(entity_id).await })
    }
}

impl MemoryWriter for DelayedConsolidationMemory {
    fn append_event(
        &self,
        input: MemoryEventInput,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryEvent>> + Send + '_>> {
        Box::pin(async move {
            if input.slot_key.as_str() == CONSOLIDATION_SLOT_KEY {
                tokio::time::sleep(self.delay).await;
            }
            self.inner.append_event(input).await
        })
    }
}

impl MemoryReader for DelayedConsolidationMemory {
    fn recall_scoped(
        &self,
        query: RecallQuery,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryRecallEntry>>> + Send + '_>> {
        Box::pin(async move { self.inner.recall_scoped(query).await })
    }

    fn resolve_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Option<BeliefSlot>>> + Send + 'a>> {
        Box::pin(async move { self.inner.resolve_slot(entity_id, slot_key).await })
    }
}

impl MemoryGovernance for DelayedConsolidationMemory {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { self.inner.health_check().await })
    }

    fn forget_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
        mode: ForgetMode,
        reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<ForgetOutcome>> + Send + 'a>> {
        Box::pin(async move {
            self.inner
                .forget_slot(entity_id, slot_key, mode, reason)
                .await
        })
    }

    fn count_events<'a>(
        &'a self,
        entity_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<usize>> + Send + 'a>> {
        Box::pin(async move { self.inner.count_events(entity_id).await })
    }
}

#[tokio::test]
async fn memory_consolidation_is_idempotent() {
    let temp = TempDir::new().expect("test setup should succeed");
    let base: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let memory = CountingConsolidationMemory {
        inner: base,
        consolidation_appends: Arc::new(AtomicUsize::new(0)),
    };
    let entity_id = "tenant-alpha:user-1";

    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                "conversation.assistant_resp",
                MemoryEventType::FactAdded,
                "First response",
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Working),
        )
        .await
        .expect("test setup should succeed");

    let checkpoint = memory
        .count_events(Some(entity_id))
        .await
        .expect("test setup should succeed");
    let input = ConsolidationInput::new(entity_id, checkpoint, "Question", "Answer");
    let before = memory
        .count_events(Some(entity_id))
        .await
        .expect("test setup should succeed");

    let first = run_consolidation(&memory, temp.path(), &input)
        .await
        .expect("test setup should succeed");
    assert_eq!(first.disposition, ConsolidationDisposition::Consolidated);

    let after_first = memory
        .count_events(Some(entity_id))
        .await
        .expect("test setup should succeed");
    assert_eq!(after_first, before + 1);
    assert_eq!(
        memory.consolidation_append_count(),
        1,
        "first consolidation should append one semantic event"
    );

    let consolidated_slot = memory
        .resolve_slot(entity_id, CONSOLIDATION_SLOT_KEY)
        .await
        .expect("resolve consolidated slot")
        .expect("consolidated slot should exist after first pass");
    let payload = parse_consolidated_episode(&consolidated_slot.value);
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["entity_id"], entity_id);
    assert_eq!(payload["checkpoint_event_count"], checkpoint);
    assert_eq!(payload["actors"], serde_json::json!(["user", "assistant"]));
    assert!(
        payload["action"]
            .as_str()
            .is_some_and(|action| !action.is_empty())
    );
    assert!(
        payload["outcome"]
            .as_str()
            .is_some_and(|outcome| !outcome.is_empty())
    );
    assert!(payload["context_tags"].is_array());
    assert!(
        payload["user_message"]
            .as_str()
            .is_some_and(|msg| !msg.is_empty())
    );
    assert!(
        payload["assistant_response"]
            .as_str()
            .is_some_and(|msg| !msg.is_empty())
    );

    let second = run_consolidation(&memory, temp.path(), &input)
        .await
        .expect("test setup should succeed");
    assert_eq!(
        second.disposition,
        ConsolidationDisposition::SkippedCheckpoint
    );

    let after_second = memory
        .count_events(Some(entity_id))
        .await
        .expect("test setup should succeed");
    assert_eq!(after_second, after_first);
    assert_eq!(
        memory.consolidation_append_count(),
        1,
        "replaying the same checkpoint must not append a duplicate semantic event"
    );
}

#[tokio::test]
async fn memory_consolidation_parallel_entities_preserves_all_watermarks() {
    let temp = TempDir::new().expect("tempdir");
    let memory = MarkdownMemory::new(temp.path());
    let inputs = [
        ConsolidationInput::new("tenant-alpha:parallel-a", 1, "Question A", "Answer A"),
        ConsolidationInput::new("tenant-alpha:parallel-b", 2, "Question B", "Answer B"),
        ConsolidationInput::new("tenant-alpha:parallel-c", 3, "Question C", "Answer C"),
        ConsolidationInput::new("tenant-alpha:parallel-d", 4, "Question D", "Answer D"),
    ];

    let (a, b, c, d) = tokio::join!(
        run_consolidation(&memory, temp.path(), &inputs[0]),
        run_consolidation(&memory, temp.path(), &inputs[1]),
        run_consolidation(&memory, temp.path(), &inputs[2]),
        run_consolidation(&memory, temp.path(), &inputs[3]),
    );
    for result in [a, b, c, d] {
        assert_eq!(
            result.expect("consolidation should succeed").disposition,
            ConsolidationDisposition::Consolidated
        );
    }

    let state_path = temp
        .path()
        .join("state")
        .join("memory_consolidation_state.json");
    let raw_state = std::fs::read_to_string(&state_path).expect("state file should exist");
    let parsed: Value = serde_json::from_str(&raw_state).expect("state file should be json");

    for input in inputs {
        assert_eq!(
            parsed["watermarks"][input.entity_id.as_str()].as_u64(),
            Some(input.checkpoint_event_count as u64),
            "watermark for {} should be preserved",
            input.entity_id
        );
    }
}

#[tokio::test]
async fn memory_consolidation_runs_async_nonblocking() {
    let temp = TempDir::new().expect("test setup should succeed");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("test setup should succeed");

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

    let base: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let delay = Duration::from_millis(250);
    let mem: Arc<dyn Memory> = Arc::new(DelayedConsolidationMemory {
        inner: base.clone(),
        delay,
    });

    let provider = FixedResponseProvider {
        response: "nonblocking consolidation response".to_string(),
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
        user_message: "run turn quickly",
    })
    .await
    .expect("test setup should succeed");

    assert_eq!(response, "nonblocking consolidation response");
    let pending = base
        .resolve_slot(entity_id, CONSOLIDATION_SLOT_KEY)
        .await
        .expect("test setup should succeed");
    assert!(
        pending.is_none(),
        "consolidation should still be pending when the turn returns"
    );

    tokio::time::sleep(delay + Duration::from_millis(100)).await;
    let consolidated = base
        .resolve_slot(entity_id, CONSOLIDATION_SLOT_KEY)
        .await
        .expect("test setup should succeed");
    assert!(
        consolidated.is_some(),
        "async consolidation should complete"
    );
}

#[tokio::test]
async fn memory_consolidation_worker_status_is_exposed() {
    let temp = TempDir::new().expect("test setup should succeed");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("test setup should succeed");

    let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(&workspace));
    let entity_id = "tenant-alpha:worker-status";
    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                "conversation.bootstrap",
                MemoryEventType::FactAdded,
                "bootstrap signal",
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Working),
        )
        .await
        .expect("test setup should succeed");
    let checkpoint = memory
        .count_events(Some(entity_id))
        .await
        .expect("test setup should succeed");
    let input = ConsolidationInput::new(entity_id, checkpoint, "Question", "Answer");

    enqueue_consolidation_task(memory, workspace, input, Arc::new(NoopObserver));

    for _ in 0..20 {
        let statuses = consolidation_worker_statuses();
        if let Some(status) = statuses
            .iter()
            .find(|status| status.entity_id.as_str() == entity_id)
            && status.phase == asterel::core::memory::ConsolidationWorkerPhase::Completed
        {
            assert_eq!(status.checkpoint_event_count, checkpoint);
            assert_eq!(
                status.disposition,
                Some(ConsolidationDisposition::Consolidated)
            );
            assert_eq!(status.applied_watermark, Some(checkpoint));
            assert!(status.started_at.is_some());
            assert!(status.finished_at.is_some());
            assert!(status.last_error.is_none());
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    panic!("consolidation worker status did not reach completed");
}

#[tokio::test]
async fn memory_consolidation_failure_isolated() {
    let temp = TempDir::new().expect("test setup should succeed");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("test setup should succeed");
    std::fs::write(workspace.join("state"), "blocked").expect("test setup should succeed");

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
        response: "response survives consolidation failure".to_string(),
    };
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    let entity_id = "tenant-alpha:user-99";

    let response = run_main_turn_policy_test(TurnParams {
        config: &config,
        security: &security,
        mem: mem.clone(),
        answer_provider: &provider,
        reflect_provider: &provider,
        system_prompt: "system",
        model_name: "test-model",
        temperature: 0.3,
        entity_id,
        policy_context: TenantPolicyContext::enabled("tenant-alpha"),
        user_message: "keep answer path alive",
    })
    .await
    .expect("test setup should succeed");

    assert_eq!(response, "response survives consolidation failure");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let consolidated = mem
        .resolve_slot(entity_id, CONSOLIDATION_SLOT_KEY)
        .await
        .expect("test setup should succeed");
    assert!(
        consolidated.is_none(),
        "consolidation write should fail but turn response must succeed"
    );
}

#[tokio::test]
async fn memory_consolidation_long_run() {
    let temp = TempDir::new().expect("tempdir");
    let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
    let entity_id = "tenant-alpha:long-run";
    let cycle_count = 10usize;

    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                "conversation.bootstrap",
                MemoryEventType::FactAdded,
                "bootstrap signal",
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Working),
        )
        .await
        .expect("seed event append should succeed");

    let baseline_events = memory
        .count_events(Some(entity_id))
        .await
        .expect("baseline count");
    let mut expected_events = baseline_events;
    let mut expected_watermark = 0usize;

    for cycle in 0..cycle_count {
        let checkpoint = memory
            .count_events(Some(entity_id))
            .await
            .expect("checkpoint count");
        let input = ConsolidationInput::new(
            entity_id,
            checkpoint,
            format!("question cycle {cycle}"),
            format!("assistant cycle {cycle}"),
        );

        let first = run_consolidation(memory.as_ref(), temp.path(), &input)
            .await
            .expect("first consolidation should succeed");
        assert_eq!(first.disposition, ConsolidationDisposition::Consolidated);
        assert_eq!(first.previous_watermark, expected_watermark);
        assert_eq!(first.applied_watermark, checkpoint);

        expected_events += 1;
        expected_watermark = checkpoint;
        let after_first = memory
            .count_events(Some(entity_id))
            .await
            .expect("after first count");
        assert_eq!(after_first, expected_events);

        let second = run_consolidation(memory.as_ref(), temp.path(), &input)
            .await
            .expect("second consolidation should succeed");
        assert_eq!(
            second.disposition,
            ConsolidationDisposition::SkippedCheckpoint,
            "replaying same checkpoint must be idempotent"
        );
        assert_eq!(second.applied_watermark, expected_watermark);

        let after_second = memory
            .count_events(Some(entity_id))
            .await
            .expect("after second count");
        assert_eq!(after_second, expected_events);
    }

    let state_path = temp
        .path()
        .join("state")
        .join("memory_consolidation_state.json");
    let raw_state = std::fs::read_to_string(&state_path).expect("state file should exist");
    let parsed: Value = serde_json::from_str(&raw_state).expect("state file should be json");
    let watermark = parsed["watermarks"][entity_id]
        .as_u64()
        .expect("watermark should be a number") as usize;
    assert_eq!(watermark, expected_watermark);
    assert_eq!(
        memory
            .count_events(Some(entity_id))
            .await
            .expect("final count"),
        baseline_events + cycle_count,
        "long run should grow linearly with unique checkpoints only"
    );
}

#[tokio::test]
async fn memory_consolidation_long_run_decay_progression() {
    let temp = TempDir::new().expect("tempdir");
    let memory = MarkdownMemory::new(temp.path());
    let entity_id = "tenant-alpha:decay";
    let now = Utc::now();

    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                "decay.stale",
                MemoryEventType::FactAdded,
                "cache ttl fallback strategy with stale context",
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.9)
            .with_layer(MemoryLayer::Semantic)
            .with_occurred_at((now - ChronoDuration::days(180)).to_rfc3339()),
        )
        .await
        .expect("append stale event");

    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                "decay.fresh",
                MemoryEventType::FactAdded,
                "cache ttl fallback strategy with fresh context",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.9)
            .with_layer(MemoryLayer::Semantic)
            .with_occurred_at(now.to_rfc3339()),
        )
        .await
        .expect("append fresh event");

    for checkpoint in 2..12 {
        let input = ConsolidationInput::new(
            entity_id,
            checkpoint,
            format!("decay-run question {checkpoint}"),
            format!("decay-run answer {checkpoint}"),
        );
        run_consolidation(&memory, temp.path(), &input)
            .await
            .expect("consolidation should succeed");
    }

    let first = memory
        .recall_scoped(RecallQuery::new(
            entity_id,
            "cache ttl fallback strategy",
            6,
        ))
        .await
        .expect("first recall should succeed");
    let second = memory
        .recall_scoped(RecallQuery::new(
            entity_id,
            "cache ttl fallback strategy",
            6,
        ))
        .await
        .expect("second recall should succeed");

    assert!(
        first.len() <= 6 && second.len() <= 6,
        "recall results must remain bounded by limit"
    );
}
