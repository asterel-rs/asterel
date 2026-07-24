//! Sub-agent runtime: global configuration, inline/background run
//! execution, lifecycle management, and tool-loop integration.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{AgentExtensionProfile, ExtensionLoader, SkillMetadataProvider};
use crate::config::SkillsRuntimeConfig;
use crate::contracts::ids::{EntityId, RunId};
use crate::core::agent::tool_loop::{ToolLoop, ToolLoopRunParams};
use crate::core::providers::Provider;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::registry::ToolRegistry;
use crate::security::{AutonomyLevel, SecurityPolicy};
use crate::utils::text::{sanitize_prompt_line, strip_internal_prompt_blocks, truncate_ellipsis};

/// Maximum tool loop iterations for subagent runs (capped well below parent agent).
const SUBAGENT_MAX_TOOL_ITERATIONS: u32 = 5;
const SUBAGENT_HANDOFF_LINE_MAX_CHARS: usize = 600;
const SUBAGENT_HANDOFF_CONTEXT_MAX_CHARS: usize = 3_000;
const SUBAGENT_ROOT_PARENT_ID: &str = "subagent-root";

fn sanitize_subagent_handoff_line(value: &str) -> String {
    truncate_ellipsis(
        sanitize_prompt_line(value).as_str(),
        SUBAGENT_HANDOFF_LINE_MAX_CHARS,
    )
}

