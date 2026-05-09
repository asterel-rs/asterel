//! Rule-based session-to-semantic memory consolidation.
//!
//! After each conversational turn, the pipeline checks whether the running
//! event count has crossed a new consolidation threshold. When it has, the
//! current turn's content is distilled into a structured [`ConsolidatedEpisode`]
//! and written to the memory backend under the canonical
//! `CONSOLIDATION_SLOT_KEY`.
//!
//! ## Watermark mechanism
//!
//! [`ConsolidationState`] persists a per-entity watermark (`watermarks` map)
//! to `memory_consolidation_state.json` in the workspace directory. The
//! watermark records the last `checkpoint_event_count` that triggered a
//! consolidation write, ensuring that re-runs over the same session do not
//! re-consolidate already-processed turns.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::contracts::ids::EntityId;
use crate::contracts::observability::{MemorySignal, Observer};
use crate::core::memory::traits::MemoryLayer;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
};
use crate::utils::text::truncate_ellipsis;

const STATE_FILE: &str = "memory_consolidation_state.json";
/// Slot key used for storing consolidated semantic memory entries.
pub const CONSOLIDATION_SLOT_KEY: &str =
    crate::contracts::strings::data_model::SLOT_CONSOLIDATION_SEMANTIC_LATEST;
const CONSOLIDATION_PROVENANCE_REF: &str = "memory.consolidation.session_to_semantic";
const EPISODE_SCHEMA_VERSION: u8 = 1;
const MAX_CONSOLIDATION_LOCKS: usize = 1024;
static CONSOLIDATION_LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConsolidationState {
    watermarks: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConsolidatedEpisode {
    schema_version: u8,
    episode_id: String,
    entity_id: EntityId,
    checkpoint_event_count: usize,
    occurred_at: String,
    actors: Vec<String>,
    action: String,
    outcome: String,
    context_tags: Vec<String>,
    user_message: String,
    assistant_response: String,
}

/// Input parameters for a single consolidation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationInput {
    /// Entity whose memory is being consolidated.
    pub entity_id: EntityId,
    /// Number of events at the current checkpoint.
    pub checkpoint_event_count: usize,
    /// The user message from this turn.
    pub user_message: String,
    /// The assistant response from this turn.
    pub assistant_response: String,
}

impl ConsolidationInput {
    /// Create a new consolidation input from the given parameters.
    #[must_use]
    pub fn new(
        entity_id: impl AsRef<str>,
        checkpoint_event_count: usize,
        user_message: impl Into<String>,
        assistant_response: impl Into<String>,
    ) -> Self {
        Self {
            entity_id: EntityId::new(entity_id.as_ref()),
            checkpoint_event_count,
            user_message: user_message.into(),
            assistant_response: assistant_response.into(),
        }
    }
}

/// Outcome disposition of a consolidation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationDisposition {
    /// Successfully consolidated into a semantic slot.
    Consolidated,
    /// Skipped because the input contained no meaningful signal.
    SkippedNoSignal,
    /// Skipped because the checkpoint was already processed.
    SkippedCheckpoint,
}

/// Result of a consolidation pass, including watermark bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationOutput {
    /// What happened during this consolidation attempt.
    pub disposition: ConsolidationDisposition,
    /// Watermark value before this pass.
    pub previous_watermark: usize,
    /// Watermark value after this pass.
    pub applied_watermark: usize,
}

/// Current lifecycle phase of a background consolidation worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationWorkerPhase {
    /// Task has been enqueued but has not started executing yet.
    Queued,
    /// Task is currently running.
    Running,
    /// Task finished successfully.
    Completed,
    /// Task failed; the answer path was preserved.
    Failed,
}

