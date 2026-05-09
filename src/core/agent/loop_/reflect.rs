//! Persona reflect/writeback: post-turn identity and memory refinement.
//!
//! # Why it exists
//!
//! `Asterel` maintains a persistent `StateHeader` that captures
//! the persona's current objective, open loops, commitments, and
//! context summary.  Without a writeback pass, the persona's internal
//! state would remain frozen regardless of what happened in the
//! conversation.
//!
//! After every non-ephemeral turn, `run_persona_reflect_writeback`
//! makes a **second deterministic LLM call** (the *reflect call*)
//! that:
//! 1. Reads the current `StateHeader` and the turn's user/assistant
//!    exchange.
//! 2. Produces a strict JSON payload covering: updated state header,
//!    memory-append strings, self-task enqueuing, style-profile
//!    updates, context-level memory inferences, and user inferences.
//! 3. Validates the payload against the **identity contract** (immutable
//!    fields, monotonic timestamp, continuity gate) — preventing the
//!    LLM from corrupting stable identity invariants.
//! 4. Persists each accepted component to its respective storage slot.
//!
//! # Validation pipeline
//!
//! ```text
//! invoke_reflect_provider()
//!     │  (calls reflect LLM, parses JSON)
//!     ▼
//! validate_writeback()                     [writeback_guard]
//!     │  (structural + immutable-field check)
//!     ▼
//! validate_writeback_candidate()
//!     │  (StateHeader schema + IdentityContractV1 mutation rules)
//!     ▼
//! check_continuity_gate()
//!     │  (drift / continuity score thresholds)
//!     ▼
//! persist: state_header, style_profile, memory_append,
//!          memory_inferences, user_inferences, self_tasks
//! ```
//!
//! Any failure in validation silently discards the payload and logs a
//! warning rather than aborting the turn — the answer path is always
//! preserved.

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde_json::Value;

use crate::config::Config;
use crate::contracts::strings::data_model::{
    ENTITY_PREFIX_PERSON, ENTITY_PREFIX_USER, PREFIX_PERSONA_WRITEBACK,
    RESERVED_SLOT_PREFIXES as CONTRACT_RESERVED_SLOT_PREFIXES, SLOT_CONVERSATION_ASSISTANT_RESP,
    SLOT_CONVERSATION_USER_MSG, SOURCE_REF_PERSONA_REFLECT_MEMORY_INFERENCE,
    SOURCE_REF_PERSONA_REFLECT_USER_INFERENCE,
};
use crate::contracts::strings::limits::{MAX_MEMORY_APPEND_ITEM_CHARS, MAX_SELF_TASK_EXPIRY_HOURS};
use crate::core::experience::{
    ExperienceAtom, ExperienceKind, ExperienceOutcome, persist_experience_atom,
    record_codespace_experience, record_self_task_experience,
};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};
use crate::core::persona::continuity_gate::{evaluate_continuity_gate, run_rollback_drill};
use crate::core::persona::identity_contract::IdentityContractV1;
use crate::core::persona::state_header::StateHeader;
use crate::core::persona::state_persistence::BackendHeaderPersist;
use crate::core::persona::style_profile::apply_style_profile_update;
use crate::core::providers::Provider;
use crate::security::writeback_guard::{
    AllowedWritebackSlot, ImmutableStateHeader, MemoryInferenceEntry, SelfTaskWriteback,
    StyleWriteback, WritebackPayload, WritebackPlanMetadata, WritebackVerdict,
    enforce_persona_long_term_write_policy, enforce_user_inference_write_policy,
    validate_writeback,
};

#[derive(Debug, Clone)]
struct ReflectWritebackMetadata {
    source_rationale: String,
    plan: WritebackPlanMetadata,
}