fn sanitize_subagent_handoff_block(value: &str) -> String {
    let stripped = strip_internal_prompt_blocks(value);
    let mut sanitized =
        String::with_capacity(stripped.len().min(SUBAGENT_HANDOFF_CONTEXT_MAX_CHARS));
    for line in stripped.lines() {
        let line = sanitize_subagent_handoff_line(line);
        if line.is_empty() {
            continue;
        }
        if !sanitized.is_empty() {
            sanitized.push('\n');
        }
        sanitized.push_str(&line);
    }
    truncate_ellipsis(&sanitized, SUBAGENT_HANDOFF_CONTEXT_MAX_CHARS)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentHandoffEnvelope {
    pub objective: Option<String>,
    pub done_when: Option<String>,
    pub context: Option<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

impl SubagentHandoffEnvelope {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objective.is_none()
            && self.done_when.is_none()
            && self.context.is_none()
            && self.constraints.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentDelegationConfig {
    pub depth: u8,
    pub max_depth: u8,
    pub child_quota: u32,
}

#[derive(Clone)]
pub struct SubagentConfig {
    pub provider: Arc<dyn Provider>,
    pub system_prompt: String,
    pub default_model: String,
    pub default_temperature: f64,
    /// When present, subagent tool loop is enabled.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Workspace directory for tool execution context.
    pub workspace_dir: PathBuf,
    /// Security policy inherited from the parent runtime for skill loading.
    pub skill_loading_security: SecurityPolicy,
    /// Skill loading and hint configuration inherited from the parent runtime.
    pub skills: SkillsRuntimeConfig,
    /// Maximum allowed delegation depth for nested child agents.
    pub max_delegation_depth: u8,
    /// Direct-child quota granted to each subagent execution context.
    pub child_delegation_quota: u32,
    /// File-driven agent extension profiles available to subagents.
    pub agent_extensions: Vec<AgentExtensionProfile>,
    /// Optional loader for re-resolving agent extensions from a per-turn workspace.
    pub extension_loader: Option<Arc<dyn ExtensionLoader>>,
    pub skill_metadata_provider: Arc<dyn SkillMetadataProvider>,
}

pub struct SubagentDefaultRuntimeSpec<'a> {
    pub config: &'a crate::config::Config,
    pub system_prompt: &'a str,
    pub model_name: &'a str,
    pub temperature: f64,
    pub security: &'a SecurityPolicy,
    pub provider: Arc<dyn Provider>,
    pub registry: Arc<ToolRegistry>,
    pub extension_loader: Arc<dyn ExtensionLoader>,
    pub skill_metadata_provider: Arc<dyn SkillMetadataProvider>,
}

#[derive(Clone, Default)]
pub struct SubagentRunOptions {
    pub label: Option<String>,
    pub system_prompt_override: Option<String>,
    pub model_override: Option<String>,
    pub temperature_override: Option<f64>,
    pub handoff: Option<SubagentHandoffEnvelope>,
    pub delegation: Option<SubagentDelegationConfig>,
    pub parent_context: Option<ExecutionContext>,
}

impl fmt::Debug for SubagentRunOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubagentRunOptions")
            .field("label", &self.label)
            .field("system_prompt_override", &self.system_prompt_override)
            .field("model_override", &self.model_override)
            .field("temperature_override", &self.temperature_override)
            .field("handoff", &self.handoff)
            .field("delegation", &self.delegation)
            .field(
                "parent_entity_id",
                &self
                    .parent_context
                    .as_ref()
                    .map(|ctx| ctx.entity_id.as_str()),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentRunSnapshot {
    pub run_id: RunId,
    pub label: Option<String>,
    pub task: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_entity_id: Option<EntityId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff: Option<SubagentHandoffEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation: Option<SubagentDelegationConfig>,
    pub status: SubagentRunStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

struct SubagentRunEntry {
    snapshot: SubagentRunSnapshot,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct SubagentOrchestrator {
    runtime: RwLock<Option<SubagentConfig>>,
    runs: Mutex<HashMap<RunId, SubagentRunEntry>>,
}

#[cfg(test)]
pub(crate) static TEST_RUNTIME_LOCK: Mutex<()> = Mutex::new(());

static LEGACY_MANAGER: OnceLock<Arc<SubagentOrchestrator>> = OnceLock::new();

fn global_manager() -> &'static Arc<SubagentOrchestrator> {
    LEGACY_MANAGER.get_or_init(|| Arc::new(SubagentOrchestrator::new()))
}

impl SubagentOrchestrator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    /// Returns an error if the default runtime manager cannot be configured.
    pub fn configured_default(spec: SubagentDefaultRuntimeSpec<'_>) -> Result<Arc<Self>> {
        let manager = Arc::new(Self::new());
        manager.configure_default_runtime(spec)?;
        Ok(manager)
    }

    fn get_runtime(&self) -> Result<SubagentConfig> {
        let guard = self
            .runtime
            .read()
            .map_err(|error| anyhow::anyhow!("subagent runtime lock poisoned: {error}"))?;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("subagent runtime is not configured"))
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.runtime.read().is_ok_and(|guard| guard.is_some())
    }

    fn complete_run(&self, run_id: &RunId, result: Result<String>) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = runs.get_mut(run_id) {
            // Do not overwrite a Cancelled status — the cancel was authoritative.
            if entry.snapshot.status == SubagentRunStatus::Cancelled {
                return;
            }
            entry.snapshot.finished_at = Some(Utc::now().to_rfc3339());
            entry.handle = None;
            match result {
                Ok(output) => {
                    entry.snapshot.status = SubagentRunStatus::Completed;
                    entry.snapshot.output = Some(output);
                    entry.snapshot.error = None;
                }
                Err(error) => {
                    entry.snapshot.status = SubagentRunStatus::Failed;
                    entry.snapshot.output = None;
                    entry.snapshot.error = Some(error.to_string());
                }
            }
        }
    }

    /// # Errors
    /// Returns an error if the subagent runtime lock is poisoned.
    pub fn configure_runtime(&self, config: SubagentConfig) -> Result<()> {
        let mut guard = self
            .runtime
            .write()
            .map_err(|error| anyhow::anyhow!("subagent runtime lock poisoned: {error}"))?;
        *guard = Some(config);
        Ok(())
    }

    /// # Errors
    /// Returns an error if the subagent runtime lock is poisoned.
    pub fn configure_default_runtime(&self, spec: SubagentDefaultRuntimeSpec<'_>) -> Result<()> {
        self.configure_runtime(SubagentConfig {
            provider: spec.provider,
            system_prompt: spec.system_prompt.to_string(),
            default_model: spec.model_name.to_string(),
            default_temperature: spec.temperature,
            tool_registry: Some(spec.registry),
            workspace_dir: spec.config.workspace_dir.clone(),
            skill_loading_security: spec.security.clone(),
            skills: spec.config.skills.clone(),
            max_delegation_depth: crate::core::tools::DEFAULT_MAX_DELEGATION_DEPTH,
            child_delegation_quota: crate::core::tools::DEFAULT_SUBAGENT_CHILD_DELEGATION_QUOTA,
            agent_extensions: spec
                .extension_loader
                .load_agent_extensions_from_workspace(&spec.config.workspace_dir),
            extension_loader: Some(Arc::clone(&spec.extension_loader)),
            skill_metadata_provider: spec.skill_metadata_provider,
        })
    }

    /// # Errors
    /// Returns an error if runtime is not configured or provider inference fails.
    pub async fn run_inline(&self, task: String, model: Option<String>) -> Result<String> {
        self.run_inline_with_options(
            task,
            SubagentRunOptions {
                model_override: model,
                ..SubagentRunOptions::default()
            },
        )
        .await
    }

    /// # Errors
    /// Returns an error if runtime is not configured or provider inference fails.
    pub async fn run_inline_with_options(
        &self,
        task: String,
        options: SubagentRunOptions,
    ) -> Result<String> {
        let runtime = self.get_runtime()?;
        execute_subagent_task(&runtime, &task, &options).await
    }

    /// # Errors
    /// Returns an error if runtime is not configured or shared state locking fails.
    pub fn spawn(
        self: &Arc<Self>,
        task: String,
        label: Option<&str>,
        model: Option<&str>,
    ) -> Result<SubagentRunSnapshot> {
        self.spawn_with_options(
            task,
            SubagentRunOptions {
                label: label.map(ToString::to_string),
                model_override: model.map(ToString::to_string),
                ..SubagentRunOptions::default()
            },
        )
    }

    /// Build a lineage snapshot for spawn-limit checks.
    ///
    /// Converts the current run map into `LineageNode` slices. If the
    /// `parent_id` is not already present (root-level spawn from the main
    /// agent), a synthetic root node at depth 0 is appended so that
    /// `check_spawn_allowed` does not fail-safe-block the first spawn.
    fn build_lineage_for_parent(&self, parent_id: &str) -> Vec<super::spawn_limits::LineageNode> {
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut lineage: Vec<super::spawn_limits::LineageNode> = runs
            .values()
            .map(|entry| super::spawn_limits::LineageNode {
                agent_id: entry.snapshot.run_id.as_str().to_string(),
                parent_id: entry
                    .snapshot
                    .parent_entity_id
                    .as_ref()
                    .map(|id| id.as_str().to_string())
                    .or_else(|| {
                        (parent_id == SUBAGENT_ROOT_PARENT_ID)
                            .then(|| SUBAGENT_ROOT_PARENT_ID.to_string())
                    }),
                depth: entry.snapshot.delegation.map_or(0, |d| d.depth as usize),
            })
            .collect();
        // Root-level spawns: the main agent entity is not in the subagent run
        // map, so add it as a depth-0 root to satisfy the lineage walk.
        if !lineage.iter().any(|n| n.agent_id == parent_id) {
            lineage.push(super::spawn_limits::LineageNode {
                agent_id: parent_id.to_string(),
                parent_id: None,
                depth: 0,
            });
        }
        lineage
    }

    /// # Errors
    /// Returns an error if runtime is not configured or shared state locking fails.
    pub fn spawn_with_options(
        self: &Arc<Self>,
        task: String,
        options: SubagentRunOptions,
    ) -> Result<SubagentRunSnapshot> {
        // Enforce spawn depth/descendant limits (phase-J). Parentless root
        // spawns still share the synthetic root lineage so root-level fan-out
        // cannot bypass the same descendant cap.
        let parent_id = options
            .parent_context
            .as_ref()
            .map_or(SUBAGENT_ROOT_PARENT_ID, |ctx| ctx.entity_id.as_str());
        let lineage = self.build_lineage_for_parent(parent_id);
        super::spawn_limits::check_spawn_allowed(
            parent_id,
            &lineage,
            &super::spawn_limits::SpawnLimits::default(),
        )
        .map_err(|reason| anyhow::anyhow!("subagent spawn blocked: {reason}"))?;

        let runtime = self.get_runtime()?;
        let run_id = RunId::new(format!("subagent_{}", Uuid::new_v4().simple()));
        let extension = resolve_agent_extension(&runtime, &options);
        let snapshot = SubagentRunSnapshot {
            run_id,
            label: options.label.clone(),
            task: task.clone(),
            model: effective_model(&runtime, &options, extension.as_ref()),
            parent_entity_id: options
                .parent_context
                .as_ref()
                .map(|ctx| ctx.entity_id.clone()),
            handoff: options.handoff.clone(),
            delegation: options.delegation,
            status: SubagentRunStatus::Running,
            output: None,
            error: None,
            started_at: Utc::now().to_rfc3339(),
            finished_at: None,
        };

        {
            let mut runs = self
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            runs.insert(
                snapshot.run_id.clone(),
                SubagentRunEntry {
                    snapshot: snapshot.clone(),
                    handle: None,
                },
            );
        }

        let run_id_for_task = snapshot.run_id.clone();
        let _label_for_task = snapshot.label.clone();
        let manager = Arc::clone(self);
        let task_handle = tokio::spawn(async move {
            let result = execute_subagent_task(&runtime, &task, &options).await;

            manager.complete_run(&run_id_for_task, result);
        });

        {
            let mut runs = self
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(entry) = runs.get_mut(&snapshot.run_id)
                && entry.snapshot.status == SubagentRunStatus::Running
            {
                entry.handle = Some(task_handle);
            }
        }

        Ok(snapshot)
    }

    pub fn get(&self, run_id: &RunId) -> Option<SubagentRunSnapshot> {
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        runs.get(run_id).map(|entry| entry.snapshot.clone())
    }

    #[must_use]
    pub fn get_scoped(
        &self,
        run_id: &RunId,
        parent_entity_id: &str,
    ) -> Option<SubagentRunSnapshot> {
        self.get(run_id).filter(|snapshot| {
            snapshot.parent_entity_id.as_ref().map(EntityId::as_str) == Some(parent_entity_id)
        })
    }

    pub fn list(&self) -> Vec<SubagentRunSnapshot> {
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut snapshots = runs
            .values()
            .map(|entry| entry.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        snapshots
    }

    #[must_use]
    pub fn list_scoped(&self, parent_entity_id: &str) -> Vec<SubagentRunSnapshot> {
        self.list()
            .into_iter()
            .filter(|snapshot| {
                snapshot.parent_entity_id.as_ref().map(EntityId::as_str) == Some(parent_entity_id)
            })
            .collect()
    }

    /// # Errors
    /// Returns an error if the run ID does not exist.
    pub fn cancel(&self, run_id: &RunId) -> Result<()> {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = runs.get_mut(run_id) else {
            bail!("subagent run not found: {run_id}");
        };
        if entry.snapshot.status != SubagentRunStatus::Running {
            return Ok(());
        }
        if let Some(handle) = entry.handle.take() {
            handle.abort();
        }
        entry.snapshot.status = SubagentRunStatus::Cancelled;
        entry.snapshot.finished_at = Some(Utc::now().to_rfc3339());
        entry.snapshot.output = None;
        entry.snapshot.error = Some("cancelled".to_string());
        Ok(())
    }

    /// # Errors
    /// Returns an error if the run is not owned by the given parent entity or does not exist.
    pub fn cancel_scoped(&self, run_id: &RunId, parent_entity_id: &str) -> Result<()> {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = runs.get_mut(run_id) else {
            bail!("subagent run not found: {run_id}");
        };
        if entry
            .snapshot
            .parent_entity_id
            .as_ref()
            .map(EntityId::as_str)
            != Some(parent_entity_id)
        {
            bail!("subagent run not found: {run_id}");
        }
        if entry.snapshot.status != SubagentRunStatus::Running {
            return Ok(());
        }
        if let Some(handle) = entry.handle.take() {
            handle.abort();
        }
        entry.snapshot.status = SubagentRunStatus::Cancelled;
        entry.snapshot.finished_at = Some(Utc::now().to_rfc3339());
        entry.snapshot.output = None;
        entry.snapshot.error = Some("cancelled".to_string());
        Ok(())
    }
}

#[must_use]
pub fn is_configured() -> bool {
    global_manager().is_configured()
}

#[cfg(test)]
/// # Errors
/// Returns an error if the subagent runtime lock is poisoned.
pub fn configure_runtime(config: SubagentConfig) -> Result<()> {
    global_manager().configure_runtime(config)
}

#[cfg(test)]
/// # Errors
/// Returns an error if runtime is not configured or provider inference fails.
pub async fn run_inline(task: String, model: Option<String>) -> Result<String> {
    global_manager().run_inline(task, model).await
}

/// # Errors
/// Returns an error if runtime is not configured or provider inference fails.
pub async fn run_inline_with_options(task: String, options: SubagentRunOptions) -> Result<String> {
    global_manager()
        .run_inline_with_options(task, options)
        .await
}

#[cfg(test)]
/// # Errors
/// Returns an error if runtime is not configured or shared state locking fails.
pub fn spawn(
    task: String,
    label: Option<&str>,
    model: Option<&str>,
) -> Result<SubagentRunSnapshot> {
    global_manager().spawn(task, label, model)
}

/// # Errors
/// Returns an error if runtime is not configured or shared state locking fails.
pub fn spawn_with_options(
    task: String,
    options: SubagentRunOptions,
) -> Result<SubagentRunSnapshot> {
    global_manager().spawn_with_options(task, options)
}

#[cfg(test)]
#[must_use]
pub fn get(run_id: &RunId) -> Option<SubagentRunSnapshot> {
    global_manager().get(run_id)
}

#[must_use]
pub fn get_scoped(run_id: &RunId, parent_entity_id: &str) -> Option<SubagentRunSnapshot> {
    global_manager().get_scoped(run_id, parent_entity_id)
}

#[cfg(test)]
#[must_use]
pub fn list() -> Vec<SubagentRunSnapshot> {
    global_manager().list()
}

#[must_use]
pub fn list_scoped(parent_entity_id: &str) -> Vec<SubagentRunSnapshot> {
    global_manager().list_scoped(parent_entity_id)
}

#[cfg(test)]
/// # Errors
/// Returns an error if the run ID does not exist.
pub fn cancel(run_id: &RunId) -> Result<()> {
    global_manager().cancel(run_id)
}

/// # Errors
/// Returns an error if the run is not owned by the given parent entity or does not exist.
pub fn cancel_scoped(run_id: &RunId, parent_entity_id: &str) -> Result<()> {
    global_manager().cancel_scoped(run_id, parent_entity_id)
}

fn build_subagent_execution_context(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
) -> ExecutionContext {
    let mut ctx = if let Some(parent_ctx) = options.parent_context.as_ref() {
        let mut inherited = parent_ctx.clone();
        let mut policy = (*inherited.security).clone();
        policy.autonomy = AutonomyLevel::Supervised;
        policy.workspace_dir.clone_from(&inherited.workspace_dir);
        inherited.security = Arc::new(policy);
        inherited.autonomy_level = AutonomyLevel::Supervised;
        inherited.turn_number = 0;
        inherited.current_tool_capabilities.clear();
        inherited
    } else {
        let policy = SecurityPolicy {
            workspace_dir: runtime.workspace_dir.clone(),
            autonomy: AutonomyLevel::Supervised,
            ..SecurityPolicy::default()
        };
        ExecutionContext::from_security(Arc::new(policy))
    };
    ctx.entity_id = format!("subagent:{}", Uuid::new_v4().simple()).into();
    let delegation = options.delegation.unwrap_or(SubagentDelegationConfig {
        depth: 1,
        max_depth: runtime.max_delegation_depth,
        child_quota: runtime.child_delegation_quota,
    });
    ctx.delegation_depth = delegation.depth;
    ctx.max_delegation_depth = delegation.max_depth;
    ctx.child_delegation_quota = delegation.child_quota;
    ctx.remaining_child_delegations =
        Arc::new(std::sync::atomic::AtomicU32::new(delegation.child_quota));
    ctx
}

#[cfg(test)]
pub(crate) fn test_build_subagent_execution_context(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
) -> ExecutionContext {
    build_subagent_execution_context(runtime, options)
}

async fn execute_subagent_task(
    runtime: &SubagentConfig,
    task: &str,
    options: &SubagentRunOptions,
) -> Result<String> {
    let extension = resolve_agent_extension(runtime, options);
    let system_prompt = effective_system_prompt(runtime, options, extension.as_ref());
    let model_name = effective_model(runtime, options, extension.as_ref());
    let temperature = effective_temperature(runtime, options, extension.as_ref());
    let delegated_task = compose_subagent_task(task, options);
    let delegated_task =
        enrich_subagent_task_with_relevant_skills(runtime, options, &delegated_task);

    if let Some(ref registry) = runtime.tool_registry {
        let tool_loop = ToolLoop::new(Arc::clone(registry), SUBAGENT_MAX_TOOL_ITERATIONS);
        let ctx = build_subagent_execution_context(runtime, options);
        let result = tool_loop
            .run(ToolLoopRunParams {
                provider: runtime.provider.as_ref(),
                system_prompt: &system_prompt,
                user_message: &delegated_task,
                image_content: &[],
                model: &model_name,
                temperature,
                inference_options: None,
                ctx: &ctx,
                stream_sink: None,
                conversation_history: &[],
                state_notifier: None,
                checkpoint_dir: None,
            })
            .await?;
        Ok(result.final_text)
    } else {
        runtime
            .provider
            .chat_with_system(
                Some(system_prompt.as_str()),
                delegated_task.as_str(),
                &model_name,
                temperature,
            )
            .await
            .map_err(anyhow::Error::from)
    }
}

fn compose_subagent_task(task: &str, options: &SubagentRunOptions) -> String {
    let Some(handoff) = options.handoff.as_ref() else {
        return task.to_string();
    };
    if handoff.is_empty() {
        return task.to_string();
    }

    let mut composed = String::from("[Delegation Handoff]\n");
    if let Some(objective) = handoff
        .objective
        .as_deref()
        .map(sanitize_subagent_handoff_line)
        .filter(|value| !value.is_empty())
    {
        composed.push_str("Objective: ");
        composed.push_str(&objective);
        composed.push('\n');
    }
    if let Some(done_when) = handoff
        .done_when
        .as_deref()
        .map(sanitize_subagent_handoff_line)
        .filter(|value| !value.is_empty())
    {
        composed.push_str("Done When: ");
        composed.push_str(&done_when);
        composed.push('\n');
    }
    if !handoff.constraints.is_empty() {
        composed.push_str("Constraints:\n");
        for constraint in &handoff.constraints {
            let constraint = sanitize_subagent_handoff_line(constraint);
            if constraint.is_empty() {
                continue;
            }
            composed.push_str("- ");
            composed.push_str(&constraint);
            composed.push('\n');
        }
    }
    if let Some(context) = handoff
        .context
        .as_deref()
        .map(sanitize_subagent_handoff_block)
        .filter(|value| !value.is_empty())
    {
        composed.push_str("Context (sanitized untrusted handoff):\n");
        composed.push_str(&context);
        composed.push('\n');
    }
    composed.push_str("Task:\n");
    composed.push_str(task.trim());
    composed
}

fn enrich_subagent_task_with_relevant_skills(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
    task: &str,
) -> String {
    if runtime.skills.turn_hint_limit == 0 {
        return task.to_string();
    }

    let workspace_dir = options
        .parent_context
        .as_ref()
        .map_or(runtime.workspace_dir.as_path(), |ctx| {
            ctx.workspace_dir.as_path()
        });
    let security = options
        .parent_context
        .as_ref()
        .map_or(&runtime.skill_loading_security, |ctx| ctx.security.as_ref());
    let skill_snapshot = runtime
        .skill_metadata_provider
        .load_skill_metadata_snapshot_with_policy_and_config(
            workspace_dir,
            security,
            &runtime.skills,
        );
    let block = skill_snapshot.render_relevant_block(
        task,
        runtime.skills.prompt_description_chars,
        runtime.skills.turn_hint_limit,
    );
    if block.is_empty() {
        task.to_string()
    } else {
        format!("{block}{task}")
    }
}

fn resolve_agent_extension(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
) -> Option<AgentExtensionProfile> {
    let label = options.label.as_deref()?.trim();
    if label.is_empty() {
        return None;
    }

    let extensions = options
        .parent_context
        .as_ref()
        .and_then(|ctx| {
            runtime.extension_loader.as_ref().map(|loader| {
                loader.load_agent_extensions_from_workspace(ctx.workspace_dir.as_path())
            })
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| runtime.agent_extensions.clone());

    extensions
        .iter()
        .find(|extension| extension.id == label)
        .or_else(|| {
            extensions
                .iter()
                .find(|extension| extension.role.as_deref() == Some(label))
        })
        .cloned()
}

fn effective_system_prompt(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
    extension: Option<&AgentExtensionProfile>,
) -> String {
    let mut prompt = match extension {
        None => runtime.system_prompt.clone(),
        Some(extension) if extension.system_prompt.trim().is_empty() => {
            runtime.system_prompt.clone()
        }
        Some(extension) if runtime.system_prompt.trim().is_empty() => {
            extension.system_prompt.clone()
        }
        Some(extension) => format!(
            "{}\n\n## Agent Extension: {}\n{}",
            runtime.system_prompt, extension.id, extension.system_prompt
        ),
    };

    if let Some(override_prompt) = options
        .system_prompt_override
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        if prompt.trim().is_empty() {
            prompt = override_prompt.to_string();
        } else {
            prompt.push_str("\n\n## Subagent Runtime Override\n");
            prompt.push_str(override_prompt);
        }
    }

    prompt
}

fn effective_model(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
    extension: Option<&AgentExtensionProfile>,
) -> String {
    options
        .model_override
        .as_deref()
        .or_else(|| extension.and_then(|agent| agent.model.as_deref()))
        .unwrap_or(runtime.default_model.as_str())
        .to_string()
}

fn effective_temperature(
    runtime: &SubagentConfig,
    options: &SubagentRunOptions,
    extension: Option<&AgentExtensionProfile>,
) -> f64 {
    options
        .temperature_override
        .or_else(|| extension.and_then(|agent| agent.temperature))
        .unwrap_or(runtime.default_temperature)
}
