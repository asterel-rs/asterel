//! Async cron scheduler loop and job execution engine.
//!
//! Periodically polls for due jobs, routes them to shell or
//! guarded agent-origin executors, applies security policy checks,
//! and manages quiet-hours and circuit-breaker backoff.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinHandle;
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
const USER_SHELL_TIMEOUT_SECONDS: u64 = 30;
const USER_SHELL_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
const USER_SHELL_OUTPUT_DRAIN_SECONDS: u64 = 2;
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
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    run_until_shutdown(config, shutdown_rx).await
}

/// Runs the cron scheduler poll loop until a shutdown signal is observed.
///
/// In-flight jobs are allowed to finish before the signal is honored so reload
/// does not abort a job after its side effect but before reschedule persistence.
pub async fn run_until_shutdown(
    config: Arc<Config>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let poll_secs = config.reliability.scheduler_poll_secs.max(MIN_POLL_SECONDS);
    let mut interval = time::interval(Duration::from_secs(poll_secs));
    let security = SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );

    crate::runtime::diagnostics::health::mark_component_ok("scheduler");

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    break;
                }
            }
        }
        if *shutdown.borrow() {
            break;
        }
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
            if *shutdown.borrow() {
                break;
            }
        }
    }

    Ok(())
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

    run_bounded_user_shell_command(&job.command, &config.workspace_dir).await
}

async fn run_bounded_user_shell_command(
    command: &str,
    current_dir: &std::path::Path,
) -> (bool, String) {
    run_bounded_user_shell_command_with_limits(
        command,
        current_dir,
        Duration::from_secs(USER_SHELL_TIMEOUT_SECONDS),
        USER_SHELL_OUTPUT_LIMIT_BYTES,
    )
    .await
}

async fn run_bounded_user_shell_command_with_limits(
    command: &str,
    current_dir: &std::path::Path,
    timeout: Duration,
    output_limit: usize,
) -> (bool, String) {
    // Use `-c` (not `-lc`) to avoid loading the login shell profile,
    // and clear the environment to match ShellTool's hardened execution model.
    let mut shell = Command::new("sh");
    shell
        .arg("-c")
        .arg(command)
        .current_dir(current_dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .env("LANG", std::env::var("LANG").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_shell_process_group(&mut shell);
    let child = shell.spawn();

    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            return (
                false,
                format!("{ROUTE_MARKER_USER_SHELL}\nspawn error: {e}"),
            );
        }
    };

    let process_id = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(async move { read_limited_output(stdout, output_limit).await });
    let stderr_task = tokio::spawn(async move { read_limited_output(stderr, output_limit).await });

    let wait_result = time::timeout(timeout, child.wait()).await;
    let (success, status_line, timed_out) = match wait_result {
        Ok(Ok(status)) => (status.success(), format!("status={status}"), false),
        Ok(Err(error)) => (false, format!("wait error: {error}"), false),
        Err(_) => {
            terminate_shell_process_group(process_id);
            let _ = child.kill().await;
            let _ = child.wait().await;
            (false, format!("timeout after {}s", timeout.as_secs()), true)
        }
    };
    terminate_shell_process_group(process_id);

    let drain_timeout = Duration::from_secs(USER_SHELL_OUTPUT_DRAIN_SECONDS).min(timeout);
    let stdout = await_limited_output_task(stdout_task, "stdout", drain_timeout, process_id).await;
    let stderr = await_limited_output_task(stderr_task, "stderr", drain_timeout, process_id).await;

    let mut combined = format!(
        "{ROUTE_MARKER_USER_SHELL}\n{status_line}\nstdout:\n{}\nstderr:\n{}",
        stdout.text.trim(),
        stderr.text.trim()
    );
    if stdout.truncated || stderr.truncated {
        combined.push_str("\noutput_truncated=true");
    }
    if timed_out {
        combined.push_str("\ntimeout=true");
    }
    if let Some(error) = stdout.read_error.as_deref() {
        combined.push_str(&format!("\nstdout_read_error={error}"));
    }
    if let Some(error) = stderr.read_error.as_deref() {
        combined.push_str(&format!("\nstderr_read_error={error}"));
    }

    (success, combined)
}

