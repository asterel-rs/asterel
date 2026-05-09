use anyhow::{Context, Result};
use sqlx_core::pool::Pool;
use sqlx_core::query::query;
use sqlx_postgres::Postgres;

pub(super) async fn ensure_sessions_schema(pool: &Pool<Postgres>) -> Result<()> {
    query(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            surface TEXT NOT NULL,
            owner_scope TEXT NOT NULL DEFAULT 'default',
            state TEXT NOT NULL DEFAULT 'active',
            model TEXT,
            metadata TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            archived_at TEXT
        )",
    )
    .execute(pool)
    .await
    .context("create sessions table")?;

    Ok(())
}

pub(super) async fn run_best_effort_schema_migrations(pool: &Pool<Postgres>) {
    for statement in [
        "ALTER TABLE sessions RENAME COLUMN channel TO surface",
        "ALTER TABLE sessions RENAME COLUMN user_id TO owner_scope",
        "ALTER TABLE sessions ADD COLUMN IF NOT EXISTS archived_at TEXT",
        "DROP INDEX IF EXISTS sessions_channel_user_id_state_key",
        "ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_channel_user_id_state_key",
        "DROP INDEX IF EXISTS sessions_unique_active_per_channel_user",
    ] {
        let _ = query(statement).execute(pool).await;
    }
}

pub(super) async fn ensure_binding_schema(pool: &Pool<Postgres>) -> Result<()> {
    query(
        "CREATE TABLE IF NOT EXISTS session_bindings (
            surface TEXT NOT NULL,
            binding_key TEXT NOT NULL,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            released_at TEXT,
            created_at TEXT NOT NULL,
            last_used_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .context("create session_bindings table")?;

    query(
        "CREATE UNIQUE INDEX IF NOT EXISTS session_bindings_active
         ON session_bindings (surface, binding_key) WHERE released_at IS NULL",
    )
    .execute(pool)
    .await
    .context("create session_bindings active index")?;

    Ok(())
}

pub(super) async fn ensure_messages_schema(pool: &Pool<Postgres>) -> Result<()> {
    query(
        "CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            input_tokens BIGINT,
            output_tokens BIGINT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .context("create chat_messages table")?;

    query(
        "CREATE INDEX IF NOT EXISTS idx_chat_messages_session
         ON chat_messages(session_id, created_at)",
    )
    .execute(pool)
    .await
    .context("create idx_chat_messages_session")?;

    Ok(())
}

pub(super) async fn ensure_message_parts_schema(pool: &Pool<Postgres>) -> Result<()> {
    query(
        "CREATE TABLE IF NOT EXISTS chat_message_parts (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            kind TEXT NOT NULL,
            mime_type TEXT,
            content TEXT NOT NULL,
            metadata TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .context("create chat_message_parts table")?;

    query(
        "CREATE INDEX IF NOT EXISTS idx_chat_message_parts_message
         ON chat_message_parts(message_id, ordinal, created_at)",
    )
    .execute(pool)
    .await
    .context("create idx_chat_message_parts_message")?;

    Ok(())
}