fn build_reflect_writeback_metadata() -> ReflectWritebackMetadata {
    let rationale =
        "persona reflect phase-1 writeback scope (persona + conversation working memory)";
    ReflectWritebackMetadata {
        source_rationale: rationale.to_string(),
        plan: WritebackPlanMetadata {
            allowed_slots: vec![
                AllowedWritebackSlot {
                    slot: SLOT_CONVERSATION_USER_MSG.to_string(),
                    source_rationale: rationale.to_string(),
                },
                AllowedWritebackSlot {
                    slot: SLOT_CONVERSATION_ASSISTANT_RESP.to_string(),
                    source_rationale: rationale.to_string(),
                },
                AllowedWritebackSlot {
                    slot: "user.*".to_string(),
                    source_rationale: rationale.to_string(),
                },
                AllowedWritebackSlot {
                    slot: "language.current".to_string(),
                    source_rationale: rationale.to_string(),
                },
                AllowedWritebackSlot {
                    slot: "topic.active".to_string(),
                    source_rationale: rationale.to_string(),
                },
                AllowedWritebackSlot {
                    slot: "timezone.current".to_string(),
                    source_rationale: rationale.to_string(),
                },
            ],
        },
    }
}

/// Build the system prompt for the reflect LLM call.
///
/// The prompt instructs the model to behave as a purely deterministic
/// JSON-emitting stage with no free-form prose.  The exact output
/// shape is specified inline so the model cannot deviate from the
/// expected schema.  Temperature 0.0 is used on the provider call to
/// maximize determinism and minimize hallucinated field values.
fn build_persona_reflect_system_prompt() -> String {
    format!(
        r#"You are a deterministic reflection/writeback stage.
Output must be a single strict JSON object, with no markdown and no extra text.

Required top-level shape:
{{
  "state_header": {{
    "identity_principles_hash": string,
    "safety_posture": string,
    "current_objective": string,
    "open_loops": string[],
    "next_actions": string[],
    "commitments": string[],
    "recent_context_summary": string,
    "last_updated_at": string (RFC3339)
  }},
  "memory_append": string[],
  "self_tasks": [
    {{
      "title": string,
      "instructions": string,
      "expires_at": string (RFC3339)
    }}
  ],
  "style_profile": {{
    "formality": integer (0-100),
    "verbosity": integer (0-100),
    "temperature": number (0.0-1.0)
  }},
  "memory_inferences": [
    {{
      "slot_key": string,
      "value": string
    }}
  ],
  "user_inferences": [
    {{
      "slot_key": string,
      "value": string
    }}
  ]
}}

`memory_inferences` (optional): inferred facts about the current conversation or near-term context.
`slot_key` is the suffix only; the runtime stores it as `inferred.<slot_key>`, so do NOT include the
leading `inferred.` prefix yourself. Use keys like `language.current`, `topic.active`, or
`timezone.current`. Values should be concise, factual claims. Only include high-confidence inferences.

`user_inferences` (optional): inferred facts specifically about the user (expertise, goals, preferences).
Slot keys MUST start with `user.` (e.g. `user.expertise.rust`, `user.goal.current`,
`user.preference.response_style`). Stable user preferences such as language or response style belong
here, not in `memory_inferences`.

`self_tasks` (optional): only include bounded tasks that should happen within the next
{MAX_SELF_TASK_EXPIRY_HOURS} hours relative to `state_header.last_updated_at`. If no such bounded
task exists, omit `self_tasks`.

`state_header.last_updated_at` must be RFC3339 and strictly later than the current canonical
state header's `last_updated_at`. If unsure, emit a fresh current timestamp instead of reusing the
previous one.

`memory_append` items must each be a short factual sentence no longer than
{MAX_MEMORY_APPEND_ITEM_CHARS} characters. If an item would be longer, shorten it instead of
writing a paragraph.

Do not include unknown keys.
Do not change immutable fields.
All optional top-level fields (`self_tasks`, `style_profile`, `memory_inferences`, `user_inferences`) are allowed but may be omitted.
If uncertain, keep mutable values close to current state."#
    )
}

fn build_reflect_message(
    canonical_state: Option<&StateHeader>,
    user_message: &str,
    answer: &str,
    experience_block: &str,
) -> Result<String> {
    let canonical_json = match canonical_state {
        Some(state) => {
            serde_json::to_string_pretty(state).context("serialize canonical state header")?
        }
        None => "null".to_string(),
    };

    let mut msg = format!(
        "Current canonical state header (JSON):\n{canonical_json}\n\nLatest user message:\n{user_message}\n\nLatest assistant answer:\n{answer}"
    );

    if !experience_block.is_empty() {
        msg.push_str("\n\n");
        msg.push_str(experience_block);
    }

    msg.push_str("\n\nReturn only the strict JSON payload.");
    Ok(msg)
}

