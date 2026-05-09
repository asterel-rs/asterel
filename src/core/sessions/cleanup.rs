use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{MemoryBackend, MemoryConfig};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCleanupReport {
    pub archived_count: u64,
    pub purged_sessions: u64,
    pub purged_messages: u64,
}

/// # Errors
///
/// Returns an error when session cleanup database operations fail.
pub fn reap_stale_sessions(
    workspace_dir: &Path,
    config: &MemoryConfig,
) -> Result<SessionCleanupReport> {
    if config.backend != MemoryBackend::Postgres {
        return Ok(SessionCleanupReport::default());
    }

    if config.archive_after_days == 0 && config.purge_after_days == 0 {
        return Ok(SessionCleanupReport::default());
    }

    #[cfg(feature = "postgres")]
    {
        use sqlx_core::query::query;

        let pool = open_pool(workspace_dir, config)?;

        block_on_pg_result(async {
            let mut tx = pool
                .begin()
                .await
                .context("begin session cleanup transaction")?;

            let mut report = SessionCleanupReport::default();

            if config.archive_after_days > 0 {
                report.archived_count = query(archive_active_sessions_sql())
                    .bind(i32::try_from(config.archive_after_days).unwrap_or(i32::MAX))
                    .execute(&mut *tx)
                    .await
                    .context("archive stale active sessions")?
                    .rows_affected();
            }

            if config.purge_after_days > 0 {
                let purge_days = i32::try_from(config.purge_after_days).unwrap_or(i32::MAX);

                query(delete_stale_chat_message_parts_sql())
                    .bind(purge_days)
                    .execute(&mut *tx)
                    .await
                    .context("delete chat message parts for stale sessions")?;

                report.purged_messages = query(delete_stale_chat_messages_sql())
                    .bind(purge_days)
                    .execute(&mut *tx)
                    .await
                    .context("delete chat messages for stale sessions")?
                    .rows_affected();

                query(delete_stale_session_bindings_sql())
                    .bind(purge_days)
                    .execute(&mut *tx)
                    .await
                    .context("delete bindings for stale sessions")?;

                report.purged_sessions = query(delete_stale_sessions_sql())
                    .bind(purge_days)
                    .execute(&mut *tx)
                    .await
                    .context("delete stale archived and compacted sessions")?
                    .rows_affected();
            }

            tx.commit()
                .await
                .context("commit session cleanup transaction")?;

            Ok(report)
        })
    }

    #[cfg(not(feature = "postgres"))]
    {
        let _ = workspace_dir;
        let _ = config;
        Ok(SessionCleanupReport::default())
    }
}

const fn archive_active_sessions_sql() -> &'static str {
    "UPDATE sessions
     SET state = 'archived', updated_at = now()
     WHERE state = 'active'
       AND updated_at::timestamptz < now() - make_interval(days => $1)"
}

const fn delete_stale_chat_message_parts_sql() -> &'static str {
    "DELETE FROM chat_message_parts
     WHERE message_id IN (
         SELECT id
         FROM chat_messages
         WHERE session_id IN (
             SELECT id
             FROM sessions
             WHERE state IN ('archived', 'compacted')
               AND updated_at::timestamptz < now() - make_interval(days => $1)
         )
     )"
}

const fn delete_stale_chat_messages_sql() -> &'static str {
    "DELETE FROM chat_messages
     WHERE session_id IN (
         SELECT id
         FROM sessions
         WHERE state IN ('archived', 'compacted')
           AND updated_at::timestamptz < now() - make_interval(days => $1)
     )"
}

const fn delete_stale_session_bindings_sql() -> &'static str {
    "DELETE FROM session_bindings
     WHERE session_id IN (
         SELECT id
         FROM sessions
         WHERE state IN ('archived', 'compacted')
           AND updated_at::timestamptz < now() - make_interval(days => $1)
     )"
}

const fn delete_stale_sessions_sql() -> &'static str {
    "DELETE FROM sessions
     WHERE state IN ('archived', 'compacted')
       AND updated_at::timestamptz < now() - make_interval(days => $1)"
}

#[cfg(feature = "postgres")]
fn open_pool(
    workspace_dir: &Path,
    config: &MemoryConfig,
) -> Result<sqlx_core::pool::Pool<sqlx_postgres::Postgres>> {
    use sqlx_core::pool::PoolOptions;

    let database_url = crate::utils::postgres::require_postgres_url(
        config.postgres_url.as_deref(),
        Some(workspace_dir),
        "session cleanup",
    )?;

    block_on_pg_result(async {
        PoolOptions::<sqlx_postgres::Postgres>::new()
            .max_connections(config.pg_max_connections.max(1))
            .connect(&database_url)
            .await
            .context("connect postgres for session cleanup")
    })
}

#[cfg(feature = "postgres")]
fn block_on_pg_result<T, F>(future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            anyhow::bail!(
                "session cleanup requires multi-thread tokio runtime; skipping in current-thread runtime"
            );
        }
    } else {
        let runtime = tokio::runtime::Runtime::new().context("create session cleanup runtime")?;
        runtime.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SessionCleanupReport, archive_active_sessions_sql, delete_stale_chat_message_parts_sql,
        delete_stale_chat_messages_sql, delete_stale_session_bindings_sql,
        delete_stale_sessions_sql, reap_stale_sessions,
    };
    use crate::config::{MemoryBackend, MemoryConfig};
    use std::path::Path;

    #[test]
    fn cleanup_short_circuits_without_postgres_backend() {
        let config = MemoryConfig {
            backend: MemoryBackend::None,
            ..MemoryConfig::default()
        };

        let report = reap_stale_sessions(Path::new("."), &config)
            .expect("non-postgres cleanup should return default report");

        assert_eq!(report, SessionCleanupReport::default());
    }

    #[test]
    fn sql_targets_only_non_active_sessions_for_purge() {
        let archive_sql = archive_active_sessions_sql();
        assert!(archive_sql.contains("state = 'active'"));
        assert!(archive_sql.contains("SET state = 'archived'"));
        assert!(archive_sql.contains("updated_at::timestamptz"));

        let purge_sessions_sql = delete_stale_sessions_sql();
        assert!(purge_sessions_sql.contains("state IN ('archived', 'compacted')"));
        assert!(purge_sessions_sql.contains("updated_at::timestamptz"));

        let purge_messages_sql = delete_stale_chat_messages_sql();
        assert!(purge_messages_sql.contains("FROM chat_messages"));
        assert!(purge_messages_sql.contains("FROM sessions"));
        assert!(purge_messages_sql.contains("state IN ('archived', 'compacted')"));
        assert!(purge_messages_sql.contains("updated_at::timestamptz"));

        let purge_parts_sql = delete_stale_chat_message_parts_sql();
        assert!(purge_parts_sql.contains("FROM chat_message_parts"));
        assert!(purge_parts_sql.contains("FROM chat_messages"));
        assert!(purge_parts_sql.contains("FROM sessions"));
        assert!(purge_parts_sql.contains("updated_at::timestamptz"));

        let purge_bindings_sql = delete_stale_session_bindings_sql();
        assert!(purge_bindings_sql.contains("FROM session_bindings"));
        assert!(purge_bindings_sql.contains("FROM sessions"));
        assert!(purge_bindings_sql.contains("state IN ('archived', 'compacted')"));
        assert!(purge_bindings_sql.contains("updated_at::timestamptz"));
    }
}
