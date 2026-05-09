//! PostgreSQL-backed cron job persistence and query layer.
//!
//! Manages the `cron_jobs` table: inserting, listing, removing,
//! rescheduling, and querying due jobs with circuit-breaker state.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;
use uuid::Uuid;

use super::expression::{next_run_for, parse_max_attempts};
use super::types::{AGENT_PENDING_CAP, CronJob, CronJobKind, CronJobMetadata, CronJobOrigin};
use super::validation::validate_main_runtime_cron_command;
use crate::config::Config;
use crate::security::scrub::sanitize_api_error;
use crate::utils::text::truncate_ellipsis;

const MAX_LAST_OUTPUT_CHARS: usize = 2_000;

pub(super) fn sanitize_cron_last_output(output: &str) -> String {
    truncate_ellipsis(&sanitize_api_error(output), MAX_LAST_OUTPUT_CHARS)
}

/// Adds a new cron job with default metadata.
///
/// # Errors
///
/// Returns an error if the expression is invalid or the database
/// write fails.
pub fn add_job(config: &Config, expression: &str, command: &str) -> Result<CronJob> {
    add_job_meta(config, expression, command, &CronJobMetadata::default())
}

/// Adds a new cron job with explicit metadata (kind, origin,
/// expiration, max attempts).
///
/// # Errors
///
/// Returns an error if the expression is invalid, the agent
/// queue cap is exceeded, or the database write fails.
pub fn add_job_meta(
    config: &Config,
    expression: &str,
    command: &str,
    metadata: &CronJobMetadata,
) -> Result<CronJob> {
    let now = Utc::now();
    validate_main_runtime_cron_command(command).map_err(anyhow::Error::new)?;
    let next_run = next_run_for(expression, now)?;
    let id = Uuid::new_v4().to_string();
    let max_attempts = metadata.max_attempts.max(1);

    with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            if metadata.origin.is_agent() {
                cleanup_expired_jobs(pool, now).await?;
                let pending = pending_agent_jobs(pool, now).await?;
                if pending >= AGENT_PENDING_CAP {
                    anyhow::bail!(
                        "agent-origin queue cap reached ({AGENT_PENDING_CAP} pending jobs)"
                    );
                }
            }

            query(
                "INSERT INTO cron_jobs (
                    id, enabled, expression, command, created_at, next_run, job_kind, origin,
                    expires_at, max_attempts, consecutive_failures, breaker_open_until
                 ) VALUES ($1, TRUE, $2, $3, $4, $5, $6, $7, $8, $9, 0, NULL)",
            )
            .bind(&id)
            .bind(expression)
            .bind(command)
            .bind(now)
            .bind(next_run)
            .bind(metadata.job_kind.as_db())
            .bind(metadata.origin.as_db())
            .bind(metadata.expires_at)
            .bind(i64::from(max_attempts))
            .execute(pool)
            .await
            .context("insert cron job")?;
            Ok::<(), anyhow::Error>(())
        })
    })?;

    Ok(CronJob {
        id,
        enabled: true,
        expression: expression.to_string(),
        command: command.to_string(),
        next_run,
        last_run: None,
        last_status: None,
        job_kind: metadata.job_kind,
        origin: metadata.origin,
        expires_at: metadata.expires_at,
        max_attempts,
        consecutive_failures: 0,
        breaker_open_until: None,
    })
}

/// Lists all cron jobs ordered by next run time.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub fn list_jobs(config: &Config) -> Result<Vec<CronJob>> {
    with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            let rows = query(
                "SELECT id, enabled, expression, command, next_run, last_run, last_status,
                        job_kind, origin, expires_at, max_attempts,
                        consecutive_failures, breaker_open_until
                 FROM cron_jobs
                 ORDER BY next_run ASC",
            )
            .fetch_all(pool)
            .await
            .context("list cron jobs from database")?;
            Ok(rows.into_iter().map(|row| map_cron_row(&row)).collect())
        })
    })
}

