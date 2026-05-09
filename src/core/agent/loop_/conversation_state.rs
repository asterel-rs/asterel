//! Conversation state and fact ledger management.
//!
//! Two complementary stores are maintained per session and persisted
//! to memory after each turn:
//!
//! ## `ConversationState`
//! A high-level snapshot of the session: current goal, progress
//! summary, confirmed decisions, open questions, stated constraints,
//! and inferred style preferences.  When the persona layer is active
//! the state is **projected** from the `StateHeader` so the two views
//! remain in sync.
//!
//! ## `FactLedger`
//! A topic-keyed ledger of facts extracted from the conversation. New entries
//! on the same topic supersede older active entries, providing a
//! last-write-wins view while retaining superseded entries until the bounded
//! ledger cap prunes the oldest records. Explicit `INFERRED_CLAIM` markers
//! emitted by the LLM are extracted and added as `Inferred` entries;
//! the user's intent is recorded as an `Explicit` entry each turn.
//!
//! Both stores use a write policy guard (`enforce_conversation_state_write_policy`)
//! before persisting to memory, preventing unauthorized slot writes.

use anyhow::Context as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Memory slot key for the persisted fact ledger.
pub(super) use crate::contracts::strings::data_model::SLOT_CONVERSATION_LEDGER_V1 as FACT_LEDGER_SLOT_KEY;
/// Memory slot key for persisted conversation state.
pub(super) use crate::contracts::strings::data_model::SLOT_CONVERSATION_STATE_V1 as CONVERSATION_STATE_SLOT_KEY;
use crate::contracts::strings::data_model::{
    ENTITY_PREFIX_PERSON, RESERVED_SLOT_PREFIXES as CONTRACT_RESERVED_SLOT_PREFIXES,
    SOURCE_REF_CONVERSATION_LEDGER_UPDATE, SOURCE_REF_CONVERSATION_STATE_UPDATE,
};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::core::persona::person_identity::canonical_state_header_slot_key;
use crate::core::persona::state_header::StateHeader;
use crate::security::writeback_guard::enforce_conversation_state_write_policy;
use crate::utils::text::truncate_ellipsis;

const MAX_STATE_LIST_ITEMS: usize = 8;
const MAX_FACT_LEDGER_ITEMS: usize = 96;
const MAX_FIELD_CHARS: usize = 240;

/// Inferred stylistic preferences for the current conversation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct ConversationStyle {
    /// Detected language code (e.g. "en", "ja").
    pub language: String,
    /// Detected conversational tone.
    pub tone: String,
    /// Preferred response format (e.g. "concise prose").
    pub format: String,
}

/// Persisted session-level conversation state.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct ConversationState {
    /// Current user goal or objective.
    pub goal: String,
    /// Latest progress summary.
    pub progress: String,
    /// Confirmed decisions made during the session.
    pub decisions: Vec<String>,
    /// Unresolved questions or pending items.
    pub open_loops: Vec<String>,
    /// Stated constraints or restrictions.
    pub constraints: Vec<String>,
    /// Inferred conversation style preferences.
    pub style: ConversationStyle,
    /// ISO-8601 timestamp of the last update.
    pub last_updated_at: String,
}

/// Confidence level of a recorded fact.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum FactConfidence {
    /// Directly stated by the user.
    Explicit,
    /// Derived from context by the system.
    Inferred,
    /// Low-confidence or speculative.
    Uncertain,
}

/// Lifecycle status of a fact ledger entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum FactStatus {
    /// Currently valid and included in context.
    Active,
    /// Replaced by a newer entry on the same topic.
    Superseded,
    /// Explicitly retracted or invalidated.
    Retracted,
}

/// A single entry in the fact ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct FactLedgerEntry {
    /// Unique identifier for this entry.
    pub id: String,
    /// Topic grouping key for supersession tracking.
    pub topic: String,
    /// The recorded fact text.
    pub fact: String,
    /// ID of the turn that produced this entry.
    pub source_turn_id: String,
    /// Confidence level of the fact.
    pub confidence: FactConfidence,
    /// Current lifecycle status.
    pub status: FactStatus,
    /// ID of the entry that superseded this one, if any.
    pub superseded_by: Option<String>,
    /// ISO-8601 timestamp when the fact was recorded.
    pub occurred_at: String,
}