#[cfg(unix)]
fn configure_shell_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_shell_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_shell_process_group(process_id: Option<u32>) {
    let Some(process_id) = process_id else {
        return;
    };
    if let Ok(pid) = i32::try_from(process_id) {
        let Some(pid) = rustix::process::Pid::from_raw(pid) else {
            return;
        };
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(not(unix))]
fn terminate_shell_process_group(_process_id: Option<u32>) {}

struct LimitedOutput {
    text: String,
    truncated: bool,
    read_error: Option<String>,
}

async fn await_limited_output_task(
    mut task: JoinHandle<LimitedOutput>,
    stream_name: &'static str,
    timeout: Duration,
    process_id: Option<u32>,
) -> LimitedOutput {
    match time::timeout(timeout, &mut task).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => LimitedOutput {
            text: String::new(),
            truncated: true,
            read_error: Some(format!("{stream_name} task failed: {error}")),
        },
        Err(_) => {
            terminate_shell_process_group(process_id);
            task.abort();
            LimitedOutput {
                text: String::new(),
                truncated: true,
                read_error: Some(format!("{stream_name} drain timeout")),
            }
        }
    }
}

async fn read_limited_output<R>(reader: Option<R>, limit: usize) -> LimitedOutput
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return LimitedOutput {
            text: String::new(),
            truncated: false,
            read_error: None,
        };
    };
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    let mut truncated = false;
    let mut buffer = [0_u8; 8 * 1024];

    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => {
                let available = limit.saturating_sub(bytes.len());
                if available > 0 {
                    let keep = read.min(available);
                    bytes.extend_from_slice(&buffer[..keep]);
                    if keep < read {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(error) => {
                return LimitedOutput {
                    text: String::from_utf8_lossy(&bytes).into_owned(),
                    truncated,
                    read_error: Some(error.to_string()),
                };
            }
        }
    }

    LimitedOutput {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
        read_error: None,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{Duration, run_bounded_user_shell_command_with_limits};

    #[tokio::test]
    async fn scheduler_user_shell_times_out() {
        let temp = TempDir::new().expect("temp dir");
        let (success, output) = run_bounded_user_shell_command_with_limits(
            "while true; do sleep 1; done",
            temp.path(),
            Duration::from_millis(50),
            1024,
        )
        .await;

        assert!(!success);
        assert!(output.contains("timeout=true"), "{output}");
    }

    #[tokio::test]
    async fn scheduler_user_shell_output_is_bounded() {
        let temp = TempDir::new().expect("temp dir");
        let (success, output) = run_bounded_user_shell_command_with_limits(
            "printf 'abcdefghijklmnopqrstuvwxyz'",
            temp.path(),
            Duration::from_secs(5),
            8,
        )
        .await;

        assert!(success, "{output}");
        assert!(output.contains("stdout:\nabcdefgh"), "{output}");
        assert!(output.contains("output_truncated=true"), "{output}");
        assert!(!output.contains("ijklmnopqrstuvwxyz"), "{output}");
    }

    #[tokio::test]
    async fn scheduler_user_shell_background_descendant_is_bounded() {
        let temp = TempDir::new().expect("temp dir");
        let started = std::time::Instant::now();
        let (success, output) = run_bounded_user_shell_command_with_limits(
            "sleep 5 & printf done",
            temp.path(),
            Duration::from_millis(100),
            1024,
        )
        .await;

        assert!(success, "{output}");
        assert!(started.elapsed() < Duration::from_secs(3), "{output}");
        assert!(
            output.contains("done") || output.contains("drain timeout"),
            "{output}"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn scheduler_user_shell_reaps_redirected_background_descendant() {
        let temp = TempDir::new().expect("temp dir");
        let (success, output) = run_bounded_user_shell_command_with_limits(
            "sleep 5 >/dev/null 2>&1 & echo $! > child.pid; printf done",
            temp.path(),
            Duration::from_secs(5),
            1024,
        )
        .await;

        assert!(success, "{output}");
        let pid = std::fs::read_to_string(temp.path().join("child.pid"))
            .expect("child pid should be written")
            .trim()
            .to_string();
        let proc_path = std::path::Path::new("/proc").join(&pid);
        for _ in 0..20 {
            if !proc_path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("background descendant {pid} survived cron shell cleanup: {output}");
    }
}