/// Updates an existing cron job's schedule, command, or enabled state.
///
/// # Errors
///
/// Returns an error if the job does not exist, the expression is invalid,
/// or the database update fails.
pub fn update_job(
    config: &Config,
    id: &str,
    expression: Option<&str>,
    command: Option<&str>,
    enabled: Option<bool>,
) -> Result<CronJob> {
    let existing = list_jobs(config)?
        .into_iter()
        .find(|job| job.id == id)
        .ok_or_else(|| anyhow::anyhow!("Cron job '{id}' not found"))?;

    let expression = expression.unwrap_or(existing.expression.as_str()).trim();
    let command = command.unwrap_or(existing.command.as_str()).trim();
    if expression.is_empty() || command.is_empty() {
        anyhow::bail!("expression and command must not be empty");
    }
    validate_main_runtime_cron_command(command).map_err(anyhow::Error::new)?;
    let next_run = next_run_for(expression, Utc::now())?;
    let enabled = enabled.unwrap_or(existing.enabled);

    let changed = with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            Ok::<u64, anyhow::Error>(
                query(
                    "UPDATE cron_jobs
                     SET enabled = $1,
                         expression = $2,
                         command = $3,
                         next_run = $4,
                         breaker_open_until = CASE WHEN $1 THEN breaker_open_until ELSE NULL END
                     WHERE id = $5",
                )
                .bind(enabled)
                .bind(expression)
                .bind(command)
                .bind(next_run)
                .bind(id)
                .execute(pool)
                .await
                .context("update cron job")?
                .rows_affected(),
            )
        })
    })?;

    if changed == 0 {
        anyhow::bail!("Cron job '{id}' not found");
    }

    Ok(CronJob {
        id: existing.id,
        enabled,
        expression: expression.to_string(),
        command: command.to_string(),
        next_run,
        last_run: existing.last_run,
        last_status: existing.last_status,
        job_kind: existing.job_kind,
        origin: existing.origin,
        expires_at: existing.expires_at,
        max_attempts: existing.max_attempts,
        consecutive_failures: existing.consecutive_failures,
        breaker_open_until: if enabled {
            existing.breaker_open_until
        } else {
            None
        },
    })
}

/// Removes a cron job by its identifier.
///
/// # Errors
///
/// Returns an error if the job is not found or the delete fails.
pub fn remove_job(config: &Config, id: &str) -> Result<()> {
    let changed = with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            Ok::<u64, anyhow::Error>(
                query("DELETE FROM cron_jobs WHERE id = $1")
                    .bind(id)
                    .execute(pool)
                    .await
                    .context("delete cron job")?
                    .rows_affected(),
            )
        })
    })?;

    if changed == 0 {
        anyhow::bail!("Cron job '{id}' not found");
    }

    Ok(())
}

/// Returns all jobs whose next run time is at or before `now`,
/// excluding expired and circuit-broken jobs.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub fn due_jobs(config: &Config, now: DateTime<Utc>) -> Result<Vec<CronJob>> {
    with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            cleanup_expired_jobs(pool, now).await?;

            let rows = query(
                "SELECT id, enabled, expression, command, next_run, last_run, last_status,
                        job_kind, origin, expires_at, max_attempts,
                        consecutive_failures, breaker_open_until
                 FROM cron_jobs
                 WHERE enabled = TRUE
                   AND next_run <= $1
                   AND (expires_at IS NULL OR expires_at > $1)
                   AND (breaker_open_until IS NULL OR breaker_open_until <= $1)
                 ORDER BY next_run ASC",
            )
            .bind(now)
            .fetch_all(pool)
            .await
            .context("list due cron jobs")?;

            Ok(rows.into_iter().map(|row| map_cron_row(&row)).collect())
        })
    })
}

/// Records a job execution result and computes the next run time.
///
/// # Errors
///
/// Returns an error if rescheduling or the database update fails.
pub fn reschedule_after_run(
    config: &Config,
    job: &CronJob,
    success: bool,
    output: &str,
) -> Result<()> {
    reschedule_after_run_with_breaker_state(
        config,
        job,
        success,
        output,
        job.consecutive_failures,
        job.breaker_open_until,
    )
}