/// Last known status for a background consolidation worker keyed by entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationWorkerStatus {
    /// Entity whose memory was consolidated.
    pub entity_id: EntityId,
    /// Event count checkpoint associated with the task.
    pub checkpoint_event_count: usize,
    /// Current worker phase.
    pub phase: ConsolidationWorkerPhase,
    /// Successful consolidation disposition, if available.
    pub disposition: Option<ConsolidationDisposition>,
    /// Watermark before the task, if available.
    pub previous_watermark: Option<usize>,
    /// Watermark after the task, if available.
    pub applied_watermark: Option<usize>,
    /// RFC 3339 timestamp for the latest start.
    pub started_at: Option<String>,
    /// RFC 3339 timestamp for the latest terminal state.
    pub finished_at: Option<String>,
    /// Last error message, only set for failed tasks.
    pub last_error: Option<String>,
}

fn state_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("state").join(STATE_FILE)
}

fn worker_status_registry() -> &'static Mutex<BTreeMap<String, ConsolidationWorkerStatus>> {
    static STATUSES: OnceLock<Mutex<BTreeMap<String, ConsolidationWorkerStatus>>> = OnceLock::new();
    STATUSES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn upsert_worker_status(
    entity_id: &EntityId,
    update: impl FnOnce(Option<ConsolidationWorkerStatus>) -> ConsolidationWorkerStatus,
) {
    let registry = worker_status_registry();
    let mut guard = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = guard.remove(entity_id.as_str());
    let next = update(previous);
    guard.insert(entity_id.to_string(), next);
}

fn record_worker_queued(input: &ConsolidationInput) {
    upsert_worker_status(&input.entity_id, |_| ConsolidationWorkerStatus {
        entity_id: input.entity_id.clone(),
        checkpoint_event_count: input.checkpoint_event_count,
        phase: ConsolidationWorkerPhase::Queued,
        disposition: None,
        previous_watermark: None,
        applied_watermark: None,
        started_at: None,
        finished_at: None,
        last_error: None,
    });
}

fn record_worker_running(input: &ConsolidationInput) {
    let started_at = chrono::Utc::now().to_rfc3339();
    upsert_worker_status(&input.entity_id, |previous| ConsolidationWorkerStatus {
        entity_id: input.entity_id.clone(),
        checkpoint_event_count: input.checkpoint_event_count,
        phase: ConsolidationWorkerPhase::Running,
        disposition: None,
        previous_watermark: previous.and_then(|status| status.previous_watermark),
        applied_watermark: None,
        started_at: Some(started_at),
        finished_at: None,
        last_error: None,
    });
}

fn record_worker_completed(input: &ConsolidationInput, output: &ConsolidationOutput) {
    let finished_at = chrono::Utc::now().to_rfc3339();
    upsert_worker_status(&input.entity_id, |previous| ConsolidationWorkerStatus {
        entity_id: input.entity_id.clone(),
        checkpoint_event_count: input.checkpoint_event_count,
        phase: ConsolidationWorkerPhase::Completed,
        disposition: Some(output.disposition),
        previous_watermark: Some(output.previous_watermark),
        applied_watermark: Some(output.applied_watermark),
        started_at: previous.and_then(|status| status.started_at),
        finished_at: Some(finished_at),
        last_error: None,
    });
}

fn record_worker_failed(input: &ConsolidationInput, error: &anyhow::Error) {
    let finished_at = chrono::Utc::now().to_rfc3339();
    upsert_worker_status(&input.entity_id, |previous| ConsolidationWorkerStatus {
        entity_id: input.entity_id.clone(),
        checkpoint_event_count: input.checkpoint_event_count,
        phase: ConsolidationWorkerPhase::Failed,
        disposition: None,
        previous_watermark: previous
            .as_ref()
            .and_then(|status| status.previous_watermark),
        applied_watermark: None,
        started_at: previous.and_then(|status| status.started_at),
        finished_at: Some(finished_at),
        last_error: Some(error.to_string()),
    });
}

