use anyhow::{Context, Result};
use sqlx_core::row::Row;

use super::transcript::i64_to_u64;
use crate::contracts::ids::{MessageId, SessionId, UserId};
use crate::core::sessions::types::{
    ChatMessage, ChatMessagePart, MessagePartKind, MessageRole, Session, SessionMetadata,
    SessionState,
};

fn parse_session_state(value: &str) -> Result<SessionState> {
    match value {
        "active" => Ok(SessionState::Active),
        "archived" => Ok(SessionState::Archived),
        "compacted" => Ok(SessionState::Compacted),
        _ => Err(anyhow::anyhow!("unknown session state: {value}")),
    }
}

fn parse_message_role(value: &str) -> Result<MessageRole> {
    match value {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        "system" => Ok(MessageRole::System),
        _ => Err(anyhow::anyhow!("unknown message role: {value}")),
    }
}

fn parse_message_part_kind(value: &str) -> Result<MessagePartKind> {
    match value {
        "user_text" => Ok(MessagePartKind::UserText),
        "assistant_text" => Ok(MessagePartKind::AssistantText),
        "system_text" => Ok(MessagePartKind::SystemText),
        "reasoning" => Ok(MessagePartKind::Reasoning),
        "tool_call" => Ok(MessagePartKind::ToolCall),
        "tool_result" => Ok(MessagePartKind::ToolResult),
        "patch" => Ok(MessagePartKind::Patch),
        "compaction" => Ok(MessagePartKind::Compaction),
        "subagent_event" => Ok(MessagePartKind::SubagentEvent),
        "runtime_metadata" => Ok(MessagePartKind::RuntimeMetadata),
        "loop_detection" => Ok(MessagePartKind::LoopDetection),
        _ => Err(anyhow::anyhow!("unknown message part kind: {value}")),
    }
}

pub(super) fn map_session_row(row: &sqlx_postgres::PgRow) -> Result<Session> {
    let state_raw: String = row.get("state");
    let metadata_raw: Option<String> = row.get("metadata");
    let metadata = metadata_raw
        .map(|value| serde_json::from_str::<SessionMetadata>(&value))
        .transpose()
        .context("parse session metadata")?;

    Ok(Session {
        id: SessionId::from(row.get::<String, _>("id")),
        surface: row.get("surface"),
        owner_scope: UserId::from(row.get::<String, _>("owner_scope")),
        state: parse_session_state(&state_raw)?,
        model: row.get("model"),
        metadata,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        archived_at: row.get("archived_at"),
    })
}

pub(super) fn map_chat_message_row(row: &sqlx_postgres::PgRow) -> Result<ChatMessage> {
    let role_raw: String = row.get("role");
    Ok(ChatMessage {
        id: MessageId::from(row.get::<String, _>("id")),
        session_id: SessionId::from(row.get::<String, _>("session_id")),
        role: parse_message_role(&role_raw)?,
        content: row.get("content"),
        input_tokens: row
            .get::<Option<i64>, _>("input_tokens")
            .map(i64_to_u64)
            .transpose()?,
        output_tokens: row
            .get::<Option<i64>, _>("output_tokens")
            .map(i64_to_u64)
            .transpose()?,
        created_at: row.get("created_at"),
    })
}

pub(super) fn map_chat_message_part_row(row: &sqlx_postgres::PgRow) -> Result<ChatMessagePart> {
    let kind_raw: String = row.get("kind");
    let metadata_raw: Option<String> = row.get("metadata");
    let metadata = metadata_raw
        .map(|value| serde_json::from_str::<serde_json::Value>(&value))
        .transpose()
        .context("parse transcript part metadata")?;
    let ordinal: i32 = row.get("ordinal");

    Ok(ChatMessagePart {
        id: row.get("id"),
        message_id: MessageId::from(row.get::<String, _>("message_id")),
        session_id: SessionId::from(row.get::<String, _>("session_id")),
        ordinal: usize::try_from(ordinal)
            .map_err(|error| anyhow::anyhow!("part ordinal conversion failed: {error}"))?,
        kind: parse_message_part_kind(&kind_raw)?,
        mime_type: row.get("mime_type"),
        content: row.get("content"),
        metadata,
        created_at: row.get("created_at"),
    })
}