/// Records execution result with explicit circuit-breaker state.
///
/// # Errors
///
/// Returns an error if rescheduling or the database update fails.
pub fn reschedule_after_run_with_breaker_state(
    config: &Config,
    job: &CronJob,
    success: bool,
    output: &str,
    consecutive_failures: u32,
    breaker_open_until: Option<DateTime<Utc>>,
) -> Result<()> {
    let now = Utc::now();
    let next_run = next_run_for(&job.expression, now)?;
    let status = if success { "ok" } else { "error" };
    let safe_output = sanitize_cron_last_output(output);

    let changed = with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            Ok::<u64, anyhow::Error>(
                query(
                    "UPDATE cron_jobs
                     SET next_run = $1,
                         last_run = $2,
                         last_status = $3,
                         last_output = $4,
                         consecutive_failures = $5,
                         breaker_open_until = $6
                     WHERE id = $7",
                )
                .bind(next_run)
                .bind(now)
                .bind(status)
                .bind(&safe_output)
                .bind(i64::from(consecutive_failures))
                .bind(breaker_open_until)
                .bind(&job.id)
                .execute(pool)
                .await
                .context("update cron run+breaker state")?
                .rows_affected(),
            )
        })
    })?;

    if changed == 0 {
        anyhow::bail!(
            "Cron job '{}' not found while recording scheduler run",
            job.id
        );
    }

    Ok(())
}

/// Updates only the circuit-breaker columns for a job.
///
/// # Errors
///
/// Returns an error if the database update fails.
pub fn update_job_breaker_state(
    config: &Config,
    job_id: &str,
    consecutive_failures: u32,
    breaker_open_until: Option<DateTime<Utc>>,
) -> Result<()> {
    with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            query(
                "UPDATE cron_jobs
                 SET consecutive_failures = $1,
                     breaker_open_until = $2
                 WHERE id = $3",
            )
            .bind(i64::from(consecutive_failures))
            .bind(breaker_open_until)
            .bind(job_id)
            .execute(pool)
            .await
            .context("update cron breaker state")?;
            Ok::<(), anyhow::Error>(())
        })
    })
}

/// Converts an `i64` to `u32`, logging and defaulting to 0 on
/// negative or overflowed values.
pub(super) fn parse_non_negative_u32(raw: i64) -> u32 {
    if let Ok(v) = u32::try_from(raw) {
        v
    } else {
        tracing::warn!(
            raw,
            "unexpected negative or overflowed i64 in cron DB; using 0"
        );
        0
    }
}

/// Count agent-origin cron job success/failure after at least one run.
pub fn aggregate_agent_job_stats(config: &Config) -> (u32, u32) {
    match with_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            let row = query(
                "SELECT
                    COALESCE(SUM(CASE WHEN last_status = 'ok' THEN 1 ELSE 0 END), 0) AS success,
                    COALESCE(SUM(CASE WHEN last_status = 'error' THEN 1 ELSE 0 END), 0) AS failure
                 FROM cron_jobs
                 WHERE origin = 'agent' AND last_status IS NOT NULL",
            )
            .fetch_one(pool)
            .await
            .context("aggregate agent job stats")?;

            let success: i64 = row.get("success");
            let failure: i64 = row.get("failure");
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok((success.max(0) as u32, failure.max(0) as u32))
        })
    }) {
        Ok(stats) => stats,
        Err(error) => {
            tracing::warn!(error = %error, "failed to aggregate agent job stats from cron DB");
            (0, 0)
        }
    }
}

async fn cleanup_expired_jobs(pool: &Pool<Postgres>, now: DateTime<Utc>) -> Result<()> {
    query("DELETE FROM cron_jobs WHERE expires_at IS NOT NULL AND expires_at <= $1")
        .bind(now)
        .execute(pool)
        .await
        .context("cleanup expired cron jobs")?;
    Ok(())
}