/// Return the latest known background consolidation worker statuses.
#[must_use]
pub fn consolidation_worker_statuses() -> Vec<ConsolidationWorkerStatus> {
    let registry = worker_status_registry();
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

fn load_state(workspace_dir: &Path) -> Result<ConsolidationState> {
    let path = state_path(workspace_dir);
    if !path.exists() {
        return Ok(ConsolidationState::default());
    }

    let raw = fs::read_to_string(path)?;
    let state = serde_json::from_str(&raw).context("parse consolidation state")?;
    Ok(state)
}

fn ensure_state_parent(workspace_dir: &Path) -> Result<()> {
    let path = state_path(workspace_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn save_state(workspace_dir: &Path, state: &ConsolidationState) -> Result<()> {
    ensure_state_parent(workspace_dir)?;
    let payload = serde_json::to_vec_pretty(state)?;
    let path = state_path(workspace_dir);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, payload)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn join_whitespace(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for word in raw.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn collect_context_tags(user: &str, assistant: &str) -> Vec<String> {
    let mut tags = BTreeSet::new();
    let merged = format!("{user} {assistant}");
    let lower = merged.to_ascii_lowercase();

    if merged.contains("```") {
        tags.insert("code");
    }
    if lower.contains("error") || lower.contains("exception") || lower.contains("failed") {
        tags.insert("debug");
    }
    if lower.contains("plan") || lower.contains("step") || lower.contains("roadmap") {
        tags.insert("structured_reasoning");
    }
    if lower.contains("test") || lower.contains("assert") {
        tags.insert("testing");
    }
    if lower.contains("remember") || lower.contains("memory") || lower.contains("recall") {
        tags.insert("memory");
    }
    if lower.contains("security") || lower.contains("policy") || lower.contains("tenant") {
        tags.insert("security");
    }
    if lower.contains("why") || merged.contains('?') {
        tags.insert("qa");
    }
    if tags.is_empty() {
        tags.insert("general");
    }

    tags.into_iter()
        .map(std::string::ToString::to_string)
        .collect()
}

fn build_consolidation_value(input: &ConsolidationInput) -> String {
    let user = truncate_ellipsis(&join_whitespace(&input.user_message), 240);
    let assistant = truncate_ellipsis(&join_whitespace(&input.assistant_response), 480);
    let occurred_at = chrono::Utc::now().to_rfc3339();
    let tags = collect_context_tags(&user, &assistant);

    let episode = ConsolidatedEpisode {
        schema_version: EPISODE_SCHEMA_VERSION,
        episode_id: format!(
            "episode:{}:{}",
            input.entity_id, input.checkpoint_event_count
        ),
        entity_id: input.entity_id.clone(),
        checkpoint_event_count: input.checkpoint_event_count,
        occurred_at,
        actors: vec!["user".to_string(), "assistant".to_string()],
        action: truncate_ellipsis(&format!("respond_to_user_request: {user}"), 180),
        outcome: truncate_ellipsis(&assistant, 180),
        context_tags: tags,
        user_message: user,
        assistant_response: assistant,
    };

    serde_json::to_string(&episode).unwrap_or_else(|_| {
        format!(
            "checkpoint={} | user={} | assistant={}",
            input.checkpoint_event_count, episode.user_message, episode.assistant_response
        )
    })
}

fn consolidation_lock(entity_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let locks = CONSOLIDATION_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.len() >= MAX_CONSOLIDATION_LOCKS && !guard.contains_key(entity_id) {
        prune_idle_consolidation_locks(&mut guard);
    }
    guard
        .entry(entity_id.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn prune_idle_consolidation_locks(locks: &mut BTreeMap<String, Arc<tokio::sync::Mutex<()>>>) {
    locks.retain(|_, lock| Arc::strong_count(lock) > 1);
}

#[cfg(test)]
mod tests {
    use super::{collect_context_tags, prune_idle_consolidation_locks};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[test]
    fn context_tags_use_companion_neutral_reasoning_label() {
        let tags = collect_context_tags("let's make a roadmap", "step 1: sketch the idea");
        assert!(tags.iter().any(|tag| tag == "structured_reasoning"));
        assert!(!tags.iter().any(|tag| tag == "planning"));
    }

    #[test]
    fn consolidation_lock_pruning_keeps_active_locks_only() {
        let active_lock = Arc::new(tokio::sync::Mutex::new(()));
        let active_clone = active_lock.clone();
        let mut locks = BTreeMap::from([
            ("person:active".to_string(), active_lock),
            (
                "person:idle".to_string(),
                Arc::new(tokio::sync::Mutex::new(())),
            ),
        ]);

        prune_idle_consolidation_locks(&mut locks);

        assert!(locks.contains_key("person:active"));
        assert!(!locks.contains_key("person:idle"));
        drop(active_clone);
    }
}

/// # Errors
///
/// Returns an error when consolidation state cannot be loaded or persisted, or
/// when writing the consolidation memory event fails.
pub async fn run_consolidation(
    memory: &dyn Memory,
    workspace_dir: &Path,
    input: &ConsolidationInput,
) -> Result<ConsolidationOutput> {
    if input.user_message.trim().is_empty() && input.assistant_response.trim().is_empty() {
        return Ok(ConsolidationOutput {
            disposition: ConsolidationDisposition::SkippedNoSignal,
            previous_watermark: 0,
            applied_watermark: 0,
        });
    }

    let entity_lock = consolidation_lock(input.entity_id.as_str());
    let _guard = entity_lock.lock().await;
    ensure_state_parent(workspace_dir)?;
    let mut state = load_state(workspace_dir)?;
    let previous_watermark = state
        .watermarks
        .get(input.entity_id.as_str())
        .copied()
        .unwrap_or_default();

    if input.checkpoint_event_count <= previous_watermark {
        return Ok(ConsolidationOutput {
            disposition: ConsolidationDisposition::SkippedCheckpoint,
            previous_watermark,
            applied_watermark: previous_watermark,
        });
    }

    let compacted = build_consolidation_value(input);
    memory
        .append_event(
            MemoryEventInput::new(
                input.entity_id.as_str(),
                CONSOLIDATION_SLOT_KEY,
                MemoryEventType::SummaryCompacted,
                compacted,
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Semantic)
            .with_confidence(0.85)
            .with_importance(0.65)
            .with_provenance(MemoryProvenance::source_reference(
                MemorySource::System,
                CONSOLIDATION_PROVENANCE_REF,
            )),
        )
        .await?;

    state
        .watermarks
        .insert(input.entity_id.to_string(), input.checkpoint_event_count);
    save_state(workspace_dir, &state)?;

    Ok(ConsolidationOutput {
        disposition: ConsolidationDisposition::Consolidated,
        previous_watermark,
        applied_watermark: input.checkpoint_event_count,
    })
}

/// # Errors
///
/// Returns an error when event counting fails before a consolidation task can
/// be scheduled.
pub async fn schedule_durable_memory_consolidation(
    memory: Arc<dyn Memory>,
    workspace_dir: PathBuf,
    entity_id: &str,
    user_message: &str,
    assistant_response: &str,
    observer: Arc<dyn Observer>,
) -> Result<()> {
    let checkpoint_event_count = memory.count_events(Some(entity_id)).await?;
    let input = ConsolidationInput::new(
        entity_id,
        checkpoint_event_count,
        user_message,
        assistant_response,
    );
    enqueue_consolidation_task(memory, workspace_dir, input, observer);
    Ok(())
}

/// Spawn a background task that runs a single consolidation pass.
pub fn enqueue_consolidation_task(
    memory: Arc<dyn Memory>,
    workspace_dir: PathBuf,
    input: ConsolidationInput,
    observer: Arc<dyn Observer>,
) {
    record_worker_queued(&input);
    tokio::spawn(async move {
        observer.emit_memory_signal(MemorySignal::ConsolidationStarted);
        record_worker_running(&input);
        match run_consolidation(memory.as_ref(), &workspace_dir, &input).await {
            Ok(output) => {
                record_worker_completed(&input, &output);
                observer.emit_memory_signal(MemorySignal::ConsolidationCompleted);
            }
            Err(error) => {
                record_worker_failed(&input, &error);
                tracing::warn!(error = %error, "post-turn consolidation task failed; answer path preserved");
            }
        }
    });
}