fn parse_reflect_payload(raw: &str) -> Result<Value> {
    let mut payload: Value =
        serde_json::from_str(raw.trim()).context("parse reflect payload JSON")?;
    if !payload.is_object() {
        anyhow::bail!("reflect output must be a JSON object");
    }
    normalize_memory_append_entries(&mut payload);
    prune_out_of_horizon_self_tasks(&mut payload);
    Ok(payload)
}

fn normalize_memory_append_entries(payload: &mut Value) {
    let Some(root) = payload.as_object_mut() else {
        return;
    };
    let Some(entries) = root.get_mut("memory_append").and_then(Value::as_array_mut) else {
        return;
    };

    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries.iter() {
        let Some(raw) = entry.as_str() else {
            tracing::warn!("dropping non-string reflect memory_append entry before validation");
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            tracing::warn!("dropping empty reflect memory_append entry before validation");
            continue;
        }

        if trimmed.len() > MAX_MEMORY_APPEND_ITEM_CHARS {
            let char_count = trimmed.chars().count();
            if char_count > MAX_MEMORY_APPEND_ITEM_CHARS {
                let truncated = trimmed
                    .chars()
                    .take(MAX_MEMORY_APPEND_ITEM_CHARS)
                    .collect::<String>();
                tracing::warn!(
                    original_chars = char_count,
                    truncated_chars = MAX_MEMORY_APPEND_ITEM_CHARS,
                    "truncating overlong reflect memory_append entry before validation"
                );
                normalized.push(Value::String(truncated.trim().to_string()));
                continue;
            }
        }

        normalized.push(Value::String(trimmed.to_string()));
    }

    *entries = normalized;
}

fn prune_out_of_horizon_self_tasks(payload: &mut Value) {
    let Some(root) = payload.as_object_mut() else {
        return;
    };
    let Some(last_updated_at) = root
        .get("state_header")
        .and_then(Value::as_object)
        .and_then(|state_header| state_header.get("last_updated_at"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Ok(baseline) = DateTime::<FixedOffset>::parse_from_rfc3339(last_updated_at) else {
        return;
    };
    let max_expires_at = baseline + Duration::hours(MAX_SELF_TASK_EXPIRY_HOURS);
    let Some(tasks) = root.get_mut("self_tasks").and_then(Value::as_array_mut) else {
        return;
    };

    let original_len = tasks.len();
    tasks.retain(|task| {
        let Some(task_object) = task.as_object() else {
            return true;
        };
        let Some(expires_at) = task_object.get("expires_at").and_then(Value::as_str) else {
            return true;
        };
        let Ok(parsed_expires_at) = DateTime::<FixedOffset>::parse_from_rfc3339(expires_at) else {
            return true;
        };
        let within_horizon = parsed_expires_at > baseline && parsed_expires_at <= max_expires_at;
        if !within_horizon {
            tracing::warn!(
                expires_at,
                baseline = %baseline.to_rfc3339(),
                max_expires_at = %max_expires_at.to_rfc3339(),
                "dropping reflect self task outside allowed horizon"
            );
        }
        within_horizon
    });

    if tasks.len() != original_len {
        tracing::info!(
            dropped = original_len - tasks.len(),
            remaining = tasks.len(),
            "pruned out-of-horizon reflect self tasks before validation"
        );
    }
}

async fn apply_optional_style_profile(
    mem: &dyn Memory,
    person_id: &str,
    style_profile: Option<&StyleWriteback>,
    reflected_at: &str,
) {
    let Some(style_profile) = style_profile else {
        return;
    };

    match apply_style_profile_update(mem, person_id, style_profile, reflected_at).await {
        Ok(decision) => {
            tracing::info!(
                person_id,
                clamped = decision.clamped,
                requested_formality = decision.requested.formality,
                requested_verbosity = decision.requested.verbosity,
                requested_temperature = decision.requested.temperature,
                applied_formality = decision.applied.formality,
                applied_verbosity = decision.applied.verbosity,
                applied_temperature = decision.applied.temperature,
                "applied bounded style profile update"
            );
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "style profile adaptation failed; continuing with state writeback only"
            );
        }
    }
}

async fn append_memory_entries(
    mem: &dyn Memory,
    person_id: &str,
    entries: &[String],
    occurred_at: &str,
) -> Result<()> {
    for (idx, entry) in entries.iter().enumerate() {
        let input = MemoryEventInput::new(
            format!("{ENTITY_PREFIX_PERSON}{person_id}"),
            format!("{PREFIX_PERSONA_WRITEBACK}{idx}"),
            MemoryEventType::SummaryCompacted,
            entry.clone(),
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_confidence(0.9)
        .with_importance(0.8)
        .with_source_kind(SourceKind::Manual)
        .with_source_ref(format!("persona-reflect-memory-append:{idx}"))
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            "persona.reflect.memory_append",
        ))
        .with_occurred_at(occurred_at.to_string());
        enforce_persona_long_term_write_policy(&input, person_id)
            .context("enforce persona writeback policy")?;
        mem.append_event(input)
            .await
            .context("append persona writeback memory event")?;
    }

    Ok(())
}