async fn pending_agent_jobs(pool: &Pool<Postgres>, now: DateTime<Utc>) -> Result<usize> {
    let count: i64 = query(
        "SELECT COUNT(*) AS count
         FROM cron_jobs
         WHERE origin = 'agent'
           AND (expires_at IS NULL OR expires_at > $1)",
    )
    .bind(now)
    .fetch_one(pool)
    .await?
    .get("count");
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

fn map_cron_row(row: &sqlx_postgres::PgRow) -> CronJob {
    let max_attempts_raw: i64 = row.get("max_attempts");
    let consecutive_failures_raw: i64 = row.get("consecutive_failures");
    CronJob {
        id: row.get("id"),
        enabled: row.get("enabled"),
        expression: row.get("expression"),
        command: row.get("command"),
        next_run: row.get("next_run"),
        last_run: row.get("last_run"),
        last_status: row.get("last_status"),
        job_kind: CronJobKind::from_db(&row.get::<String, _>("job_kind")),
        origin: CronJobOrigin::from_db(&row.get::<String, _>("origin")),
        expires_at: row.get("expires_at"),
        max_attempts: parse_max_attempts(max_attempts_raw),
        consecutive_failures: parse_non_negative_u32(consecutive_failures_raw),
        breaker_open_until: row.get("breaker_open_until"),
    }
}

/// Creates the `cron_jobs` table and index if they do not exist.
///
/// # Errors
///
/// Returns an error if the DDL execution fails.
pub(super) async fn ensure_schema(pool: &Pool<Postgres>) -> Result<()> {
    query(
        "CREATE TABLE IF NOT EXISTS cron_jobs (
            id TEXT PRIMARY KEY,
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            expression TEXT NOT NULL,
            command TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            next_run TIMESTAMPTZ NOT NULL,
            last_run TIMESTAMPTZ,
            last_status TEXT,
            last_output TEXT,
            job_kind TEXT NOT NULL DEFAULT 'user',
            origin TEXT NOT NULL DEFAULT 'user',
            expires_at TIMESTAMPTZ,
            max_attempts BIGINT NOT NULL DEFAULT 1,
            consecutive_failures BIGINT NOT NULL DEFAULT 0,
            breaker_open_until TIMESTAMPTZ
        )",
    )
    .execute(pool)
    .await
    .context("create cron_jobs table")?;

    for (column_name, column_ddl) in [
        ("enabled", "enabled BOOLEAN DEFAULT TRUE"),
        ("created_at", "created_at TIMESTAMPTZ"),
        ("next_run", "next_run TIMESTAMPTZ"),
        ("last_run", "last_run TIMESTAMPTZ"),
        ("last_status", "last_status TEXT"),
        ("last_output", "last_output TEXT"),
        ("job_kind", "job_kind TEXT DEFAULT 'user'"),
        ("origin", "origin TEXT DEFAULT 'user'"),
        ("expires_at", "expires_at TIMESTAMPTZ"),
        ("max_attempts", "max_attempts BIGINT DEFAULT 1"),
        (
            "consecutive_failures",
            "consecutive_failures BIGINT DEFAULT 0",
        ),
        ("breaker_open_until", "breaker_open_until TIMESTAMPTZ"),
    ] {
        query(&format!(
            "ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS {column_ddl}"
        ))
        .execute(pool)
        .await
        .with_context(|| format!("ensure cron_jobs.{column_name} column"))?;
    }

    backfill_missing_next_runs(pool).await?;

    for statement in [
        "UPDATE cron_jobs SET enabled = TRUE WHERE enabled IS NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN enabled SET DEFAULT TRUE",
        "ALTER TABLE cron_jobs ALTER COLUMN enabled SET NOT NULL",
        "UPDATE cron_jobs SET created_at = NOW() WHERE created_at IS NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN created_at SET NOT NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN next_run SET NOT NULL",
        "UPDATE cron_jobs SET job_kind = 'user' WHERE job_kind IS NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN job_kind SET DEFAULT 'user'",
        "ALTER TABLE cron_jobs ALTER COLUMN job_kind SET NOT NULL",
        "UPDATE cron_jobs SET origin = 'user' WHERE origin IS NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN origin SET DEFAULT 'user'",
        "ALTER TABLE cron_jobs ALTER COLUMN origin SET NOT NULL",
        "UPDATE cron_jobs SET max_attempts = 1 WHERE max_attempts IS NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN max_attempts SET DEFAULT 1",
        "ALTER TABLE cron_jobs ALTER COLUMN max_attempts SET NOT NULL",
        "UPDATE cron_jobs SET consecutive_failures = 0 WHERE consecutive_failures IS NULL",
        "ALTER TABLE cron_jobs ALTER COLUMN consecutive_failures SET DEFAULT 0",
        "ALTER TABLE cron_jobs ALTER COLUMN consecutive_failures SET NOT NULL",
    ] {
        query(statement)
            .execute(pool)
            .await
            .with_context(|| format!("repair cron_jobs schema with `{statement}`"))?;
    }

    query("CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run)")
        .execute(pool)
        .await
        .context("create idx_cron_jobs_next_run")?;

    Ok(())
}

