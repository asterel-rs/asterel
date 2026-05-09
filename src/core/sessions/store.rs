//! PostgreSQL-backed session store: async-native CRUD for sessions
//! and chat messages.

mod mapping;
mod schema;
mod transcript;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;
use uuid::Uuid;

use super::types::{
    ChatMessage, ChatMessagePart, ChatMessagePartInput, MessagePartKind, MessageRole, Session,
    SessionMetadata, SessionState, TranscriptMessage,
};
use crate::contracts::ids::{MessageId, SessionId, UserId};

const SELECT_SESSION_BY_ID_SQL: &str =
    "SELECT id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at
     FROM sessions
     WHERE id = $1";
const SELECT_ACTIVE_SESSION_SQL: &str =
    "SELECT id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at
     FROM sessions
     WHERE surface = $1 AND owner_scope = $2 AND state = 'active'
     ORDER BY updated_at DESC
     LIMIT 1";
const SELECT_SESSIONS_BY_SURFACE_SQL: &str =
    "SELECT id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at
     FROM sessions
     WHERE surface = $1
     ORDER BY updated_at DESC";
const SELECT_ALL_SESSIONS_SQL: &str =
    "SELECT id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at
     FROM sessions
     ORDER BY updated_at DESC";
const SELECT_TENANT_SCOPED_SESSIONS_PAGE_SQL: &str =
    "SELECT id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at
     FROM sessions
     WHERE owner_scope = $1
        OR surface = $1
        OR owner_scope = $2
        OR surface = $2
        OR owner_scope LIKE $3
        OR surface LIKE $3
     ORDER BY updated_at DESC, id DESC
     LIMIT $4";
const SELECT_TENANT_SCOPED_SESSIONS_PAGE_AFTER_SQL: &str =
    "SELECT id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at
     FROM sessions
     WHERE (owner_scope = $1
        OR surface = $1
        OR owner_scope = $2
        OR surface = $2
        OR owner_scope LIKE $3
        OR surface LIKE $3)
       AND (updated_at < $4 OR (updated_at = $4 AND id < $5))
     ORDER BY updated_at DESC, id DESC
     LIMIT $6";
const SELECT_BOUND_SESSION_SQL: &str =
    "SELECT s.id, s.surface, s.owner_scope, s.state, s.model, s.metadata, s.created_at,
            s.updated_at, s.archived_at
     FROM session_bindings b
     JOIN sessions s ON s.id = b.session_id
     WHERE b.surface = $1 AND b.binding_key = $2 AND b.released_at IS NULL
     ORDER BY b.last_used_at DESC
     LIMIT 1";
const SELECT_MESSAGES_LIMITED_SQL: &str =
    "SELECT id, session_id, role, content, input_tokens, output_tokens, created_at
     FROM chat_messages
     WHERE session_id = $1
     ORDER BY created_at DESC
     LIMIT $2";
const SELECT_MESSAGES_ASC_SQL: &str =
    "SELECT id, session_id, role, content, input_tokens, output_tokens, created_at
     FROM chat_messages
     WHERE session_id = $1
     ORDER BY created_at ASC";
const SELECT_MESSAGES_PAGE_SQL: &str =
    "SELECT id, session_id, role, content, input_tokens, output_tokens, created_at
     FROM chat_messages
     WHERE session_id = $1
     ORDER BY created_at ASC, id ASC
     LIMIT $2";
const SELECT_MESSAGES_PAGE_AFTER_SQL: &str =
    "SELECT id, session_id, role, content, input_tokens, output_tokens, created_at
     FROM chat_messages
     WHERE session_id = $1
       AND (created_at > $2 OR (created_at = $2 AND id > $3))
     ORDER BY created_at ASC, id ASC
     LIMIT $4";
const SELECT_MESSAGE_PARTS_BY_MESSAGE_IDS_SQL: &str =
    "SELECT id, message_id, session_id, ordinal, kind, mime_type, content, metadata, created_at
     FROM chat_message_parts
     WHERE message_id = ANY($1)
     ORDER BY message_id ASC, ordinal ASC, created_at ASC";

/// Source-of-truth store for chat sessions.
/// PostgreSQL-backed session repository.
pub struct PostgresSessionStore {
    pool: Pool<Postgres>,
}

