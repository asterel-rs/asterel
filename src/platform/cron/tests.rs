//! Tests for cron expression parsing, job repository CRUD, and
//! rescheduling logic.

use chrono::{Duration as ChronoDuration, Utc};
use tempfile::TempDir;

use super::*;
use crate::config::Config;

fn test_config(tmp: &TempDir) -> Config {
    let database_url = crate::utils::test_env::postgres_url()
        .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.memory.postgres_url = Some(database_url);
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

#[test]
fn sanitize_cron_last_output_redacts_and_truncates_persisted_output() {
    let raw = format!(
        "route=user-direct-shell\nstdout:\nsecret sk-cron-output-token {}",
        "x".repeat(3_000)
    );

    let sanitized = super::repository::sanitize_cron_last_output(&raw);

    assert!(!sanitized.contains("sk-cron-output-token"));
    assert!(sanitized.contains("[REDACTED]"));
    assert!(sanitized.chars().count() <= 2_003);
    assert!(sanitized.ends_with("..."));
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn add_job_accepts_five_field_expression() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/5 * * * *", "echo ok").unwrap();

    assert_eq!(job.expression, "*/5 * * * *");
    assert_eq!(job.command, "echo ok");
    assert_eq!(job.job_kind, CronJobKind::User);
    assert_eq!(job.origin, CronJobOrigin::User);
    assert_eq!(job.expires_at, None);
    assert_eq!(job.max_attempts, 1);
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn add_job_accepts_natural_language_expression() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "every 5 minutes", "echo natural").unwrap();

    assert_eq!(job.expression, "every 5 minutes");
    assert_eq!(job.command, "echo natural");
    assert!(job.next_run > Utc::now());
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn add_job_rejects_invalid_field_count() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let err = add_job(&config, "* * * *", "echo bad").unwrap_err();
    assert!(err.to_string().contains("expected 5, 6, or 7 fields"));
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn add_list_remove_roundtrip() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/10 * * * *", "echo roundtrip").unwrap();
    let listed = list_jobs(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, job.id);

    remove_job(&config, &job.id).unwrap();
    assert!(list_jobs(&config).unwrap().is_empty());
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn due_jobs_filters_by_timestamp() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let _job = add_job(&config, "* * * * *", "echo due").unwrap();

    let due_now = due_jobs(&config, Utc::now()).unwrap();
    assert!(due_now.is_empty(), "new job should not be due immediately");

    let far_future = Utc::now() + ChronoDuration::days(365);
    let due_future = due_jobs(&config, far_future).unwrap();
    assert_eq!(due_future.len(), 1, "job should be due in far future");
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn reschedule_after_run_persists_last_status_and_last_run() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/15 * * * *", "echo run").unwrap();
    reschedule_after_run(&config, &job, false, "failed output").unwrap();

    let listed = list_jobs(&config).unwrap();
    let stored = listed.iter().find(|j| j.id == job.id).unwrap();
    assert_eq!(stored.last_status.as_deref(), Some("error"));
    assert!(stored.last_run.is_some());
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn update_job_breaker_state_persists_values() {
    use chrono::DurationRound;
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "*/15 * * * *", "echo breaker").unwrap();
    // Postgres `timestamptz` stores microsecond precision; truncate the
    // source value so the round-trip comparison matches exactly.
    let open_until = (Utc::now() + ChronoDuration::minutes(5))
        .duration_trunc(ChronoDuration::microseconds(1))
        .expect("timestamp should truncate to microseconds");

    update_job_breaker_state(&config, &job.id, 2, Some(open_until)).unwrap();

    let listed = list_jobs(&config).unwrap();
    let stored = listed.iter().find(|j| j.id == job.id).unwrap();
    assert_eq!(stored.consecutive_failures, 2);
    assert_eq!(stored.breaker_open_until, Some(open_until));
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn due_jobs_excludes_jobs_with_open_breaker() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "* * * * *", "echo guarded").unwrap();
    let far_future = Utc::now() + ChronoDuration::days(365);
    let open_until = far_future + ChronoDuration::minutes(5);

    update_job_breaker_state(&config, &job.id, 0, Some(open_until)).unwrap();

    let due = due_jobs(&config, far_future).unwrap();
    assert!(
        due.iter().all(|scheduled| scheduled.id != job.id),
        "job should be excluded while breaker is open"
    );
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn due_jobs_includes_jobs_when_breaker_opens_until_now() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = add_job(&config, "* * * * *", "echo due-now").unwrap();
    let far_future = Utc::now() + ChronoDuration::days(365);

    update_job_breaker_state(&config, &job.id, 0, Some(far_future)).unwrap();

    let due = due_jobs(&config, far_future).unwrap();
    assert!(
        due.iter().any(|scheduled| scheduled.id == job.id),
        "job should be due when breaker_open_until equals the query timestamp"
    );
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn agent_pending_cap_enforced() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let meta = CronJobMetadata {
        origin: CronJobOrigin::Agent,
        job_kind: CronJobKind::Agent,
        ..CronJobMetadata::default()
    };

    for i in 0..types::AGENT_PENDING_CAP {
        add_job_meta(&config, "*/5 * * * *", &format!("echo {i}"), &meta)
            .unwrap_or_else(|e| panic!("job {i} should succeed: {e}"));
    }

    let err = add_job_meta(&config, "*/5 * * * *", "echo overflow", &meta).unwrap_err();
    assert!(
        err.to_string().contains("cap reached"),
        "expected cap error, got: {err}"
    );
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn agent_cap_ignores_user_origin_jobs() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    for i in 0..10 {
        add_job(&config, "*/5 * * * *", &format!("echo user-{i}")).unwrap();
    }

    let meta = CronJobMetadata {
        origin: CronJobOrigin::Agent,
        job_kind: CronJobKind::Agent,
        ..CronJobMetadata::default()
    };
    add_job_meta(&config, "*/5 * * * *", "echo agent-ok", &meta)
        .expect("agent job should succeed — user jobs don't count toward cap");
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn due_jobs_excludes_expired_jobs() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let past = Utc::now() - ChronoDuration::hours(1);
    let meta = CronJobMetadata {
        expires_at: Some(past),
        ..CronJobMetadata::default()
    };

    add_job_meta(&config, "* * * * *", "echo expired", &meta).unwrap();
    let far_future = Utc::now() + ChronoDuration::days(365);
    let due = due_jobs(&config, far_future).unwrap();
    assert!(
        due.is_empty(),
        "expired jobs must be cleaned up by due_jobs"
    );
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn aggregate_agent_job_stats_counts_successes_and_failures() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let meta = CronJobMetadata {
        origin: CronJobOrigin::Agent,
        job_kind: CronJobKind::Agent,
        ..CronJobMetadata::default()
    };

    let j1 = add_job_meta(&config, "*/5 * * * *", "echo ok", &meta).unwrap();
    let j2 = add_job_meta(&config, "*/5 * * * *", "echo fail", &meta).unwrap();
    let j3 = add_job_meta(&config, "*/5 * * * *", "echo ok2", &meta).unwrap();

    reschedule_after_run(&config, &j1, true, "").unwrap();
    reschedule_after_run(&config, &j2, false, "err").unwrap();
    reschedule_after_run(&config, &j3, true, "").unwrap();

    let (success, failure) = aggregate_agent_job_stats(&config);
    assert_eq!(success, 2, "expected 2 successful runs");
    assert_eq!(failure, 1, "expected 1 failed run");
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn aggregate_agent_job_stats_empty_db_returns_zeros() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let (success, failure) = aggregate_agent_job_stats(&config);
    assert_eq!((success, failure), (0, 0));
}