async fn backfill_missing_next_runs(pool: &Pool<Postgres>) -> Result<()> {
    let rows = query("SELECT id, expression FROM cron_jobs WHERE next_run IS NULL")
        .fetch_all(pool)
        .await
        .context("find cron jobs missing next_run")?;
    let now = Utc::now();

    for row in rows {
        let id: String = row.get("id");
        let expression: String = row.get("expression");
        let (next_run, enabled) = match next_run_for(&expression, now) {
            Ok(next_run) => (next_run, true),
            Err(error) => {
                tracing::warn!(%id, %expression, %error, "failed to backfill cron next_run from expression; disabling legacy cron job");
                (now, false)
            }
        };
        query(
            "UPDATE cron_jobs
             SET next_run = $1, enabled = CASE WHEN $2 THEN enabled ELSE FALSE END
             WHERE id = $3 AND next_run IS NULL",
        )
        .bind(next_run)
        .bind(enabled)
        .bind(&id)
        .execute(pool)
        .await
        .with_context(|| format!("backfill cron_jobs.next_run for {id}"))?;
    }

    Ok(())
}

fn with_pool<T: Send>(
    config: &Config,
    f: impl FnOnce(&tokio::runtime::Runtime, &Pool<Postgres>) -> Result<T> + Send,
) -> Result<T> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| with_pool_inner(config, f));
        }

        return std::thread::scope(|scope| {
            let join = scope.spawn(move || with_pool_inner(config, f));
            join.join().unwrap_or_else(|panic_payload| {
                let message = if let Some(message) = panic_payload.downcast_ref::<&str>() {
                    *message
                } else if let Some(message) = panic_payload.downcast_ref::<String>() {
                    message.as_str()
                } else {
                    "unknown panic"
                };
                Err(anyhow::anyhow!(
                    "cron repository worker thread panicked: {message}"
                ))
            })
        });
    }
    with_pool_inner(config, f)
}

fn with_pool_inner<T: Send>(
    config: &Config,
    f: impl FnOnce(&tokio::runtime::Runtime, &Pool<Postgres>) -> Result<T> + Send,
) -> Result<T> {
    let database_url = crate::utils::postgres::require_postgres_url(
        config.memory.postgres_url.as_deref(),
        Some(&config.workspace_dir),
        "cron repository",
    )?;

    let runtime = tokio::runtime::Runtime::new().context("create cron repository runtime")?;
    let pool = crate::utils::postgres::block_on_sync(&runtime, async {
        PoolOptions::<Postgres>::new()
            .max_connections(config.memory.pg_max_connections.max(1))
            .connect(&database_url)
            .await
            .context("connect postgres for cron repository")
    })?;

    crate::utils::postgres::block_on_sync(&runtime, ensure_schema(&pool))?;
    f(&runtime, &pool)
}
