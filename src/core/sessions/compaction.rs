//! Token-aware session compaction pipeline.
//!
//! The flow has four phases:
//! 1. check if compaction should run (token threshold + tracked token telemetry),
//! 2. microcompact oversized tool outputs to reduce summarization noise,
//! 3. summarize older messages into a structured system summary,
//! 4. rehydrate continuity context after summary insertion.

use std::fmt::Write;

use anyhow::Result;

use super::compaction_context::{CompanionStateSnapshot, render_rehydration_block};
use super::store::PostgresSessionStore;
use super::types::{ChatMessage, CompactionConfig, CompactionResult, MessageRole};
use crate::contracts::ids::SessionId;
use crate::utils::text::truncate_ellipsis;

const MICROCOMPACT_HEAD_CHARS: usize = 200;
const KEY_EXCHANGE_ITEM_CHARS: usize = 300;
const TOOL_ACTIVITY_LIMIT: usize = 5;
const KEY_EXCHANGE_LIMIT: usize = 5;
const COMPANION_STATE_HEADER: &str = "## Companion State (restored after compaction)";
const COMPANION_GENERATION_PREFIX: &str = "Compaction generation: ";

fn tail_chars(value: &str, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let start = value
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map_or(0, |(index, _)| index);
    value[start..].to_string()
}

fn contains_tool_output_pattern(content: &str) -> bool {
    content
        .lines()
        .any(|line| line.trim_start().starts_with("Tool result:"))
}

fn prune_tool_output(content: &str, hot_tail_chars: usize) -> String {
    // Byte length is a lower bound on char count. If bytes already fit within
    // the head+tail budget, char-counting is unnecessary — return early.
    if content.len() <= MICROCOMPACT_HEAD_CHARS.saturating_add(hot_tail_chars) {
        return content.to_string();
    }
    // Single char-count pass; derive head/tail sizes from the result.
    let total_chars = content.chars().count();
    let head_len = total_chars.min(MICROCOMPACT_HEAD_CHARS);
    let tail_len = total_chars.min(hot_tail_chars);

    // If head + tail together cover (or overlap) the whole content, nothing is pruned.
    let retained = head_len + tail_len;
    if retained >= total_chars {
        return content.to_string();
    }

    let head: String = content.chars().take(head_len).collect();
    let tail = tail_chars(content, tail_len);
    let pruned = total_chars - retained;
    format!("{head}[... {pruned} chars pruned ...]{tail}")
}

fn decision_markers() -> &'static [&'static str] {
    &[
        "進めて",
        "お願いします",
        "でいい",
        "いいよ",
        "go ahead",
        "sounds good",
        "approved",
        "approve",
    ]
}

fn active_context_line(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map_or_else(
            || "No active user goal captured.".to_string(),
            |message| truncate_ellipsis(&message.content, KEY_EXCHANGE_ITEM_CHARS),
        )
}

fn collect_exchange_pairs(messages: &[ChatMessage]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut pending_user: Option<String> = None;
    for message in messages {
        match message.role {
            MessageRole::User => {
                pending_user = Some(truncate_ellipsis(&message.content, KEY_EXCHANGE_ITEM_CHARS));
            }
            MessageRole::Assistant => {
                if let Some(user_content) = pending_user.take() {
                    pairs.push((
                        user_content,
                        truncate_ellipsis(&message.content, KEY_EXCHANGE_ITEM_CHARS),
                    ));
                }
            }
            MessageRole::System => {}
        }
    }
    pairs
}

fn append_key_exchanges(summary: &mut String, messages: &[ChatMessage]) {
    summary.push_str("## Key exchanges\n");
    let pairs = collect_exchange_pairs(messages);
    if pairs.is_empty() {
        summary.push_str("- No user/assistant exchange pairs available.\n\n");
        return;
    }

    let mut recent_pairs = pairs
        .into_iter()
        .rev()
        .take(KEY_EXCHANGE_LIMIT)
        .collect::<Vec<_>>();
    recent_pairs.reverse();
    for (idx, (user_content, assistant_content)) in recent_pairs.iter().enumerate() {
        let pair_number = idx + 1;
        let _ = write!(
            summary,
            "- Pair {pair_number}\n  user: {user_content}\n  assistant: {assistant_content}\n"
        );
    }
    summary.push('\n');
}

