//! Async cron scheduler loop and job execution engine.
//!
//! Periodically polls for due jobs, routes them to shell or
//! guarded agent-origin executors, applies security policy checks,
//! and manages quiet-hours and circuit-breaker backoff.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};
use tokio::process::Command;
use tokio::time::{self, Duration};

use crate::config::Config;
use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::platform::cron::{CronJob, due_jobs, reschedule_after_run_with_breaker_state};
use crate::security::SecurityPolicy;

mod agent_command_guard;
mod policy;
mod routes;

use agent_command_guard::run_agent_job_command;
use policy::enforce_policy_invariants;
use routes::{
    ParsedRoutedJob, malformed_routed_job_output, parse_routed_job_command,
    run_channel_send_job_command, run_ingestion_job_command, run_rss_poll_job_command,
    run_trend_aggregation_job_command, run_x_poll_job_command,
};

const MIN_POLL_SECONDS: u64 = 5;
const ROUTE_MARKER_USER_SHELL: &str = "route=user-direct-shell";
const ROUTE_MARKER_AGENT_BLOCKED: &str = "route=agent-no-direct-shell";
const ROUTE_MARKER_INGEST_PIPELINE: &str = "route=user-ingestion-pipeline";
const ROUTE_MARKER_TREND_AGGREGATION: &str = "route=user-trend-aggregation";
const ROUTE_MARKER_X_POLL: &str = "route=user-x-poll";
const ROUTE_MARKER_RSS_POLL: &str = "route=user-rss-poll";
const ROUTE_MARKER_CHANNEL_SEND: &str = "route=user-channel-send";
const ROUTE_MARKER_INVALID_ROUTE: &str = "route=invalid-routed-cron";
const TREND_AGGREGATION_LIMIT: usize = 20;
const TREND_AGGREGATION_TOP_ITEMS: usize = 5;
const INGEST_API_MIN_INTERVAL_SECONDS: i64 = 10;
const INGEST_RSS_MIN_INTERVAL_SECONDS: i64 = 30;
const X_RECENT_SEARCH_ENDPOINT: &str = "https://api.twitter.com/2/tweets/search/recent";
fn scheduler_breaker_enabled(config: &Config) -> bool {
    config.reliability.scheduler_failure_budget > 0
        && config.reliability.scheduler_breaker_cooldown_secs > 0
}

fn job_breaker_is_open(job: &CronJob, now: DateTime<Utc>) -> bool {
    match job.breaker_open_until {
        Some(until) => now < until,
        None => false,
    }
}

fn next_job_breaker_state(
    config: &Config,
    job: &CronJob,
    success: bool,
    now: DateTime<Utc>,
) -> (u32, Option<DateTime<Utc>>) {
    if !scheduler_breaker_enabled(config) || success {
        return (0, None);
    }

    let failures = job.consecutive_failures.saturating_add(1);
    if failures > config.reliability.scheduler_failure_budget {
        let cooldown_secs =
            i64::try_from(config.reliability.scheduler_breaker_cooldown_secs).unwrap_or(i64::MAX);
        let open_until = now + ChronoDuration::seconds(cooldown_secs);
        (0, Some(open_until))
    } else {
        (failures, None)
    }
}

fn parse_hhmm_minutes(raw: &str) -> Option<u32> {
    let (hour_raw, minute_raw) = raw.trim().split_once(':')?;
    let hour = hour_raw.parse::<u32>().ok()?;
    let minute = minute_raw.parse::<u32>().ok()?;
    if hour <= 23 && minute <= 59 {
        Some(hour * 60 + minute)
    } else {
        None
    }
}

fn in_scheduler_hours(config: &Config, now: DateTime<Utc>) -> bool {
    let start_raw = config
        .reliability
        .scheduler_active_hours_start_utc
        .as_deref();
    let end_raw = config.reliability.scheduler_active_hours_end_utc.as_deref();

    let (start_raw, end_raw) = match (start_raw, end_raw) {
        (None, None) => return true,
        (Some(_), None) | (None, Some(_)) => {
            tracing::warn!(
                "active-hours config requires both start and end values; scheduler guarded closed"
            );
            return false;
        }
        (Some(start), Some(end)) => (start, end),
    };

    let Some(start_minutes) = parse_hhmm_minutes(start_raw) else {
        tracing::warn!(
            start = start_raw,
            "invalid reliability.scheduler_active_hours_start_utc; scheduler guarded closed"
        );
        return false;
    };
    let Some(end_minutes) = parse_hhmm_minutes(end_raw) else {
        tracing::warn!(
            end = end_raw,
            "invalid reliability.scheduler_active_hours_end_utc; scheduler guarded closed"
        );
        return false;
    };

    if start_minutes == end_minutes {
        return true;
    }

    let current_minutes = now.hour() * 60 + now.minute();
    if start_minutes < end_minutes {
        current_minutes >= start_minutes && current_minutes < end_minutes
    } else {
        current_minutes >= start_minutes || current_minutes < end_minutes
    }
}