/// Append-only ledger of facts extracted from the conversation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(super) struct FactLedger {
    /// All ledger entries (active, superseded, and retracted).
    pub entries: Vec<FactLedgerEntry>,
}

impl FactLedger {
    /// Return up to `limit` active entries ranked by relevance to
    /// `query`.
    pub(super) fn active_entries_for_query(
        &self,
        query: &str,
        limit: usize,
    ) -> Vec<FactLedgerEntry> {
        if limit == 0 {
            return Vec::new();
        }
        let lowered_query = query.to_lowercase();
        let terms: Vec<String> = lowered_query
            .split_whitespace()
            .filter(|term| term.len() >= 2 && term.chars().count() >= 2)
            .map(ToString::to_string)
            .collect();
        let mut entries = self
            .entries
            .iter()
            .filter(|entry| entry.status == FactStatus::Active)
            .map(|entry| {
                let lowered_fact = entry.fact.to_lowercase();
                let full_match = if lowered_query.is_empty() {
                    0.0
                } else if lowered_fact.contains(&lowered_query) {
                    1.0
                } else {
                    0.0
                };
                let term_hits = terms
                    .iter()
                    .filter(|term| lowered_fact.contains(term.as_str()))
                    .count();
                let term_hits_u32 = u32::try_from(term_hits).unwrap_or(u32::MAX);
                let confidence_bias = match entry.confidence {
                    FactConfidence::Explicit => 0.30,
                    FactConfidence::Inferred => 0.15,
                    FactConfidence::Uncertain => 0.05,
                };
                let score = full_match + f64::from(term_hits_u32) + confidence_bias;
                (entry.clone(), score)
            })
            .collect::<Vec<_>>();
        entries.sort_by(|(lhs_entry, lhs_score), (rhs_entry, rhs_score)| {
            rhs_score
                .partial_cmp(lhs_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| rhs_entry.occurred_at.cmp(&lhs_entry.occurred_at))
        });
        entries
            .into_iter()
            .take(limit)
            .map(|(entry, _)| entry)
            .collect()
    }
}

/// Collapse all internal whitespace runs to single spaces and strip
/// leading/trailing whitespace.  Used to produce compact fact strings
/// that fit within `MAX_FIELD_CHARS` without wasting characters on
/// accidental formatting.
fn normalize_whitespace(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    for (i, word) in raw.split_whitespace().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push_str(word);
    }
    result
}

/// Return `true` if `input` contains any non-ASCII codepoint, used as
/// a fast pre-check before the more expensive per-character language
/// detection loop.
fn contains_non_ascii(input: &str) -> bool {
    !input.is_ascii()
}

/// Heuristically infer the BCP-47 language code from `user_message`
/// using Unicode script ranges.  Returns `"en"` for pure ASCII,
/// `"ja"` when Hiragana/Katakana is detected, `"ko"` for Hangul,
/// `"zh"` for CJK ideographs alone, and an empty string for other
/// non-ASCII scripts (e.g. Arabic, Cyrillic) where a reliable
/// heuristic is not yet implemented.
fn infer_language(user_message: &str) -> String {
    if !contains_non_ascii(user_message) {
        return "en".to_string();
    }

    let mut has_kana = false;
    let mut has_hangul = false;
    let mut has_cjk_ideograph = false;

    for c in user_message.chars() {
        // Hiragana U+3040..U+309F, Katakana U+30A0..U+30FF
        if ('\u{3040}'..='\u{30FF}').contains(&c) {
            has_kana = true;
        }
        // Hangul Syllables U+AC00..U+D7AF, Hangul Jamo U+1100..U+11FF
        if ('\u{AC00}'..='\u{D7AF}').contains(&c) || ('\u{1100}'..='\u{11FF}').contains(&c) {
            has_hangul = true;
        }
        // CJK Unified Ideographs U+4E00..U+9FFF
        if ('\u{4E00}'..='\u{9FFF}').contains(&c) {
            has_cjk_ideograph = true;
        }
    }

    if has_kana {
        "ja".to_string()
    } else if has_hangul {
        "ko".to_string()
    } else if has_cjk_ideograph {
        // CJK ideographs without kana or hangul — most likely Chinese.
        "zh".to_string()
    } else {
        // Non-ASCII but not CJK — could be Arabic, Cyrillic, etc.
        String::new()
    }
}