async fn persist_reflect_experiences(
    mem: &dyn Memory,
    person_id: &str,
    self_tasks: &[SelfTaskWriteback],
    style_profile_applied: bool,
) {
    let experience_entity_id = format!("{ENTITY_PREFIX_PERSON}{person_id}");
    let mut atoms: Vec<ExperienceAtom> = Vec::new();
    atoms.push(
        ExperienceAtom::new(
            ExperienceKind::TurnInteraction,
            "Accepted and persisted reflect writeback payload",
            ExperienceOutcome::Success,
        )
        .with_lesson("Carry forward accepted writeback constraints in future turns")
        .with_confidence(0.9),
    );
    for task in self_tasks {
        atoms.push(record_self_task_experience(
            &task.title,
            &task.instructions,
            ExperienceOutcome::Unknown,
        ));
        let title_lower = task.title.to_lowercase();
        let instructions_lower = task.instructions.to_lowercase();
        let looks_like_codespace = title_lower.contains("codespace")
            || instructions_lower.contains("codespace")
            || title_lower.contains("project")
            || instructions_lower.contains("project");
        if looks_like_codespace {
            atoms.push(record_codespace_experience(
                "reflect-inferred",
                &task.title,
                ExperienceOutcome::Unknown,
                &task.instructions,
            ));
        }
    }
    if style_profile_applied {
        atoms.push(
            ExperienceAtom::new(
                ExperienceKind::PersonaWriteback,
                "Style profile was adapted from reflection writeback",
                ExperienceOutcome::Success,
            )
            .with_confidence(0.8),
        );
    }

    for atom in &atoms {
        if let Err(error) = persist_experience_atom(mem, &experience_entity_id, atom).await {
            tracing::warn!(
                error = %error,
                atom_id = %atom.id,
                "failed to persist experience atom"
            );
        }
    }
}

async fn maybe_run_rollback_drill(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    trigger: &str,
) {
    if !config.persona.enable_rollback_drills {
        return;
    }

    match run_rollback_drill(mem, &config.persona, person_id, trigger).await {
        Ok(result) => {
            tracing::info!(
                person_id,
                status = %result.status,
                trigger = %result.trigger,
                detail = %result.detail,
                "persona rollback drill completed"
            );
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                trigger,
                "persona rollback drill failed"
            );
        }
    }
}

async fn fetch_experience_block(mem: &dyn Memory, person_id: &str, user_message: &str) -> String {
    use crate::core::experience::{render_experience_block, retrieve_relevant_experiences};

    let entity_id = format!("{ENTITY_PREFIX_PERSON}{person_id}");
    let experiences = retrieve_relevant_experiences(mem, &entity_id, user_message, 5)
        .await
        .unwrap_or_default();
    render_experience_block(&experiences)
}

async fn invoke_reflect_provider(
    mem: &dyn Memory,
    provider: &dyn Provider,
    model_name: &str,
    person_id: &str,
    user_message: &str,
    answer: &str,
    canonical_state: Option<&StateHeader>,
) -> Result<Value> {
    let experience_block = fetch_experience_block(mem, person_id, user_message).await;
    let reflect_message =
        build_reflect_message(canonical_state, user_message, answer, &experience_block)
            .context("build persona reflect message")?;
    let system_prompt = build_persona_reflect_system_prompt();

    let reflect_raw = provider
        .chat_with_system(Some(&system_prompt), &reflect_message, model_name, 0.0)
        .await
        .context("call reflect provider for persona writeback")?;
    parse_reflect_payload(&reflect_raw).context("parse persona reflect payload")
}