#[test]
fn parse_non_negative_u32_handles_edge_cases() {
    use super::repository::parse_non_negative_u32;
    assert_eq!(parse_non_negative_u32(0), 0);
    assert_eq!(parse_non_negative_u32(1), 1);
    assert_eq!(parse_non_negative_u32(i64::from(u32::MAX)), u32::MAX);
    assert_eq!(parse_non_negative_u32(-1), 0);
    assert_eq!(parse_non_negative_u32(i64::from(u32::MAX) + 1), 0);
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn ensure_schema_is_idempotent() {
    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    add_job(&config, "*/5 * * * *", "echo first").unwrap();
    // Second call implicitly re-enters with_connection; schema init is a no-op.
    let jobs = list_jobs(&config).unwrap();
    assert_eq!(jobs.len(), 1, "schema re-init must not drop data");
}

#[test]
#[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
fn ensure_schema_backfills_legacy_cron_job_columns() {
    use sqlx_core::pool::PoolOptions;
    use sqlx_core::query::query;
    use sqlx_core::row::Row;
    use sqlx_postgres::Postgres;

    let _db_guard = crate::utils::test_env::acquire_test_db_blocking();
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let database_url = config.memory.postgres_url.as_deref().unwrap().to_string();
    let runtime = tokio::runtime::Runtime::new().expect("cron schema test runtime");

    crate::utils::postgres::block_on_sync(&runtime, async {
        let pool = PoolOptions::<Postgres>::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect postgres");
        query("DROP TABLE IF EXISTS cron_jobs")
            .execute(&pool)
            .await
            .expect("drop cron_jobs");
        query(
            "CREATE TABLE cron_jobs (
                id TEXT PRIMARY KEY,
                expression TEXT NOT NULL,
                command TEXT NOT NULL,
                enabled BOOLEAN,
                job_kind TEXT,
                origin TEXT,
                max_attempts BIGINT,
                consecutive_failures BIGINT
             )",
        )
        .execute(&pool)
        .await
        .expect("create legacy cron_jobs");
        query(
            "INSERT INTO cron_jobs (id, expression, command)
             VALUES ('legacy-job', '0 0 1 1 *', 'echo legacy')",
        )
        .execute(&pool)
        .await
        .expect("insert legacy job");
        query(
            "INSERT INTO cron_jobs (id, expression, command)
             VALUES ('invalid-legacy-job', 'not a cron expression', 'echo invalid')",
        )
        .execute(&pool)
        .await
        .expect("insert invalid legacy job");

        super::repository::ensure_schema(&pool)
            .await
            .expect("ensure schema should backfill columns");

        let row = query(
            "SELECT enabled, created_at, next_run, job_kind, origin, expires_at, max_attempts,
                    consecutive_failures, breaker_open_until, last_output
             FROM cron_jobs
             WHERE id = 'legacy-job'",
        )
        .fetch_one(&pool)
        .await
        .expect("read backfilled legacy row");

        assert!(row.get::<bool, _>("enabled"));
        assert_eq!(row.get::<String, _>("job_kind"), "user");
        assert_eq!(row.get::<String, _>("origin"), "user");
        let _: chrono::DateTime<Utc> = row.get("created_at");
        let next_run: chrono::DateTime<Utc> = row.get("next_run");
        assert!(
            next_run > Utc::now(),
            "missing next_run should be recomputed from expression, not made immediately due"
        );
        assert!(
            row.get::<Option<chrono::DateTime<Utc>>, _>("expires_at")
                .is_none()
        );
        assert_eq!(row.get::<i64, _>("max_attempts"), 1);
        assert_eq!(row.get::<i64, _>("consecutive_failures"), 0);
        assert!(
            row.get::<Option<chrono::DateTime<Utc>>, _>("breaker_open_until")
                .is_none()
        );
        assert!(row.get::<Option<String>, _>("last_output").is_none());

        let invalid_row = query(
            "SELECT enabled, next_run
             FROM cron_jobs
             WHERE id = 'invalid-legacy-job'",
        )
        .fetch_one(&pool)
        .await
        .expect("read invalid legacy row");
        assert!(
            !invalid_row.get::<bool, _>("enabled"),
            "invalid legacy cron expressions should be disabled rather than made immediately due"
        );
        let _: chrono::DateTime<Utc> = invalid_row.get("next_run");

        let constraint_rows = query(
            "SELECT column_name, is_nullable, column_default
             FROM information_schema.columns
             WHERE table_schema = 'public'
               AND table_name = 'cron_jobs'
               AND column_name = ANY($1)
             ORDER BY column_name",
        )
        .bind(
            &[
                "created_at",
                "enabled",
                "job_kind",
                "origin",
                "max_attempts",
                "consecutive_failures",
                "next_run",
            ][..],
        )
        .fetch_all(&pool)
        .await
        .expect("read cron schema constraints");

        assert_eq!(constraint_rows.len(), 7);
        for constraint in constraint_rows {
            let column: String = constraint.get("column_name");
            let nullable: String = constraint.get("is_nullable");
            let default: Option<String> = constraint.get("column_default");
            assert_eq!(nullable, "NO", "{column} should be NOT NULL");
            match column.as_str() {
                "enabled" => assert!(default.as_deref().is_some_and(|value| value == "true")),
                "job_kind" | "origin" => assert!(
                    default
                        .as_deref()
                        .is_some_and(|value| value.contains("'user'")),
                    "{column} should default to user"
                ),
                "max_attempts" => assert!(default.as_deref().is_some_and(|value| value == "1")),
                "consecutive_failures" => {
                    assert!(default.as_deref().is_some_and(|value| value == "0"));
                }
                "created_at" | "next_run" => assert!(
                    default.is_none(),
                    "{column} should keep app-supplied timestamps rather than a persistent DB default"
                ),
                _ => panic!("unexpected constraint column {column}"),
            }
        }
        pool.close().await;
    });

    let jobs = list_jobs(&config).expect("legacy row should map after schema backfill");
    assert_eq!(jobs.len(), 2);
    let valid_job = jobs
        .iter()
        .find(|job| job.id == "legacy-job")
        .expect("valid legacy job should be listed");
    assert!(valid_job.enabled);
    assert_eq!(valid_job.job_kind, CronJobKind::User);
    assert_eq!(valid_job.origin, CronJobOrigin::User);
    assert_eq!(valid_job.max_attempts, 1);
    assert_eq!(valid_job.consecutive_failures, 0);
    let invalid_job = jobs
        .iter()
        .find(|job| job.id == "invalid-legacy-job")
        .expect("invalid legacy job should be listed for operator repair");
    assert!(!invalid_job.enabled);
    assert!(
        due_jobs(&config, Utc::now()).unwrap().is_empty(),
        "backfilled legacy job should not be immediately due"
    );
}