type PgTx<'a> = sqlx_core::transaction::Transaction<'a, Postgres>;

impl PostgresSessionStore {
    /// # Errors
    /// Returns an error if `PostgreSQL` cannot be opened or schema initialization fails.
    pub fn new(db_path: &Path) -> Result<Self> {
        let runtime = tokio::runtime::Runtime::new().context("create session store runtime")?;
        crate::utils::postgres::block_on_sync(&runtime, Self::connect(db_path))
    }

    /// # Errors
    /// Returns an error if `PostgreSQL` cannot be opened or schema initialization fails.
    pub async fn connect(db_path: &Path) -> Result<Self> {
        let workspace_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
        let database_url = crate::utils::postgres::require_postgres_url(
            None,
            Some(workspace_dir),
            "session store",
        )?;
        let pool = PoolOptions::<Postgres>::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .context("connect postgres for session store")?;
        Self::ensure_schema(&pool).await?;
        Ok(Self { pool })
    }

    async fn ensure_schema(pool: &Pool<Postgres>) -> Result<()> {
        schema::ensure_sessions_schema(pool).await?;
        schema::run_best_effort_schema_migrations(pool).await;
        schema::ensure_binding_schema(pool).await?;
        schema::ensure_messages_schema(pool).await?;
        schema::ensure_message_parts_schema(pool).await?;

        Ok(())
    }