/// Ensure the candidate `last_updated_at` is strictly later than the
/// previous state's timestamp, preserving monotonic ordering.
///
/// When the reflect LLM reuses an old timestamp (or produces one
/// equal to the previous value), this function advances it to
/// `max(now, previous + 1µs)` so downstream readers can rely on
/// `last_updated_at` as a monotonic version key.
fn normalized_candidate_last_updated_at(previous_state: &StateHeader, requested: &str) -> String {
    let Ok(previous_timestamp) = DateTime::parse_from_rfc3339(&previous_state.last_updated_at)
    else {
        return requested.to_string();
    };
    let Ok(requested_timestamp) = DateTime::parse_from_rfc3339(requested) else {
        return requested.to_string();
    };

    if requested_timestamp > previous_timestamp {
        return requested.to_string();
    }

    let normalized = std::cmp::max(
        Utc::now(),
        previous_timestamp.with_timezone(&Utc) + Duration::microseconds(1),
    )
    .to_rfc3339();
    tracing::info!(
        requested_last_updated_at = requested,
        previous_last_updated_at = %previous_state.last_updated_at,
        normalized_last_updated_at = %normalized,
        "reflect candidate reused stale last_updated_at; normalized to preserve monotonic state"
    );
    normalized
}

/// Construct the candidate `StateHeader` by merging the accepted
/// writeback payload with the immutable fields of the previous state.
///
/// Immutable fields (`identity_principles_hash`, `safety_posture`) are
/// always taken from `previous_state` — the reflect LLM is not
/// permitted to change them, and this merge enforces that constraint
/// independently of the validation guard.
fn build_writeback_candidate(
    previous_state: &StateHeader,
    accepted: &WritebackPayload,
) -> StateHeader {
    StateHeader {
        identity_principles_hash: previous_state.identity_principles_hash.clone(),
        safety_posture: previous_state.safety_posture.clone(),
        current_objective: accepted.state_header.current_objective.clone(),
        open_loops: accepted.state_header.open_loops.clone(),
        next_actions: accepted.state_header.next_actions.clone(),
        commitments: accepted.state_header.commitments.clone(),
        recent_context_summary: accepted.state_header.recent_context_summary.clone(),
        last_updated_at: normalized_candidate_last_updated_at(
            previous_state,
            &accepted.state_header.last_updated_at,
        ),
    }
}

/// Run the two-layer writeback validation:
/// 1. `StateHeader::validate_writeback_candidate` — structural rules
///    (schema version, field lengths, timestamp monotonicity).
/// 2. `IdentityContractV1::validate_mutation` — contract-level rules
///    (immutable fields unchanged, safety posture not downgraded).
///
/// Both checks must pass; the first failure short-circuits and the
/// entire writeback is discarded.
fn validate_writeback_candidate(
    config: &Config,
    previous_state: &StateHeader,
    candidate: &StateHeader,
) -> Result<()> {
    StateHeader::validate_writeback_candidate(previous_state, candidate, &config.persona)
        .context("validate persona writeback candidate")?;
    let previous_contract = IdentityContractV1::from_state_header(previous_state);
    let candidate_contract = IdentityContractV1::from_state_header(candidate);
    IdentityContractV1::validate_mutation(&previous_contract, &candidate_contract, &config.persona)
        .context("validate identity contract mutation")
}