/// Runs the cron scheduler poll loop until cancellation.
///
/// # Errors
///
/// Returns an error only on unrecoverable startup failures;
/// individual job errors are logged and retried.
pub async fn run(config: Arc<Config>) -> Result<()> {
    let poll_secs = config.reliability.scheduler_poll_secs.max(MIN_POLL_SECONDS);
    let mut interval = time::interval(Duration::from_secs(poll_secs));
    let security = SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );

    crate::runtime::diagnostics::health::mark_component_ok("scheduler");

    loop {
        interval.tick().await;
        crate::runtime::diagnostics::health::mark_component_ok("scheduler");
        let now = Utc::now();

        if !in_scheduler_hours(&config, now) {
            continue;
        }

        let jobs = match due_jobs(&config, now) {
            Ok(jobs) => jobs,
            Err(e) => {
                crate::runtime::diagnostics::health::mark_component_error(
                    "scheduler",
                    e.to_string(),
                );
                tracing::warn!("Scheduler query failed: {e}");
                continue;
            }
        };

        for job in jobs {
            let job_now = Utc::now();
            if job_breaker_is_open(&job, job_now) {
                crate::runtime::diagnostics::health::mark_component_error(
                    "scheduler",
                    format!(
                        "job {} breaker open until {}",
                        job.id,
                        job.breaker_open_until
                            .map_or_else(|| "unknown".to_string(), |until| until.to_rfc3339())
                    ),
                );
                continue;
            }

            crate::runtime::diagnostics::health::mark_component_ok("scheduler");
            let (success, output) = execute_job_with_retry(&config, &security, &job).await;

            if !success {
                crate::runtime::diagnostics::health::mark_component_error(
                    "scheduler",
                    format!("job {} failed", job.id),
                );
            }

            let breaker_now = Utc::now();
            let (consecutive_failures, breaker_open_until) =
                next_job_breaker_state(&config, &job, success, breaker_now);
            if let Err(e) = reschedule_after_run_with_breaker_state(
                &config,
                &job,
                success,
                &output,
                consecutive_failures,
                breaker_open_until,
            ) {
                crate::runtime::diagnostics::health::mark_component_error(
                    "scheduler",
                    e.to_string(),
                );
                tracing::warn!("Failed to persist scheduler run+breaker state: {e}");
            }
        }
    }
}

async fn execute_job_with_retry(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    let mut last_output = String::new();
    let retries = effective_retry_budget(config, job);
    let mut backoff_ms = config.reliability.provider_backoff_ms.max(200);

    for attempt in 0..=retries {
        let (success, output) = run_job_command(config, security, job).await;
        last_output = output;

        if success {
            return (true, last_output);
        }

        if last_output.contains(SECURITY_POLICY_BLOCK_PREFIX.trim_end()) {
            // Deterministic policy violations are not retryable.
            return (false, last_output);
        }

        if attempt < retries {
            let jitter_ms = u64::from(Utc::now().timestamp_subsec_millis() % 250);
            time::sleep(Duration::from_millis(backoff_ms + jitter_ms)).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
        }
    }

    (false, last_output)
}

fn effective_retry_budget(config: &Config, job: &CronJob) -> u32 {
    let retries = config.reliability.scheduler_retries;
    if job.origin == crate::platform::cron::CronJobOrigin::Agent {
        retries.min(job.max_attempts.saturating_sub(1))
    } else {
        retries
    }
}

async fn run_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    match job.origin {
        crate::platform::cron::CronJobOrigin::User => {
            run_user_job_command(config, security, job).await
        }
        crate::platform::cron::CronJobOrigin::Agent => {
            run_agent_job_command(config, security, job).await
        }
    }
}

/// Executes a single job once without retry, for integration
/// tests.
pub async fn run_job_once(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    run_job_command(config, security, job).await
}

async fn run_user_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    if let Some(parsed) = parse_routed_job_command(&job.command) {
        return match parsed {
            ParsedRoutedJob::Ingestion(parsed) => {
                run_ingestion_job_command(config, security, parsed).await
            }
            ParsedRoutedJob::TrendAggregation(parsed) => {
                run_trend_aggregation_job_command(config, security, parsed).await
            }
            ParsedRoutedJob::XPoll(parsed) => {
                run_x_poll_job_command(config, security, parsed).await
            }
            ParsedRoutedJob::RssPoll(parsed) => {
                run_rss_poll_job_command(config, security, parsed).await
            }
            ParsedRoutedJob::ChannelSend { .. } => {
                run_channel_send_job_command(config, security, &job.command).await
            }
        };
    }

    if let Some(output) = malformed_routed_job_output(&job.command) {
        return (false, output);
    }

    if let Err(output) = enforce_policy_invariants(security, &job.command, ROUTE_MARKER_USER_SHELL)
    {
        return (false, output);
    }

    // Use `-c` (not `-lc`) to avoid loading the login shell profile,
    // and clear the environment to match ShellTool's hardened execution model.
    let output = Command::new("sh")
        .arg("-c")
        .arg(&job.command)
        .current_dir(&config.workspace_dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .env("LANG", std::env::var("LANG").unwrap_or_default())
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!(
                "{ROUTE_MARKER_USER_SHELL}\nstatus={}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout.trim(),
                stderr.trim()
            );
            (output.status.success(), combined)
        }
        Err(e) => (
            false,
            format!("{ROUTE_MARKER_USER_SHELL}\nspawn error: {e}"),
        ),
    }
}