    fn session_state_label(state: SessionState) -> &'static str {
        match state {
            SessionState::Active => "active",
            SessionState::Archived => "archived",
            SessionState::Compacted => "compacted",
        }
    }

    fn message_role_label(role: MessageRole) -> &'static str {
        match role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        }
    }

    fn message_part_kind_label(kind: MessagePartKind) -> &'static str {
        match kind {
            MessagePartKind::UserText => "user_text",
            MessagePartKind::AssistantText => "assistant_text",
            MessagePartKind::SystemText => "system_text",
            MessagePartKind::Reasoning => "reasoning",
            MessagePartKind::ToolCall => "tool_call",
            MessagePartKind::ToolResult => "tool_result",
            MessagePartKind::Patch => "patch",
            MessagePartKind::Compaction => "compaction",
            MessagePartKind::SubagentEvent => "subagent_event",
            MessagePartKind::RuntimeMetadata => "runtime_metadata",
            MessagePartKind::LoopDetection => "loop_detection",
        }
    }

    /// # Errors
    /// Returns an error if the active session lookup fails.
    pub async fn get_active_session(
        &self,
        surface: &str,
        owner_scope: &str,
    ) -> Result<Option<Session>> {
        let row = query(SELECT_ACTIVE_SESSION_SQL)
            .bind(surface)
            .bind(owner_scope)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| mapping::map_session_row(&row)).transpose()
    }

    /// # Errors
    /// Returns an error if the binding lookup or timestamp refresh fails.
    pub async fn resolve_binding(
        &self,
        surface: &str,
        binding_key: &str,
    ) -> Result<Option<Session>> {
        let session = Self::load_bound_session_in_txless(&self.pool, surface, binding_key).await?;

        if session.is_some() {
            Self::touch_binding_in_txless(&self.pool, surface, binding_key).await?;
        }

        Ok(session)
    }

    /// # Errors
    /// Returns an error if session lookup fails.
    pub async fn get_session(&self, id: &SessionId) -> Result<Option<Session>> {
        let row = query(SELECT_SESSION_BY_ID_SQL)
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| mapping::map_session_row(&row)).transpose()
    }

    /// # Errors
    /// Returns an error if binding resolution, session creation, or transaction commit fails.
    pub async fn resolve_or_create_bound_session(
        &self,
        surface: &str,
        binding_key: &str,
        owner_scope: &str,
    ) -> Result<Session> {
        let mut tx = self.pool.begin().await?;

        if let Some(session) = Self::load_bound_session_in_tx(&mut tx, surface, binding_key).await?
        {
            Self::touch_binding_in_tx(&mut tx, surface, binding_key).await?;
            tx.commit().await?;
            return Ok(session);
        }

        let mut created_session_id = None;
        let session = if let Some(session) =
            Self::get_active_session_in_tx(&mut tx, surface, owner_scope).await?
        {
            session
        } else {
            let session = Self::create_session_in_tx(&mut tx, surface, owner_scope).await?;
            created_session_id = Some(session.id.clone());
            session
        };

        let timestamp = Utc::now().to_rfc3339();
        let inserted = query(
            "INSERT INTO session_bindings (surface, binding_key, session_id, released_at, created_at, last_used_at)
             VALUES ($1, $2, $3, NULL, $4, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(surface)
        .bind(binding_key)
        .bind(session.id.as_str())
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;

        let resolved = if inserted {
            session
        } else {
            if let Some(created_session_id) = created_session_id.as_ref() {
                Self::delete_session_if_unbound_in_tx(&mut tx, created_session_id).await?;
            }
            let resolved = Self::load_bound_session_in_tx(&mut tx, surface, binding_key)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("bound session disappeared during concurrent resolution")
                })?;
            Self::touch_binding_in_tx(&mut tx, surface, binding_key).await?;
            resolved
        };

        tx.commit().await?;
        Ok(resolved)
    }

    /// # Errors
    /// Returns an error if the session cannot be persisted to storage.
    pub async fn create_session(&self, surface: &str, owner_scope: &str) -> Result<Session> {
        let mut tx = self.pool.begin().await?;
        let session = Self::create_session_in_tx(&mut tx, surface, owner_scope).await?;
        tx.commit().await?;
        Ok(session)
    }

    /// # Errors
    /// Returns an error if lookup or session creation fails.
    pub async fn get_or_create_session(&self, surface: &str, owner_scope: &str) -> Result<Session> {
        if let Some(session) = self.get_active_session(surface, owner_scope).await? {
            return Ok(session);
        }

        self.create_session(surface, owner_scope).await
    }

    /// # Errors
    /// Returns an error if the binding record cannot be persisted.
    pub async fn create_binding(
        &self,
        surface: &str,
        binding_key: &str,
        session_id: &SessionId,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        query(
            "INSERT INTO session_bindings (surface, binding_key, session_id, released_at, created_at, last_used_at)
             VALUES ($1, $2, $3, NULL, $4, $4)",
        )
        .bind(surface)
        .bind(binding_key)
        .bind(session_id.as_str())
        .bind(&timestamp)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// # Errors
    /// Returns an error if the binding release update fails.
    pub async fn release_binding(&self, surface: &str, binding_key: &str) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        query(
            "UPDATE session_bindings
             SET released_at = $3, last_used_at = $3
             WHERE surface = $1 AND binding_key = $2 AND released_at IS NULL",
        )
        .bind(surface)
        .bind(binding_key)
        .bind(&timestamp)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// # Errors
    /// Returns an error if listing sessions fails.
    pub async fn list_sessions(&self, surface: Option<&str>) -> Result<Vec<Session>> {
        let rows = if let Some(surface) = surface {
            query(SELECT_SESSIONS_BY_SURFACE_SQL)
                .bind(surface)
                .fetch_all(&self.pool)
                .await?
        } else {
            query(SELECT_ALL_SESSIONS_SQL).fetch_all(&self.pool).await?
        };

        rows.into_iter()
            .map(|row| mapping::map_session_row(&row))
            .collect()
    }

    /// # Errors
    /// Returns an error if the paged session read fails.
    pub async fn list_tenant_scoped_sessions_page(
        &self,
        tenant_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Session>> {
        let tenant_id = tenant_id.trim();
        if tenant_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let tenant_scope = format!("tenant:{tenant_id}");
        let tenant_prefix = format!("tenant::{tenant_id}::%");
        let limit_i64 = i64::try_from(limit)
            .map_err(|error| anyhow::anyhow!("session page limit conversion failed: {error}"))?;

        let rows = if let Some((cursor_updated_at, cursor_id)) =
            self.load_session_page_cursor(cursor).await?
        {
            query(SELECT_TENANT_SCOPED_SESSIONS_PAGE_AFTER_SQL)
                .bind(tenant_id)
                .bind(&tenant_scope)
                .bind(&tenant_prefix)
                .bind(&cursor_updated_at)
                .bind(&cursor_id)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
        } else {
            query(SELECT_TENANT_SCOPED_SESSIONS_PAGE_SQL)
                .bind(tenant_id)
                .bind(&tenant_scope)
                .bind(&tenant_prefix)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter()
            .map(|row| mapping::map_session_row(&row))
            .collect()
    }

    /// # Errors
    /// Returns an error if deleting the session fails.
    pub async fn delete_session(&self, id: &SessionId) -> Result<bool> {
        let rows = query("DELETE FROM sessions WHERE id = $1")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(rows > 0)
    }

    /// # Errors
    /// Returns an error if updating session state fails.
    pub async fn update_session_state(&self, id: &SessionId, state: SessionState) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        let archived_at = if matches!(state, SessionState::Archived) {
            Some(timestamp.clone())
        } else {
            None
        };
        let changed = query(
            "UPDATE sessions
             SET state = $2, updated_at = $3, archived_at = $4
             WHERE id = $1",
        )
        .bind(id.as_str())
        .bind(Self::session_state_label(state))
        .bind(&timestamp)
        .bind(archived_at)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if changed == 0 {
            anyhow::bail!("session not found: {id}");
        }
        Ok(())
    }

    /// # Errors
    /// Returns an error if the message cannot be persisted.
    pub async fn append_message(
        &self,
        session_id: &SessionId,
        role: MessageRole,
        content: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<ChatMessage> {
        let parts = [ChatMessagePartInput::new(
            ChatMessage::default_part_kind_for_role(role),
            content,
        )];
        self.append_message_with_parts(session_id, role, &parts, input_tokens, output_tokens)
            .await
    }

    /// # Errors
    /// Returns an error if the message or any transcript part cannot be persisted.
    pub async fn append_message_with_parts(
        &self,
        session_id: &SessionId,
        role: MessageRole,
        parts: &[ChatMessagePartInput],
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<ChatMessage> {
        let message_id = MessageId::new(Uuid::new_v4().to_string());
        let timestamp = Utc::now().to_rfc3339();
        let input_tokens_i64 = input_tokens.map(transcript::u64_to_i64).transpose()?;
        let output_tokens_i64 = output_tokens.map(transcript::u64_to_i64).transpose()?;
        let content = transcript::flatten_message_parts(parts);

        let mut tx = self.pool.begin().await?;

        query(
            "INSERT INTO chat_messages (id, session_id, role, content, input_tokens, output_tokens, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(message_id.as_str())
        .bind(session_id.as_str())
        .bind(Self::message_role_label(role))
        .bind(&content)
        .bind(input_tokens_i64)
        .bind(output_tokens_i64)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;

        for (ordinal, part) in parts.iter().enumerate() {
            let part_metadata = part
                .metadata
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .context("serialize transcript part metadata")?;
            let ordinal_i32 = i32::try_from(ordinal).map_err(|error| {
                anyhow::anyhow!("transcript part ordinal conversion failed: {error}")
            })?;
            query(
                "INSERT INTO chat_message_parts (id, message_id, session_id, ordinal, kind, mime_type, content, metadata, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(message_id.as_str())
            .bind(session_id.as_str())
            .bind(ordinal_i32)
            .bind(Self::message_part_kind_label(part.kind))
            .bind(part.mime_type.as_deref())
            .bind(&part.content)
            .bind(part_metadata)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await?;
        }

        query("UPDATE sessions SET updated_at = $2 WHERE id = $1")
            .bind(session_id.as_str())
            .bind(&timestamp)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(ChatMessage {
            id: message_id,
            session_id: session_id.clone(),
            role,
            content,
            input_tokens,
            output_tokens,
            created_at: timestamp,
        })
    }

    /// # Errors
    /// Returns an error if transcript part retrieval fails.
    pub async fn get_message_parts(&self, message_id: &str) -> Result<Vec<ChatMessagePart>> {
        let message_ids = [MessageId::new(message_id)];
        let mut parts_by_message = self.get_message_parts_by_ids(&message_ids).await?;
        Ok(parts_by_message
            .remove(&MessageId::new(message_id))
            .unwrap_or_default())
    }

    async fn get_message_parts_by_ids(
        &self,
        message_ids: &[MessageId],
    ) -> Result<HashMap<MessageId, Vec<ChatMessagePart>>> {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut parts_by_message = HashMap::new();
        for row in query(SELECT_MESSAGE_PARTS_BY_MESSAGE_IDS_SQL)
            .bind(
                message_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
            .fetch_all(&self.pool)
            .await?
        {
            let part = mapping::map_chat_message_part_row(&row)?;
            parts_by_message
                .entry(part.message_id.clone())
                .or_insert_with(Vec::new)
                .push(part);
        }
        Ok(parts_by_message)
    }

    /// # Errors
    /// Returns an error if transcript retrieval fails.
    pub async fn get_transcript(
        &self,
        session_id: &SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<TranscriptMessage>> {
        let messages = self.get_messages(session_id, limit).await?;
        self.get_transcript_for_messages(messages).await
    }

    /// # Errors
    /// Returns an error if transcript retrieval fails.
    pub async fn get_transcript_tail_by_tokens(
        &self,
        session_id: &SessionId,
        max_tokens: usize,
    ) -> Result<Vec<TranscriptMessage>> {
        if max_tokens == 0 {
            return Ok(Vec::new());
        }

        let mut limit = 32usize;
        loop {
            let messages = self.get_messages(session_id, Some(limit)).await?;
            if messages.is_empty() {
                return Ok(Vec::new());
            }

            let selected = transcript::tail_messages_within_token_limit(&messages, max_tokens);
            if messages.len() < limit || selected.len() < messages.len() {
                return self.get_transcript_for_messages(selected).await;
            }

            let next_limit = limit.saturating_mul(2);
            if next_limit == limit {
                return self.get_transcript_for_messages(selected).await;
            }
            limit = next_limit;
        }
    }

    /// # Errors
    /// Returns an error if message retrieval fails.
    pub async fn get_messages(
        &self,
        session_id: &SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<ChatMessage>> {
        let mut messages = if let Some(limit) = limit {
            let limit_i64 = i64::try_from(limit)
                .map_err(|error| anyhow::anyhow!("message limit conversion failed: {error}"))?;
            let mut rows = query(SELECT_MESSAGES_LIMITED_SQL)
                .bind(session_id.as_str())
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| mapping::map_chat_message_row(&row))
                .collect::<Result<Vec<_>>>()?;
            rows.reverse();
            rows
        } else {
            query(SELECT_MESSAGES_ASC_SQL)
                .bind(session_id.as_str())
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| mapping::map_chat_message_row(&row))
                .collect::<Result<Vec<_>>>()?
        };

        if limit.is_some() {
            messages.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        }
        Ok(messages)
    }

    /// # Errors
    /// Returns an error if the paged message read fails.
    pub async fn get_messages_page(
        &self,
        session_id: &SessionId,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let limit_i64 = i64::try_from(limit)
            .map_err(|error| anyhow::anyhow!("message page limit conversion failed: {error}"))?;

        let rows = if let Some((cursor_created_at, cursor_id)) =
            self.load_message_page_cursor(session_id, cursor).await?
        {
            query(SELECT_MESSAGES_PAGE_AFTER_SQL)
                .bind(session_id.as_str())
                .bind(&cursor_created_at)
                .bind(&cursor_id)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
        } else {
            query(SELECT_MESSAGES_PAGE_SQL)
                .bind(session_id.as_str())
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter()
            .map(|row| mapping::map_chat_message_row(&row))
            .collect()
    }

    async fn get_transcript_for_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Vec<TranscriptMessage>> {
        let message_ids = messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        let parts_by_message = self.get_message_parts_by_ids(&message_ids).await?;
        Ok(transcript::assemble_transcript(messages, parts_by_message))
    }

    /// # Errors
    /// Returns an error if writing metadata to the backing store fails.
    pub async fn update_session_metadata(
        &self,
        id: &SessionId,
        metadata: Option<SessionMetadata>,
    ) -> Result<()> {
        let metadata_json = metadata
            .map(|value| serde_json::to_string(&value))
            .transpose()
            .context("serialize session metadata")?;
        let timestamp = Utc::now().to_rfc3339();

        let changed = query(
            "UPDATE sessions
             SET metadata = $2, updated_at = $3
             WHERE id = $1",
        )
        .bind(id.as_str())
        .bind(metadata_json)
        .bind(&timestamp)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if changed == 0 {
            anyhow::bail!("session not found: {id}");
        }
        Ok(())
    }

    /// # Errors
    /// Returns an error if message counting fails.
    pub async fn count_messages(&self, session_id: &SessionId) -> Result<usize> {
        let count: i64 = query("SELECT COUNT(*) AS count FROM chat_messages WHERE session_id = $1")
            .bind(session_id.as_str())
            .fetch_one(&self.pool)
            .await?
            .get("count");
        usize::try_from(count)
            .map_err(|error| anyhow::anyhow!("message count conversion failed: {error}"))
    }

    /// # Errors
    /// Returns an error if message deletion fails.
    pub async fn delete_messages_before(
        &self,
        session_id: &SessionId,
        before_id: &str,
    ) -> Result<usize> {
        let changed = query(
            "DELETE FROM chat_messages
             WHERE session_id = $1
               AND created_at < (
                   SELECT created_at
                   FROM chat_messages
                   WHERE id = $2 AND session_id = $1
               )",
        )
        .bind(session_id.as_str())
        .bind(before_id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        usize::try_from(changed)
            .map_err(|error| anyhow::anyhow!("deleted message count conversion failed: {error}"))
    }

    /// Atomically replace the compacted transcript prefix with compaction
    /// system messages and mark the session compacted.
    ///
    /// # Errors
    /// Returns an error if any delete, insert, or state update fails.
    pub(crate) async fn compact_messages_before(
        &self,
        session_id: &SessionId,
        before_id: &str,
        compaction_messages: &[String],
    ) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let changed = query(
            "DELETE FROM chat_messages
             WHERE session_id = $1
               AND created_at < (
                   SELECT created_at
                   FROM chat_messages
                   WHERE id = $2 AND session_id = $1
               )",
        )
        .bind(session_id.as_str())
        .bind(before_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        for content in compaction_messages {
            let message_id = MessageId::new(Uuid::new_v4().to_string());
            let part_id = Uuid::new_v4().to_string();
            let timestamp = Utc::now().to_rfc3339();
            query(
                "INSERT INTO chat_messages (id, session_id, role, content, input_tokens, output_tokens, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(message_id.as_str())
            .bind(session_id.as_str())
            .bind(Self::message_role_label(MessageRole::System))
            .bind(content)
            .bind(None::<i64>)
            .bind(None::<i64>)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await?;

            query(
                "INSERT INTO chat_message_parts (id, message_id, session_id, ordinal, kind, mime_type, content, metadata, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(part_id)
            .bind(message_id.as_str())
            .bind(session_id.as_str())
            .bind(0_i32)
            .bind(Self::message_part_kind_label(MessagePartKind::Compaction))
            .bind(None::<String>)
            .bind(content)
            .bind(None::<String>)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await?;
        }

        let timestamp = Utc::now().to_rfc3339();
        let updated = query(
            "UPDATE sessions
             SET state = $2, updated_at = $3, archived_at = $4
             WHERE id = $1",
        )
        .bind(session_id.as_str())
        .bind(Self::session_state_label(SessionState::Compacted))
        .bind(&timestamp)
        .bind(None::<String>)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if updated == 0 {
            anyhow::bail!("session not found: {session_id}");
        }

        tx.commit().await?;
        usize::try_from(changed)
            .map_err(|error| anyhow::anyhow!("deleted message count conversion failed: {error}"))
    }

    async fn load_bound_session_in_txless(
        pool: &Pool<Postgres>,
        surface: &str,
        binding_key: &str,
    ) -> Result<Option<Session>> {
        let row = query(SELECT_BOUND_SESSION_SQL)
            .bind(surface)
            .bind(binding_key)
            .fetch_optional(pool)
            .await?;
        row.map(|row| mapping::map_session_row(&row)).transpose()
    }

    async fn touch_binding_in_txless(
        pool: &Pool<Postgres>,
        surface: &str,
        binding_key: &str,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        query(
            "UPDATE session_bindings
             SET last_used_at = $3
             WHERE surface = $1 AND binding_key = $2 AND released_at IS NULL",
        )
        .bind(surface)
        .bind(binding_key)
        .bind(&timestamp)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn load_bound_session_in_tx(
        tx: &mut PgTx<'_>,
        surface: &str,
        binding_key: &str,
    ) -> Result<Option<Session>> {
        let row = query(SELECT_BOUND_SESSION_SQL)
            .bind(surface)
            .bind(binding_key)
            .fetch_optional(&mut **tx)
            .await?;
        row.map(|row| mapping::map_session_row(&row)).transpose()
    }

    async fn touch_binding_in_tx(
        tx: &mut PgTx<'_>,
        surface: &str,
        binding_key: &str,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        query(
            "UPDATE session_bindings
             SET last_used_at = $3
             WHERE surface = $1 AND binding_key = $2 AND released_at IS NULL",
        )
        .bind(surface)
        .bind(binding_key)
        .bind(&timestamp)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn get_active_session_in_tx(
        tx: &mut PgTx<'_>,
        surface: &str,
        owner_scope: &str,
    ) -> Result<Option<Session>> {
        let row = query(SELECT_ACTIVE_SESSION_SQL)
            .bind(surface)
            .bind(owner_scope)
            .fetch_optional(&mut **tx)
            .await?;
        row.map(|row| mapping::map_session_row(&row)).transpose()
    }

    async fn create_session_in_tx(
        tx: &mut PgTx<'_>,
        surface: &str,
        owner_scope: &str,
    ) -> Result<Session> {
        let session_id = SessionId::new(Uuid::new_v4().to_string());
        let timestamp = Utc::now().to_rfc3339();

        query(
            "INSERT INTO sessions (id, surface, owner_scope, state, model, metadata, created_at, updated_at, archived_at)
             VALUES ($1, $2, $3, $4, NULL, NULL, $5, $5, NULL)",
        )
        .bind(session_id.as_str())
        .bind(surface)
        .bind(owner_scope)
        .bind(Self::session_state_label(SessionState::Active))
        .bind(&timestamp)
        .execute(&mut **tx)
        .await?;

        Ok(Session {
            id: session_id,
            surface: surface.to_string(),
            owner_scope: UserId::new(owner_scope),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            archived_at: None,
        })
    }

    async fn delete_session_if_unbound_in_tx(
        tx: &mut PgTx<'_>,
        session_id: &SessionId,
    ) -> Result<()> {
        query(
            "DELETE FROM sessions s
             WHERE s.id = $1
               AND NOT EXISTS (
                   SELECT 1
                   FROM session_bindings b
                   WHERE b.session_id = s.id AND b.released_at IS NULL
               )",
        )
        .bind(session_id.as_str())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn load_session_page_cursor(
        &self,
        cursor: Option<&str>,
    ) -> Result<Option<(String, String)>> {
        let Some(cursor) = cursor.map(str::trim).filter(|cursor| !cursor.is_empty()) else {
            return Ok(None);
        };

        let row = query("SELECT updated_at, id FROM sessions WHERE id = $1")
            .bind(cursor)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| {
            (
                row.get::<String, _>("updated_at"),
                row.get::<String, _>("id"),
            )
        }))
    }

    async fn load_message_page_cursor(
        &self,
        session_id: &SessionId,
        cursor: Option<&str>,
    ) -> Result<Option<(String, String)>> {
        let Some(cursor) = cursor.map(str::trim).filter(|cursor| !cursor.is_empty()) else {
            return Ok(None);
        };

        let row =
            query("SELECT created_at, id FROM chat_messages WHERE session_id = $1 AND id = $2")
                .bind(session_id.as_str())
                .bind(cursor)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|row| {
            (
                row.get::<String, _>("created_at"),
                row.get::<String, _>("id"),
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::PostgresSessionStore;
    use crate::contracts::ids::UserId;
    use crate::core::sessions::types::{
        ChatMessagePartInput, MessagePartKind, MessageRole, SessionState,
    };
    async fn store() -> (
        TempDir,
        PostgresSessionStore,
        crate::utils::test_env::TestDbGuard,
    ) {
        let db_guard = crate::utils::test_env::acquire_test_db().await;
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let tmp = TempDir::new().expect("tempdir should be created");
        let workspace_dir = tmp.path().join("workspace");
        crate::utils::test_env::write_workspace_postgres_config(&workspace_dir, &database_url)
            .expect("test config should be written");
        let store = PostgresSessionStore::connect(&workspace_dir.join("sessions.db"))
            .await
            .expect("session store should be created");
        (tmp, store, db_guard)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn create_and_fetch_round_trip() {
        let (_tmp, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u1").await.unwrap();
        let fetched = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(fetched.owner_scope, UserId::new("u1"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn append_and_count_messages() {
        let (_tmp, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u2").await.unwrap();

        store
            .append_message(&session.id, MessageRole::User, "hello", None, None)
            .await
            .unwrap();
        store
            .append_message(
                &session.id,
                MessageRole::Assistant,
                "world",
                Some(2),
                Some(3),
            )
            .await
            .unwrap();

        let count = store.count_messages(&session.id).await.unwrap();
        assert!(count >= 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn archive_state_update_works() {
        let (_tmp, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u3").await.unwrap();
        store
            .update_session_state(&session.id, SessionState::Archived)
            .await
            .unwrap();
        let fetched = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(fetched.state, SessionState::Archived);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn append_message_with_parts_round_trips_transcript_parts() {
        let (_tmp, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u4").await.unwrap();

        store
            .append_message_with_parts(
                &session.id,
                MessageRole::Assistant,
                &[
                    ChatMessagePartInput::new(MessagePartKind::AssistantText, "answer"),
                    ChatMessagePartInput::new(MessagePartKind::ToolResult, "tool result payload"),
                ],
                None,
                Some(12),
            )
            .await
            .unwrap();

        let transcript = store.get_transcript(&session.id, None).await.unwrap();
        assert_eq!(transcript.len(), 1);
        assert_eq!(transcript[0].parts.len(), 2);
        assert_eq!(transcript[0].parts[0].kind, MessagePartKind::AssistantText);
        assert_eq!(transcript[0].parts[1].kind, MessagePartKind::ToolResult);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn get_or_create_session_reuses_active_session() {
        let (_tmp, store, _db_guard) = store().await;

        let first = store.get_or_create_session("cli", "u-async").await.unwrap();
        let second = store.get_or_create_session("cli", "u-async").await.unwrap();

        assert_eq!(first.id, second.id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn append_message_with_parts_round_trips_reasoning_parts() {
        let (_tmp, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u-async-parts").await.unwrap();

        store
            .append_message_with_parts(
                &session.id,
                MessageRole::Assistant,
                &[
                    ChatMessagePartInput::new(MessagePartKind::AssistantText, "answer"),
                    ChatMessagePartInput::new(MessagePartKind::Reasoning, "trace"),
                ],
                None,
                Some(9),
            )
            .await
            .unwrap();

        let transcript = store.get_transcript(&session.id, None).await.unwrap();
        assert_eq!(transcript.len(), 1);
        assert_eq!(transcript[0].parts[0].kind, MessagePartKind::AssistantText);
        assert_eq!(transcript[0].parts[1].kind, MessagePartKind::Reasoning);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn get_transcript_tail_by_tokens_keeps_newest_budgeted_messages_with_parts() {
        let (_tmp, store, _db_guard) = store().await;
        let session = store.create_session("cli", "u-tail-budget").await.unwrap();

        store
            .append_message_with_parts(
                &session.id,
                MessageRole::User,
                &[ChatMessagePartInput::new(
                    MessagePartKind::UserText,
                    "older question",
                )],
                Some(80),
                None,
            )
            .await
            .unwrap();
        store
            .append_message_with_parts(
                &session.id,
                MessageRole::Assistant,
                &[ChatMessagePartInput::new(
                    MessagePartKind::AssistantText,
                    "older answer",
                )],
                None,
                Some(70),
            )
            .await
            .unwrap();
        store
            .append_message_with_parts(
                &session.id,
                MessageRole::User,
                &[ChatMessagePartInput::new(
                    MessagePartKind::UserText,
                    "recent question",
                )],
                Some(10),
                None,
            )
            .await
            .unwrap();
        store
            .append_message_with_parts(
                &session.id,
                MessageRole::Assistant,
                &[
                    ChatMessagePartInput::new(MessagePartKind::AssistantText, "recent answer"),
                    ChatMessagePartInput::new(MessagePartKind::Reasoning, "recent trace"),
                ],
                None,
                Some(12),
            )
            .await
            .unwrap();

        let transcript = store
            .get_transcript_tail_by_tokens(&session.id, 24)
            .await
            .unwrap();

        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].message.content, "recent question");
        assert_eq!(transcript[1].parts.len(), 2);
        assert_eq!(transcript[1].parts[0].content, "recent answer");
        assert_eq!(transcript[1].parts[1].kind, MessagePartKind::Reasoning);
    }
}