/// Returns `true` if the continuity gate allows the writeback to proceed.
async fn check_continuity_gate(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    previous_state: &StateHeader,
    candidate: &StateHeader,
) -> bool {
    let continuity_gate = evaluate_continuity_gate(&config.persona, previous_state, candidate);
    if !continuity_gate.allows_writeback() {
        tracing::warn!(
            status = continuity_gate.status.as_str(),
            severity = ?continuity_gate.severity,
            continuity_score = continuity_gate.assessment.continuity_score,
            drift_score = continuity_gate.assessment.drift_score,
            stable_layer_changed = continuity_gate.assessment.stable_layer_changed,
            timestamp_regressed = continuity_gate.assessment.timestamp_regressed,
            "persona continuity gate blocked reflect writeback candidate"
        );
        // Emit drift event to memory (P-4): record critical drift for identity ledger.
        let entity_id = crate::core::persona::person_identity::person_entity_id(person_id);
        let drift_event = crate::core::persona::identity_events::build_drift_detected_event(
            &entity_id,
            "persona continuity gate blocked; drift exceeds critical threshold",
            continuity_gate.assessment.drift_score,
        );
        if let Err(error) = mem.append_event(drift_event).await {
            tracing::debug!(%error, "failed to emit drift detected event");
        }
        maybe_run_rollback_drill(config, mem, person_id, "continuity_gate_blocked").await;
        return false;
    }

    if continuity_gate.status.as_str() == "warning" {
        tracing::warn!(
            severity = ?continuity_gate.severity,
            continuity_score = continuity_gate.assessment.continuity_score,
            drift_score = continuity_gate.assessment.drift_score,
            "persona continuity gate accepted warning-level transition"
        );
        // Emit drift event to memory (P-4): record warning-level drift for identity ledger.
        let entity_id = crate::core::persona::person_identity::person_entity_id(person_id);
        let drift_event = crate::core::persona::identity_events::build_drift_detected_event(
            &entity_id,
            "persona continuity gate accepted warning-level transition; drift elevated",
            continuity_gate.assessment.drift_score,
        );
        if let Err(error) = mem.append_event(drift_event).await {
            tracing::debug!(%error, "failed to emit drift detected event");
        }
    }

    true
}

/// Execute the persona reflect/writeback pass: invoke the reflect
/// LLM, validate the payload against the identity contract, and
/// persist the updated state header, style profile, memory
/// entries, and self-tasks.
///
/// # Errors
///
/// Returns an error if the reflect provider call, payload
/// validation, or persistence fails.
pub(super) async fn run_persona_reflect_writeback(
    config: &Config,
    mem: Arc<dyn Memory>,
    provider: &dyn Provider,
    model_name: &str,
    person_id: &str,
    user_message: &str,
    answer: &str,
) -> Result<()> {
    let persistence = BackendHeaderPersist::new(
        mem.clone(),
        config.workspace_dir.clone(),
        config.persona.clone(),
        person_id,
    );

    let canonical_state = persistence
        .reconcile_mirror_from_backend_on_startup()
        .await
        .context("load canonical persona state")?;

    let reflect_payload = invoke_reflect_provider(
        mem.as_ref(),
        provider,
        model_name,
        person_id,
        user_message,
        answer,
        canonical_state.as_ref(),
    )
    .await?;

    let Some(previous_state) = canonical_state else {
        tracing::warn!("persona reflect produced payload but canonical state header is missing");
        return Ok(());
    };

    let immutable = ImmutableStateHeader {
        schema_version: 1,
        identity_principles_hash: previous_state.identity_principles_hash.clone(),
        safety_posture: previous_state.safety_posture.clone(),
    };

    let reflect_metadata = build_reflect_writeback_metadata();
    tracing::debug!(
        source_rationale = %reflect_metadata.source_rationale,
        allowed_slot_count = reflect_metadata.plan.allowed_slots.len(),
        "applying reflect writeback contract metadata"
    );
    let accepted =
        match validate_writeback(&reflect_payload, &immutable, Some(&reflect_metadata.plan)) {
            WritebackVerdict::Accepted(payload) => payload,
            WritebackVerdict::Rejected { reason } => {
                tracing::warn!(reason, "persona writeback rejected by guard");
                return Ok(());
            }
        };

    let candidate = build_writeback_candidate(&previous_state, &accepted);
    validate_writeback_candidate(config, &previous_state, &candidate)?;

    if !check_continuity_gate(config, mem.as_ref(), person_id, &previous_state, &candidate).await {
        return Ok(());
    }

    persistence
        .persist_backend_sync(&candidate)
        .await
        .context("persist canonical persona state")?;
    apply_optional_style_profile(
        mem.as_ref(),
        person_id,
        accepted.style_profile.as_ref(),
        &candidate.last_updated_at,
    )
    .await;
    append_memory_entries(
        mem.as_ref(),
        person_id,
        &accepted.memory_append,
        &candidate.last_updated_at,
    )
    .await?;

    // ── Memory inferences (INFERRED_CLAIM) ──────────────────────────
    persist_memory_inferences(mem.as_ref(), person_id, &accepted.memory_inferences).await;

    // ── User inferences (stored under user entity) ──────────────────
    persist_user_inferences(mem.as_ref(), person_id, &accepted.user_inferences).await;

    enqueue_reflect_self_tasks(mem.as_ref(), person_id, &accepted.self_tasks).await;
    persist_reflect_experiences(
        mem.as_ref(),
        person_id,
        &accepted.self_tasks,
        accepted.style_profile.is_some(),
    )
    .await;
    maybe_run_rollback_drill(config, mem.as_ref(), person_id, "post_writeback").await;

    Ok(())
}