fn collect_tool_mentions(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .map(|message| message.content.trim())
        .filter(|content| {
            let lowered = content.to_lowercase();
            lowered.contains("tool") || content.contains("Tool result:")
        })
        .map(|content| truncate_ellipsis(content, KEY_EXCHANGE_ITEM_CHARS))
        .collect()
}

fn append_tool_activity(summary: &mut String, messages: &[ChatMessage]) {
    let mut tool_mentions = collect_tool_mentions(messages);
    summary.push_str("## Tool activity\n");
    if tool_mentions.is_empty() {
        summary.push_str("- No notable tool output activity captured.\n\n");
        return;
    }

    if tool_mentions.len() > TOOL_ACTIVITY_LIMIT {
        let start = tool_mentions.len() - TOOL_ACTIVITY_LIMIT;
        tool_mentions = tool_mentions.split_off(start);
    }
    for mention in tool_mentions {
        let _ = writeln!(summary, "- {mention}");
    }
    summary.push('\n');
}

fn collect_decisions(messages: &[ChatMessage]) -> Vec<String> {
    let markers = decision_markers();
    messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .filter_map(|message| {
            let lowered = message.content.to_lowercase();
            if markers.iter().any(|marker| lowered.contains(marker)) {
                Some(truncate_ellipsis(&message.content, KEY_EXCHANGE_ITEM_CHARS))
            } else {
                None
            }
        })
        .collect()
}

fn append_decisions(summary: &mut String, messages: &[ChatMessage]) {
    summary.push_str("## Decisions\n");
    let decisions = collect_decisions(messages);
    if decisions.is_empty() {
        summary.push_str("- No explicit decisions captured.\n");
        return;
    }

    let mut recent_decisions = decisions
        .into_iter()
        .rev()
        .take(TOOL_ACTIVITY_LIMIT)
        .collect::<Vec<_>>();
    recent_decisions.reverse();
    for decision in recent_decisions {
        let _ = writeln!(summary, "- {decision}");
    }
}

#[must_use]
pub fn session_token_total(messages: &[ChatMessage]) -> usize {
    messages.iter().map(ChatMessage::estimated_tokens).sum()
}

#[must_use]
pub fn should_compact(messages: &[ChatMessage], config: &CompactionConfig) -> bool {
    if messages.is_empty() {
        return false;
    }

    let has_tracked_token_data = messages
        .iter()
        .any(|message| message.input_tokens.is_some() || message.output_tokens.is_some());

    if config.token_threshold == 0 {
        return false;
    }

    if !has_tracked_token_data {
        return false;
    }

    session_token_total(messages) > config.token_threshold
}

#[must_use]
pub fn microcompact_messages(
    messages: &[ChatMessage],
    config: &CompactionConfig,
) -> Vec<ChatMessage> {
    if !config.enable_microcompaction {
        return messages.to_vec();
    }

    messages
        .iter()
        .cloned()
        .map(|mut message| {
            if message.role != MessageRole::Assistant {
                return message;
            }
            // Byte length ≥ char count; skip the O(n) scan for short messages.
            if message.content.len() <= config.tool_output_prune_threshold {
                return message;
            }
            let content_len = message.content.chars().count();
            let looks_like_tool_output = contains_tool_output_pattern(&message.content)
                || content_len > config.tool_output_prune_threshold;
            if !looks_like_tool_output || content_len <= config.tool_output_prune_threshold {
                return message;
            }
            message.content = prune_tool_output(&message.content, config.hot_tail_chars);
            message
        })
        .collect()
}

#[must_use]
pub fn build_structured_summary(messages: &[ChatMessage], max_chars: usize) -> String {
    let mut summary = String::new();
    let message_count = messages.len();
    let _ = write!(
        summary,
        "[Session compacted: {message_count} messages → structured summary]\n\n"
    );

    summary.push_str("## Active context\n");
    let _ = writeln!(summary, "- {}\n", active_context_line(messages));
    append_key_exchanges(&mut summary, messages);
    append_tool_activity(&mut summary, messages);
    append_decisions(&mut summary, messages);

    truncate_ellipsis(&summary, max_chars)
}