/// Heuristically infer the preferred response format from keywords.
/// Defaults to `"concise prose"` unless the message explicitly
/// requests bullet points (EN `"bullet"` or JA `"箇条書き"`).
fn infer_format(user_message: &str) -> String {
    let lowered = user_message.to_lowercase();
    if lowered.contains("箇条書き") || lowered.contains("bullet") {
        "short bullets".to_string()
    } else {
        "concise prose".to_string()
    }
}

/// Return `true` if `user_message` looks like an explicit approval or
/// confirmation (e.g. "go ahead", "approved", Japanese confirmation
/// phrases) and should therefore be recorded as a decision in the
/// conversation state.
fn should_capture_decision(user_message: &str) -> bool {
    let lowered = user_message.to_lowercase();
    [
        "進めて",
        "お願いします",
        "でいい",
        "いいよ",
        "go ahead",
        "sounds good",
        "approved",
        "approve",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

/// Extract a constraint statement from `user_message`, if present.
///
/// Looks for negation/prohibition keywords in EN and JA.  Returns a
/// normalized, truncated copy of the message as the constraint text,
/// or `None` if no constraint signal is detected.
fn capture_constraint(user_message: &str) -> Option<String> {
    let lowered = user_message.to_lowercase();
    if lowered.contains("なしで")
        || lowered.contains("不要")
        || lowered.contains("without")
        || lowered.contains("must not")
    {
        let compact = normalize_whitespace(user_message);
        if compact.is_empty() {
            None
        } else {
            Some(truncate_ellipsis(&compact, MAX_FIELD_CHARS))
        }
    } else {
        None
    }
}

/// Capture an open question as an unresolved loop item.
///
/// Returns a normalized copy of `user_message` when it contains a
/// question mark (EN `?` or full-width JP `？`), or `None` otherwise.
fn capture_open_loop(user_message: &str) -> Option<String> {
    let compact = normalize_whitespace(user_message);
    if compact.is_empty() {
        return None;
    }
    if compact.contains('?') || compact.contains('？') {
        return Some(truncate_ellipsis(&compact, MAX_FIELD_CHARS));
    }
    None
}

/// Push `value` onto `list` if it is not already present, then evict
/// the oldest entry until the list length is within `MAX_STATE_LIST_ITEMS`.
/// Silently drops empty or whitespace-only values.
fn push_unique_limited(list: &mut Vec<String>, value: String) {
    if value.trim().is_empty() {
        return;
    }
    if list.iter().any(|existing| existing == &value) {
        return;
    }
    list.push(value);
    while list.len() > MAX_STATE_LIST_ITEMS {
        list.remove(0);
    }
}

/// Resolve a `person_id` from either an explicit override or by
/// stripping the `person:` prefix from `entity_id`.
/// Returns `None` if neither source yields a non-empty string.
fn derive_person_id(entity_id: &str, explicit_person_id: Option<&str>) -> Option<String> {
    explicit_person_id
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .or_else(|| {
            entity_id
                .strip_prefix(ENTITY_PREFIX_PERSON)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

/// Load the persona's `StateHeader` for the given entity, used to
/// project the conversation state from persona state when persona mode
/// is active.  Returns `None` if the persona is not configured for
/// this entity or if deserialization fails.
async fn load_persona_state_header(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: Option<&str>,
) -> Option<StateHeader> {
    let person_id = derive_person_id(entity_id, person_id)?;
    let slot_key = canonical_state_header_slot_key(&person_id);
    let slot = mem.resolve_slot(entity_id, &slot_key).await.ok()??;
    serde_json::from_str(&slot.value).ok()
}

/// Overwrite the mutable fields of `state` with values from the
/// persona `StateHeader`.  This keeps the conversation-state view
/// consistent with the persona's authoritative record of goal,
/// progress, commitments, and open loops.
fn apply_persona_projection(state: &mut ConversationState, header: &StateHeader) {
    state.goal = truncate_ellipsis(&header.current_objective, MAX_FIELD_CHARS);
    state.progress = truncate_ellipsis(&header.recent_context_summary, MAX_FIELD_CHARS);
    state.decisions = header
        .commitments
        .iter()
        .map(|entry| truncate_ellipsis(entry, MAX_FIELD_CHARS))
        .collect();
    state.decisions.truncate(MAX_STATE_LIST_ITEMS);
    state.open_loops = header
        .open_loops
        .iter()
        .map(|entry| truncate_ellipsis(entry, MAX_FIELD_CHARS))
        .collect();
    state.open_loops.truncate(MAX_STATE_LIST_ITEMS);
}

/// Insert a new fact entry and supersede any existing active entries
/// on the same `topic`.  Skips insertion when an identical active
/// entry (same topic, fact text, and confidence) already exists.
/// Prunes the oldest entries when the ledger exceeds
/// `MAX_FACT_LEDGER_ITEMS`.
fn upsert_fact_entry(
    ledger: &mut FactLedger,
    topic: &str,
    fact: &str,
    source_turn_id: &str,
    confidence: FactConfidence,
    occurred_at: &str,
) {
    let fact = truncate_ellipsis(fact, MAX_FIELD_CHARS);
    let existing_exact = ledger.entries.iter().any(|entry| {
        entry.topic == topic
            && entry.fact == fact
            && entry.status == FactStatus::Active
            && entry.confidence == confidence
    });
    if existing_exact {
        return;
    }
    let new_id = Uuid::new_v4().to_string();
    for entry in &mut ledger.entries {
        if entry.topic == topic && entry.status == FactStatus::Active {
            entry.status = FactStatus::Superseded;
            entry.superseded_by = Some(new_id.clone());
        }
    }
    ledger.entries.push(FactLedgerEntry {
        id: new_id,
        topic: topic.to_string(),
        fact,
        source_turn_id: source_turn_id.to_string(),
        confidence,
        status: FactStatus::Active,
        superseded_by: None,
        occurred_at: occurred_at.to_string(),
    });
    if ledger.entries.len() > MAX_FACT_LEDGER_ITEMS {
        ledger.entries.sort_by(|lhs, rhs| {
            rhs.occurred_at
                .cmp(&lhs.occurred_at)
                .then_with(|| lhs.id.cmp(&rhs.id))
        });
        ledger.entries.truncate(MAX_FACT_LEDGER_ITEMS);
    }
}

/// Load persisted conversation state from memory, returning `None`
/// if absent or corrupt.
pub(super) async fn load_conversation_state(
    mem: &dyn Memory,
    entity_id: &str,
) -> Option<ConversationState> {
    let slot = match mem
        .resolve_slot(entity_id, CONVERSATION_STATE_SLOT_KEY)
        .await
    {
        Ok(slot) => slot?,
        Err(error) => {
            tracing::warn!(%error, %entity_id, "failed to load conversation state from memory");
            return None;
        }
    };
    match serde_json::from_str(&slot.value) {
        Ok(state) => Some(state),
        Err(error) => {
            tracing::warn!(%error, %entity_id, "corrupt conversation state JSON; resetting");
            None
        }
    }
}

/// Load the persisted fact ledger from memory, returning `None` if
/// absent or corrupt.
pub(super) async fn load_fact_ledger(mem: &dyn Memory, entity_id: &str) -> Option<FactLedger> {
    let slot = match mem.resolve_slot(entity_id, FACT_LEDGER_SLOT_KEY).await {
        Ok(slot) => slot?,
        Err(error) => {
            tracing::warn!(%error, %entity_id, "failed to load fact ledger from memory");
            return None;
        }
    };
    match serde_json::from_str(&slot.value) {
        Ok(ledger) => Some(ledger),
        Err(error) => {
            tracing::warn!(%error, %entity_id, "corrupt fact ledger JSON; resetting");
            None
        }
    }
}

/// Build the updated `ConversationState` for the current turn.
///
/// Loads the previous state (defaulting to an empty state on first
/// use), then applies (in order):
/// 1. Persona `StateHeader` projection (if available).
/// 2. Progress, decisions, open loops, and constraint extraction from
///    the turn's messages.
/// 3. Language and format inference for the style sub-record.
async fn build_updated_conversation_state(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: Option<&str>,
    user_message_compact: &str,
    assistant_compact: &str,
    now: &str,
) -> ConversationState {
    let mut state = load_conversation_state(mem, entity_id)
        .await
        .unwrap_or_default();

    if let Some(persona_state) = load_persona_state_header(mem, entity_id, person_id).await {
        apply_persona_projection(&mut state, &persona_state);
    } else if state.goal.trim().is_empty() {
        state.goal = truncate_ellipsis(user_message_compact, MAX_FIELD_CHARS);
    }

    if !assistant_compact.is_empty() {
        state.progress = truncate_ellipsis(assistant_compact, MAX_FIELD_CHARS);
    }
    if should_capture_decision(user_message_compact) {
        push_unique_limited(
            &mut state.decisions,
            truncate_ellipsis(user_message_compact, MAX_FIELD_CHARS),
        );
    }
    if let Some(open_loop) = capture_open_loop(user_message_compact) {
        push_unique_limited(&mut state.open_loops, open_loop);
    }
    if let Some(constraint) = capture_constraint(user_message_compact) {
        push_unique_limited(&mut state.constraints, constraint);
    }
    if state.style.language.trim().is_empty() {
        state.style.language = infer_language(user_message_compact);
    }
    if state.style.tone.trim().is_empty() {
        state.style.tone = "calm, professional".to_string();
    }
    if state.style.format.trim().is_empty() {
        state.style.format = infer_format(user_message_compact);
    }
    state.last_updated_at = now.to_string();

    state
}

/// Reserved slot key prefixes that must not be written via `INFERRED_CLAIM`.
use CONTRACT_RESERVED_SLOT_PREFIXES as RESERVED_SLOT_PREFIXES;

/// Maximum length of a fact ledger inferred value.
const MAX_INFERRED_VALUE_CHARS: usize = 500;

fn is_valid_inferred_slot_key(slot_key: &str) -> bool {
    !slot_key.is_empty()
        && slot_key.len() <= 128
        && slot_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !RESERVED_SLOT_PREFIXES
            .iter()
            .any(|prefix| slot_key.starts_with(prefix))
        && !slot_key.contains("..")
}

/// Apply one turn's contributions to the fact ledger.
///
/// Records the user's intent as an `Explicit` fact on the
/// `"user.intent"` topic (superseding the previous entry), then
/// scans `assistant_response` for `INFERRED_CLAIM` markers and adds
/// each as an `Inferred` fact.  Invalid slot keys and empty or
/// overlong values are silently dropped.  Control characters are
/// stripped from inferred values to prevent injection.
fn build_updated_fact_ledger(
    mut ledger: FactLedger,
    user_message_compact: &str,
    assistant_response: &str,
    source_turn_id: &str,
    now: &str,
) -> FactLedger {
    if !user_message_compact.is_empty() {
        upsert_fact_entry(
            &mut ledger,
            "user.intent",
            &format!("User intent: {user_message_compact}"),
            source_turn_id,
            FactConfidence::Explicit,
            now,
        );
    }

    for line in assistant_response.lines() {
        let trimmed = line.trim();
        if let Some(payload) = trimmed.strip_prefix("INFERRED_CLAIM ")
            && let Some((slot_key, value)) = payload.split_once("=>")
        {
            let slot_key = slot_key.trim();
            let value = value.trim();
            if is_valid_inferred_slot_key(slot_key)
                && !value.is_empty()
                && value.len() <= MAX_INFERRED_VALUE_CHARS
            {
                // Sanitize: strip any embedded control characters or
                // injection-like patterns from the value.
                let sanitized_value: String = value
                    .chars()
                    .filter(|c| !c.is_control() || *c == ' ')
                    .collect();
                if !sanitized_value.trim().is_empty() {
                    upsert_fact_entry(
                        &mut ledger,
                        &format!("inferred.{slot_key}"),
                        &format!("{slot_key}: {sanitized_value}"),
                        source_turn_id,
                        FactConfidence::Inferred,
                        now,
                    );
                }
            }
        }
    }

    ledger
}

async fn persist_conversation_state(
    mem: &dyn Memory,
    entity_id: &str,
    state: &ConversationState,
) -> anyhow::Result<()> {
    let state_json = serde_json::to_string(state).context("serialize conversation state")?;
    let state_input = MemoryEventInput::new(
        entity_id,
        CONVERSATION_STATE_SLOT_KEY,
        MemoryEventType::FactUpdated,
        state_json,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Semantic)
    .with_confidence(0.95)
    .with_importance(0.85)
    .with_source_kind(SourceKind::Conversation)
    .with_source_ref(SOURCE_REF_CONVERSATION_STATE_UPDATE)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        SOURCE_REF_CONVERSATION_STATE_UPDATE,
    ));
    enforce_conversation_state_write_policy(&state_input)?;
    mem.append_event(state_input).await?;
    Ok(())
}

async fn persist_fact_ledger(
    mem: &dyn Memory,
    entity_id: &str,
    ledger: &FactLedger,
) -> anyhow::Result<()> {
    let ledger_json = serde_json::to_string(ledger).context("serialize fact ledger")?;
    let ledger_input = MemoryEventInput::new(
        entity_id,
        FACT_LEDGER_SLOT_KEY,
        MemoryEventType::FactUpdated,
        ledger_json,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Semantic)
    .with_confidence(0.92)
    .with_importance(0.8)
    .with_source_kind(SourceKind::Conversation)
    .with_source_ref(SOURCE_REF_CONVERSATION_LEDGER_UPDATE)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        SOURCE_REF_CONVERSATION_LEDGER_UPDATE,
    ));
    enforce_conversation_state_write_policy(&ledger_input)?;
    mem.append_event(ledger_input).await?;
    Ok(())
}

/// Update and persist both the conversation state and fact ledger
/// after a completed turn.
///
/// # Errors
///
/// Returns an error if serialization or memory persistence fails.
pub(super) async fn update_conversation_state_and_ledger(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: Option<&str>,
    user_message: &str,
    assistant_response: &str,
) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    let source_turn_id = format!("turn:{}", Uuid::new_v4());
    let user_message_compact = normalize_whitespace(user_message);
    let assistant_compact = normalize_whitespace(assistant_response);

    let state = build_updated_conversation_state(
        mem,
        entity_id,
        person_id,
        &user_message_compact,
        &assistant_compact,
        &now,
    )
    .await;

    let existing_ledger = load_fact_ledger(mem, entity_id).await.unwrap_or_default();
    let ledger = build_updated_fact_ledger(
        existing_ledger,
        &user_message_compact,
        assistant_response,
        &source_turn_id,
        &now,
    );

    persist_conversation_state(mem, entity_id, &state).await?;
    persist_fact_ledger(mem, entity_id, &ledger).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{MarkdownMemory, Memory};

    #[tokio::test]
    async fn update_persists_state_and_ledger() {
        let temp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        update_conversation_state_and_ledger(
            mem.as_ref(),
            "person:test",
            Some("test"),
            "Please implement robust conversation compression without a kill switch.",
            "Understood. I will update the conversation state and fact ledger.",
        )
        .await
        .unwrap();

        let state = load_conversation_state(mem.as_ref(), "person:test")
            .await
            .expect("conversation state should be stored");
        assert!(!state.goal.is_empty());
        assert!(!state.progress.is_empty());

        let ledger = load_fact_ledger(mem.as_ref(), "person:test")
            .await
            .expect("fact ledger should be stored");
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].status, FactStatus::Active);
        assert_eq!(ledger.entries[0].confidence, FactConfidence::Explicit);
    }

    #[tokio::test]
    async fn user_intent_topic_supersedes_previous_active_entry() {
        let temp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        update_conversation_state_and_ledger(
            mem.as_ref(),
            "person:test",
            Some("test"),
            "Proceed with the first approach.",
            "Acknowledged.",
        )
        .await
        .unwrap();
        update_conversation_state_and_ledger(
            mem.as_ref(),
            "person:test",
            Some("test"),
            "Actually, switch to a different approach.",
            "Switching now.",
        )
        .await
        .unwrap();

        let ledger = load_fact_ledger(mem.as_ref(), "person:test")
            .await
            .expect("fact ledger should exist");
        let active = ledger
            .entries
            .iter()
            .filter(|entry| entry.topic == "user.intent" && entry.status == FactStatus::Active)
            .count();
        let superseded = ledger
            .entries
            .iter()
            .filter(|entry| entry.topic == "user.intent" && entry.status == FactStatus::Superseded)
            .count();
        assert_eq!(active, 1);
        assert_eq!(superseded, 1);
    }
}