async fn persist_memory_inferences(
    mem: &dyn Memory,
    person_id: &str,
    inferences: &[MemoryInferenceEntry],
) {
    for inference in inferences {
        let entity_id = format!("{ENTITY_PREFIX_PERSON}{person_id}");
        let slot_key = format!("inferred.{}", inference.slot_key);
        let input = MemoryEventInput::new(
            entity_id,
            slot_key,
            MemoryEventType::InferredClaim,
            &inference.value,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_confidence(0.75)
        .with_importance(0.7)
        .with_source_kind(SourceKind::Manual)
        .with_source_ref(SOURCE_REF_PERSONA_REFLECT_MEMORY_INFERENCE)
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            "persona.reflect.inferred_claim",
        ));

        if let Err(error) = enforce_persona_long_term_write_policy(&input, person_id) {
            tracing::warn!(
                %error,
                slot_key = %inference.slot_key,
                "memory inference rejected by write policy"
            );
            continue;
        }
        if let Err(error) = mem.append_event(input).await {
            tracing::warn!(
                %error,
                slot_key = %inference.slot_key,
                "failed to persist memory inference"
            );
        }
    }
}

/// Reserved slot key prefixes that must not be written via LLM reflect.
const REFLECT_RESERVED_PREFIXES: &[&str] = &[
    CONTRACT_RESERVED_SLOT_PREFIXES[0],
    CONTRACT_RESERVED_SLOT_PREFIXES[1],
    CONTRACT_RESERVED_SLOT_PREFIXES[2],
    CONTRACT_RESERVED_SLOT_PREFIXES[3],
    CONTRACT_RESERVED_SLOT_PREFIXES[4],
];

fn is_valid_reflect_slot_key(slot_key: &str) -> bool {
    !slot_key.is_empty()
        && slot_key.len() <= 128
        && slot_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !REFLECT_RESERVED_PREFIXES
            .iter()
            .any(|prefix| slot_key.starts_with(prefix))
        && !slot_key.contains("..")
}

async fn persist_user_inferences(
    mem: &dyn Memory,
    person_id: &str,
    inferences: &[MemoryInferenceEntry],
) {
    if inferences.is_empty() {
        return;
    }
    // Store under the user entity rather than the agent entity.
    // The person_id here serves as the user identifier context.
    let entity_id = format!("{ENTITY_PREFIX_USER}{person_id}");
    for inference in inferences {
        // Validate slot key to prevent overwriting system-owned slots.
        if !is_valid_reflect_slot_key(inference.slot_key.as_str()) {
            tracing::warn!(
                slot_key = %inference.slot_key,
                "rejecting user inference with invalid or reserved slot key"
            );
            continue;
        }
        let input = MemoryEventInput::new(
            &entity_id,
            inference.slot_key.as_str(),
            MemoryEventType::InferredClaim,
            &inference.value,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_confidence(0.7)
        .with_importance(0.6)
        .with_source_kind(SourceKind::Manual)
        .with_source_ref(SOURCE_REF_PERSONA_REFLECT_USER_INFERENCE)
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            "persona.reflect.user_inferred",
        ));

        if let Err(error) = enforce_user_inference_write_policy(&input, person_id) {
            tracing::warn!(
                %error,
                slot_key = %inference.slot_key,
                "user inference rejected by write policy"
            );
            continue;
        }
        if let Err(error) = mem.append_event(input).await {
            tracing::warn!(
                %error,
                slot_key = %inference.slot_key,
                "failed to persist user inference"
            );
        }
    }
}

// Self-task queue helpers extracted to self_task_queue.rs
use super::self_task_queue::enqueue_reflect_self_tasks;