#[must_use]
pub fn build_rehydration_context(summary: &str, recent_messages: &[ChatMessage]) -> Option<String> {
    if recent_messages.is_empty() {
        return None;
    }
    let summary_chars = summary.chars().count();
    let recent_count = recent_messages.len();
    Some(format!(
        "[Compaction rehydration]\n- Previous summary context available ({summary_chars} chars)\n- Last {recent_count} messages preserved for continuity\n- Continue from where we left off."
    ))
}

fn companion_state_generation(content: &str) -> Option<u32> {
    if !content.starts_with(COMPANION_STATE_HEADER) {
        return None;
    }
    content.lines().find_map(|line| {
        line.strip_prefix(COMPANION_GENERATION_PREFIX)
            .and_then(|value| value.trim().parse::<u32>().ok())
    })
}

fn next_companion_rehydration_generation(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .filter(|message| message.role == MessageRole::System)
        .filter_map(|message| companion_state_generation(&message.content))
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// # Errors
///
/// Returns an error if reading session messages, deleting old messages,
/// appending summary/rehydration messages, or updating session state fails.
pub async fn compact_session(
    store: &PostgresSessionStore,
    session_id: &SessionId,
    config: &CompactionConfig,
) -> Result<CompactionResult> {
    compact_session_with_config(store, session_id, config, None).await
}

/// Like [`compact_session`] but accepts an optional companion state snapshot
/// that is injected as an additional rehydration message after compaction.
///
/// # Errors
///
/// Returns an error if reading session messages, deleting old messages,
/// appending summary/rehydration messages, or updating session state fails.
pub(crate) async fn compact_session_with_config(
    store: &PostgresSessionStore,
    session_id: &SessionId,
    config: &CompactionConfig,
    companion_snapshot: Option<&CompanionStateSnapshot>,
) -> Result<CompactionResult> {
    let messages = store.get_messages(session_id, None).await?;
    if messages.is_empty() {
        return Ok(CompactionResult::skipped());
    }
    let companion_generation = next_companion_rehydration_generation(&messages);
    let tokens_before = session_token_total(&messages);

    if !should_compact(&messages, config) {
        return Ok(CompactionResult::skipped());
    }

    let microcompacted = microcompact_messages(&messages, config);
    let tool_outputs_pruned = messages
        .iter()
        .zip(microcompacted.iter())
        .filter(|(original, compacted)| original.content != compacted.content)
        .count();

    let keep_fraction = config.keep_fraction.clamp(0.0, 1.0);
    // Cast safety: fraction of message count, always fits in usize
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let mut keep_count = ((microcompacted.len() as f64) * keep_fraction).round() as usize;
    if keep_count >= microcompacted.len() {
        keep_count = microcompacted.len().saturating_sub(1);
    }
    let split_index = microcompacted.len().saturating_sub(keep_count);
    if split_index == 0 {
        return Ok(CompactionResult::skipped());
    }

    let to_summarize = &microcompacted[..split_index];
    let to_keep = &microcompacted[split_index..];
    if to_summarize.is_empty() {
        return Ok(CompactionResult::skipped());
    }

    let summary = build_structured_summary(to_summarize, config.summary_max_chars);

    let audit = super::compaction_audit::audit_compaction_output(&summary);
    if !audit.passed {
        for flag in &audit.flags {
            tracing::warn!(
                kind = ?flag.kind,
                detail = %flag.detail,
                "compaction audit flag"
            );
        }
    }

    let mut compaction_messages = vec![summary.clone()];
    if config.enable_rehydration
        && let Some(rehydration) = build_rehydration_context(&summary, to_keep)
    {
        compaction_messages.push(rehydration);
    }

    if let Some(companion_block) =
        build_companion_rehydration_block(config, companion_snapshot, companion_generation)
    {
        compaction_messages.push(companion_block);
    }

    let cutoff = &microcompacted[split_index - 1];
    let messages_removed = store
        .compact_messages_before(session_id, cutoff.id.as_str(), &compaction_messages)
        .await?;

    let compacted_messages = store.get_messages(session_id, None).await?;
    let tokens_after = session_token_total(&compacted_messages);

    Ok(CompactionResult {
        compacted: true,
        messages_removed,
        tokens_before,
        tokens_after,
        tool_outputs_pruned,
    })
}

fn build_companion_rehydration_block(
    config: &CompactionConfig,
    companion_snapshot: Option<&super::compaction_context::CompanionStateSnapshot>,
    generation: u32,
) -> Option<String> {
    if config.enable_rehydration
        && let Some(snapshot) = companion_snapshot
    {
        let mut snapshot = snapshot.clone();
        snapshot.generation = generation;
        let companion_block = render_rehydration_block(&snapshot);
        if !companion_block.is_empty() {
            return Some(companion_block);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use tempfile::{NamedTempFile, TempDir};

    use super::{
        build_rehydration_context, build_structured_summary, compact_session,
        microcompact_messages, next_companion_rehydration_generation, should_compact,
    };
    use crate::contracts::ids::SessionId;
    use crate::core::sessions::store::PostgresSessionStore;
    use crate::core::sessions::types::{
        ChatMessage, CompactionConfig, MessageRole, SessionState, estimate_tokens,
    };
    async fn store() -> (
        TempDir,
        NamedTempFile,
        PostgresSessionStore,
        crate::utils::test_env::TestDbGuard,
    ) {
        let db_guard = crate::utils::test_env::acquire_test_db().await;
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let temp_dir = TempDir::new().expect("tempdir should be created");
        let workspace_dir = temp_dir.path().join("workspace");
        crate::utils::test_env::write_workspace_postgres_config(&workspace_dir, &database_url)
            .expect("test config should be written");
        let db_file = NamedTempFile::new_in(&workspace_dir).expect("session db file should exist");
        let store = PostgresSessionStore::connect(db_file.path())
            .await
            .expect("session store should be created");
        (temp_dir, db_file, store, db_guard)
    }

    fn message(id: &str, role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            id: crate::contracts::ids::MessageId::new(id),
            session_id: SessionId::new("s1"),
            role,
            content: content.to_string(),
            input_tokens: None,
            output_tokens: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn estimate_tokens_ascii() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        assert_eq!(estimate_tokens("abcdefghijkl"), 3);
    }

    #[test]
    fn estimate_tokens_cjk() {
        assert_eq!(estimate_tokens("進"), 1);
        assert_eq!(estimate_tokens("進め"), 1);
        assert_eq!(estimate_tokens("進めて"), 2);
    }

    #[test]
    fn should_compact_token_threshold() {
        let messages = vec![
            ChatMessage {
                input_tokens: Some(60),
                ..message("1", MessageRole::User, "hello")
            },
            ChatMessage {
                output_tokens: Some(55),
                ..message("2", MessageRole::Assistant, "world")
            },
        ];
        let config = CompactionConfig {
            token_threshold: 100,
            ..CompactionConfig::default()
        };
        assert!(should_compact(&messages, &config));
    }

    #[test]
    fn should_not_compact_without_token_telemetry_even_with_many_messages() {
        let messages = vec![
            message("1", MessageRole::User, "a"),
            message("2", MessageRole::Assistant, "b"),
            message("3", MessageRole::User, "c"),
            message("4", MessageRole::Assistant, "d"),
        ];
        let config = CompactionConfig {
            token_threshold: 0,
            ..CompactionConfig::default()
        };
        assert!(!should_compact(&messages, &config));
    }

    #[test]
    fn should_not_compact_without_token_telemetry() {
        let messages = vec![
            message("1", MessageRole::User, "a"),
            message("2", MessageRole::Assistant, "b"),
            message("3", MessageRole::User, "c"),
            message("4", MessageRole::Assistant, "d"),
            message("5", MessageRole::User, "e"),
        ];
        let config = CompactionConfig {
            token_threshold: 1,
            ..CompactionConfig::default()
        };

        assert!(!should_compact(&messages, &config));
    }

    #[test]
    fn microcompact_prunes_large_outputs() {
        let large = format!("Tool result:\n{}", "x".repeat(3_000));
        let messages = vec![message("1", MessageRole::Assistant, &large)];
        let config = CompactionConfig {
            tool_output_prune_threshold: 600,
            hot_tail_chars: 80,
            ..CompactionConfig::default()
        };

        let compacted = microcompact_messages(&messages, &config);
        assert_eq!(compacted.len(), 1);
        assert!(compacted[0].content.contains("chars pruned"));
        assert!(compacted[0].content.len() < large.len());
    }

    #[test]
    fn structured_summary_format() {
        let messages = vec![
            message(
                "1",
                MessageRole::User,
                "Goal: implement token-aware compaction",
            ),
            message(
                "2",
                MessageRole::Assistant,
                "Tool result: scanned store and types",
            ),
            message("3", MessageRole::User, "sounds good, go ahead"),
            message("4", MessageRole::Assistant, "Implemented draft."),
        ];

        let summary = build_structured_summary(&messages, 4_000);
        assert!(summary.contains("[Session compacted:"));
        assert!(summary.contains("## Active context"));
        assert!(summary.contains("## Key exchanges"));
        assert!(summary.contains("## Tool activity"));
        assert!(summary.contains("## Decisions"));
    }

    #[test]
    fn next_companion_rehydration_generation_increments_from_existing_blocks() {
        let messages = vec![
            message(
                "1",
                MessageRole::System,
                "## Companion State (restored after compaction)\n\nCompaction generation: 1\n",
            ),
            message("2", MessageRole::User, "new turn"),
            message(
                "3",
                MessageRole::System,
                "## Companion State (restored after compaction)\n\nCompaction generation: 4\n",
            ),
        ];

        assert_eq!(next_companion_rehydration_generation(&messages), 5);
    }

    #[test]
    fn next_companion_rehydration_generation_starts_at_one_without_prior_block() {
        let messages = vec![
            message("1", MessageRole::User, "hello"),
            message(
                "2",
                MessageRole::System,
                "[Compaction rehydration]\nnot companion state",
            ),
        ];

        assert_eq!(next_companion_rehydration_generation(&messages), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn compact_below_threshold_returns_false() {
        let (_temp_dir, _db_file, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u1").await.unwrap();
        store
            .append_message(&session.id, MessageRole::User, "hello", None, None)
            .await
            .unwrap();

        let config = CompactionConfig {
            token_threshold: 0,
            ..CompactionConfig::default()
        };
        let result = compact_session(&store, &session.id, &config).await.unwrap();
        assert!(!result.compacted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn compact_above_threshold_summarizes_and_returns_true() {
        let (_temp_dir, _db_file, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u1").await.unwrap();
        for index in 0..6 {
            let role = if index % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            };
            let (input_tokens, output_tokens) = if role == MessageRole::User {
                (Some(24), None)
            } else {
                (None, Some(24))
            };
            store
                .append_message(
                    &session.id,
                    role,
                    &format!("msg-{index}"),
                    input_tokens,
                    output_tokens,
                )
                .await
                .unwrap();
        }

        let config = CompactionConfig {
            token_threshold: 100,
            keep_fraction: 0.4,
            ..CompactionConfig::default()
        };
        let result = compact_session(&store, &session.id, &config).await.unwrap();
        assert!(result.compacted);

        let messages = store.get_messages(&session.id, None).await.unwrap();
        assert!(
            messages.iter().any(|m| {
                m.role == MessageRole::System && m.content.contains("Session compacted")
            })
        );

        let session_after = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(session_after.state, SessionState::Compacted);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn compact_session_with_token_config() {
        let (_temp_dir, _db_file, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u1").await.unwrap();
        for index in 0..8 {
            let role = if index % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            };
            store
                .append_message(
                    &session.id,
                    role,
                    &format!("Tool result: payload-{}-{}", index, "x".repeat(500)),
                    Some(70),
                    Some(70),
                )
                .await
                .unwrap();
        }

        let config = CompactionConfig {
            token_threshold: 300,
            keep_fraction: 0.25,
            tool_output_prune_threshold: 300,
            hot_tail_chars: 40,
            summary_max_chars: 2_000,
            enable_rehydration: true,
            ..CompactionConfig::default()
        };

        let result = compact_session(&store, &session.id, &config).await.unwrap();
        assert!(result.compacted);
        assert!(result.messages_removed > 0);
        assert!(result.tool_outputs_pruned > 0);
    }

    #[test]
    fn rehydration_context() {
        let summary = "summary";
        let recent = vec![
            message("1", MessageRole::Assistant, "a"),
            message("2", MessageRole::User, "b"),
        ];
        let context = build_rehydration_context(summary, &recent).unwrap();
        assert!(context.contains("[Compaction rehydration]"));
        assert!(context.contains("Last 2 messages preserved"));
    }
}
