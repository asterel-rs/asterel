//! Doctor check runner: prints setup health, daemon health,
//! and governance/rollout diagnostics to stdout.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::Path;

use super::report::{
    autonomy_governance_lines, memory_rollout_lines, memory_signal_stats_lines, parse_rfc3339,
    persona_calibration_lines, persona_continuity_gate_lines, persona_drift_lines,
    persona_embodied_state_lines,
};
use super::setup::{apply_setup_repairs, run_setup_checks};
use crate::config::Config;
use crate::runtime::services::load_runtime_operational_snapshot;
use crate::ui::style as ui;

const DAEMON_STALE_SECONDS: i64 = 30;
const SCHEDULER_STALE_SECONDS: i64 = 120;
const CHANNEL_STALE_SECONDS: i64 = 300;

/// # Errors
///
/// Returns an error when reading or parsing runtime state required for doctor
/// checks fails.
pub fn run(config: &Config, repair: bool) -> Result<()> {
    println!("{}", ui::section(t!("doctor.title")));
    println!();

    if repair {
        print_repair_actions(config)?;
    }
    print_setup_health(config);
    print_daemon_health(config)?;
    print_governance_and_rollout(config);

    Ok(())
}

// ── Setup Health ──

fn print_repair_actions(config: &Config) -> Result<()> {
    println!("{}", ui::section_with_rule("Repair"));
    let actions = apply_setup_repairs(config)?;
    if actions.is_empty() {
        println!("{}", ui::skip_line("No automatic repairs were needed."));
    } else {
        for action in actions {
            println!("{}", ui::pass_line(&action));
        }
    }
    if let Err(error) = config.validate_autonomy_controls() {
        println!(
            "{}",
            ui::warn_line(format!("manual config repair still required: {error}"))
        );
    }
    println!();
    Ok(())
}

fn print_setup_health(config: &Config) {
    println!("{}", ui::section_with_rule("Setup Health"));
    let setup_checks = run_setup_checks(config);
    let mut setup_warnings = 0u32;
    for (pass, msg) in &setup_checks {
        if *pass {
            println!("{}", ui::pass_line(msg));
        } else {
            setup_warnings += 1;
            println!("{}", ui::fail_line(msg));
        }
    }
    if setup_warnings == 0 {
        println!("{}", ui::pass_line(ui::dim("All setup checks passed.")));
    } else {
        println!(
            "{}",
            ui::warn_line(format!(
                "{setup_warnings} issue(s) found. Run '{}' or '{}' to fix.",
                ui::yellow("asterel doctor --repair"),
                ui::yellow("asterel onboard"),
            ))
        );
    }
    println!();
}

// ── Daemon Health ──

fn print_daemon_health(config: &Config) -> Result<()> {
    println!("{}", ui::section_with_rule("Daemon Health"));
    let operational = load_runtime_operational_snapshot(config);
    let state_file = crate::platform::daemon::state_file_path(config);
    for line in daemon_health_lines(&state_file, &operational)? {
        println!("{line}");
    }
    println!();
    Ok(())
}

fn daemon_health_lines(
    state_file: &Path,
    operational: &crate::runtime::services::RuntimeOperationalSnapshot,
) -> Result<Vec<String>> {
    if !state_file.exists() {
        return Ok(vec![
            ui::fail_line(t!("doctor.state_not_found", path = state_file.display())),
            ui::note_line(t!("doctor.start_hint")),
        ]);
    }

    let raw = std::fs::read_to_string(state_file)
        .with_context(|| format!("Failed to read {}", state_file.display()))?;
    let snapshot: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse {}", state_file.display()))?;

    let mut lines = vec![ui::pass_line(t!(
        "doctor.state_file",
        path = state_file.display()
    ))];

    let updated_at = snapshot
        .get("updated_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    if let Ok(ts) = DateTime::parse_from_rfc3339(updated_at) {
        let age = Utc::now()
            .signed_duration_since(ts.with_timezone(&Utc))
            .num_seconds();
        if age < 0 {
            lines.push(ui::fail_line(format!(
                "daemon heartbeat timestamp is in the future: {updated_at}"
            )));
        } else if age <= DAEMON_STALE_SECONDS {
            lines.push(ui::pass_line(t!("doctor.heartbeat_fresh", age = age)));
        } else {
            lines.push(ui::fail_line(t!("doctor.heartbeat_stale", age = age)));
        }
    } else {
        lines.push(ui::fail_line(t!(
            "doctor.timestamp_invalid",
            value = updated_at
        )));
    }

    let (component_lines, channel_count, stale_channels) =
        daemon_component_lines(&snapshot, operational);
    lines.extend(component_lines);

    if channel_count == 0 {
        lines.push(ui::note_line(t!("doctor.no_channels")));
    } else {
        lines.push(format!(
            "  {}",
            t!(
                "doctor.channel_summary",
                total = channel_count,
                stale = stale_channels
            )
        ));
    }

    Ok(lines)
}

fn daemon_component_lines(
    snapshot: &serde_json::Value,
    operational: &crate::runtime::services::RuntimeOperationalSnapshot,
) -> (Vec<String>, u32, u32) {
    let components = snapshot
        .get("components")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut lines = vec![
        gateway_component_status_line(&components),
        scheduler_component_status_line(&components, operational),
    ];
    let (channel_lines, channel_count, stale_channels) =
        channel_component_health_lines(&components);
    lines.extend(channel_lines);
    (lines, channel_count, stale_channels)
}

fn gateway_component_status_line(
    components: &serde_json::Map<String, serde_json::Value>,
) -> String {
    if let Some(gateway) = components.get("gateway") {
        let gateway_ok = gateway
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| s == "ok");
        if gateway_ok {
            ui::pass_line("gateway healthy")
        } else {
            let error = gateway
                .get("last_error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error");
            ui::fail_line(format!("gateway unhealthy (last_error={error})"))
        }
    } else {
        ui::fail_line("gateway component missing")
    }
}

fn scheduler_component_status_line(
    components: &serde_json::Map<String, serde_json::Value>,
    operational: &crate::runtime::services::RuntimeOperationalSnapshot,
) -> String {
    if !operational.cron.is_runtime_required() {
        return ui::note_line(format!(
            "scheduler unsupported: {}",
            operational
                .cron
                .reason
                .as_deref()
                .unwrap_or("postgres-backed scheduler unavailable")
        ));
    }

    let Some(scheduler) = components.get("scheduler") else {
        return ui::fail_line(t!("doctor.scheduler_missing"));
    };

    let scheduler_ok = scheduler
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| s == "ok");

    let scheduler_last_ok = scheduler
        .get("last_ok")
        .and_then(serde_json::Value::as_str)
        .and_then(nonnegative_age_from_rfc3339)
        .unwrap_or(i64::MAX);

    if scheduler_ok && scheduler_last_ok <= SCHEDULER_STALE_SECONDS {
        ui::pass_line(t!("doctor.scheduler_healthy", age = scheduler_last_ok))
    } else {
        ui::fail_line(t!(
            "doctor.scheduler_unhealthy",
            ok = scheduler_ok,
            age = scheduler_last_ok
        ))
    }
}

fn channel_component_health_lines(
    components: &serde_json::Map<String, serde_json::Value>,
) -> (Vec<String>, u32, u32) {
    let mut channel_count = 0_u32;
    let mut stale_channels = 0_u32;
    let mut lines = Vec::new();

    for (name, component) in components {
        if !name.starts_with("channel:") {
            continue;
        }

        channel_count += 1;
        let status_ok = component
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| s == "ok");
        let age = component
            .get("last_ok")
            .and_then(serde_json::Value::as_str)
            .and_then(nonnegative_age_from_rfc3339)
            .unwrap_or(i64::MAX);

        if status_ok && age <= CHANNEL_STALE_SECONDS {
            lines.push(ui::pass_line(t!(
                "doctor.channel_fresh",
                name = name,
                age = age
            )));
        } else {
            stale_channels += 1;
            lines.push(ui::fail_line(t!(
                "doctor.channel_stale",
                name = name,
                ok = status_ok,
                age = age
            )));
        }
    }

    (lines, channel_count, stale_channels)
}

fn nonnegative_age_from_rfc3339(value: &str) -> Option<i64> {
    let age = Utc::now()
        .signed_duration_since(parse_rfc3339(value)?)
        .num_seconds();
    (age >= 0).then_some(age)
}

fn print_governance_and_rollout(config: &Config) {
    println!(
        "{}",
        ui::section_with_rule(t!("doctor.autonomy_governance"))
    );
    for line in autonomy_governance_lines(config) {
        println!("    {line}");
    }

    println!();
    println!("{}", ui::section_with_rule(t!("doctor.memory_rollout")));
    let state_file = crate::platform::daemon::state_file_path(config);
    let snapshot = if state_file.exists() {
        std::fs::read_to_string(&state_file)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    } else {
        None
    };
    let snapshot = snapshot.unwrap_or_else(|| serde_json::json!({}));
    for line in memory_rollout_lines(config, &snapshot) {
        println!("    {line}");
    }

    println!();
    println!("  {}", ui::subsection("Memory Signal Stats"));
    for line in memory_signal_stats_lines(config) {
        println!("    {line}");
    }

    println!();
    println!("  {}", ui::subsection("Persona Drift"));
    for line in persona_drift_lines(config) {
        println!("    {line}");
    }

    println!();
    println!("  {}", ui::subsection("Persona Calibration"));
    for line in persona_calibration_lines(config) {
        println!("    {line}");
    }

    println!();
    println!("  {}", ui::subsection("Continuity Gate"));
    for line in persona_continuity_gate_lines(config) {
        println!("    {line}");
    }

    println!();
    println!("  {}", ui::subsection("Embodied State"));
    for line in persona_embodied_state_lines(config) {
        println!("    {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHANNEL_STALE_SECONDS, SCHEDULER_STALE_SECONDS, channel_component_health_lines,
        daemon_health_lines,
    };
    use crate::runtime::services::{RuntimeCapabilityState, RuntimeOperationalSnapshot};
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    fn supported_operational_snapshot() -> RuntimeOperationalSnapshot {
        RuntimeOperationalSnapshot {
            onboarding_required: false,
            channels: Vec::new(),
            cron: RuntimeCapabilityState::supported(),
            session_persistence: RuntimeCapabilityState::supported(),
            memory_signal_metrics: RuntimeCapabilityState::supported(),
            persona_state_metrics: RuntimeCapabilityState::supported(),
            memory_review: RuntimeCapabilityState::supported(),
            observability: RuntimeCapabilityState::supported(),
        }
    }

    fn unsupported_cron_snapshot(reason: &str) -> RuntimeOperationalSnapshot {
        RuntimeOperationalSnapshot {
            cron: RuntimeCapabilityState::unsupported(reason),
            ..supported_operational_snapshot()
        }
    }

    fn write_state_file(dir: &TempDir, snapshot: serde_json::Value) -> std::path::PathBuf {
        let path = dir.path().join("daemon-state.json");
        std::fs::write(
            &path,
            serde_json::to_string(&snapshot).expect("serialize state"),
        )
        .expect("write state file");
        path
    }

    #[test]
    fn daemon_health_reports_missing_state_file_with_start_hint() {
        let dir = TempDir::new().expect("tempdir");
        let state_file = dir.path().join("missing-state.json");

        let lines = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect("daemon health lines");

        assert!(
            lines
                .iter()
                .any(|line| line.contains("daemon state file not found"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Start daemon with: asterel daemon"))
        );
    }

    #[test]
    fn daemon_health_returns_parse_error_for_invalid_json_state() {
        let dir = TempDir::new().expect("tempdir");
        let state_file = dir.path().join("daemon-state.json");
        std::fs::write(&state_file, "{not-json").expect("write invalid state");

        let error = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect_err("invalid JSON should fail");

        let rendered = format!("{error:#}");
        assert!(rendered.contains("Failed to parse"));
        assert!(rendered.contains("daemon-state.json"));
    }

    #[test]
    fn daemon_health_reports_invalid_timestamp_and_missing_components() {
        let dir = TempDir::new().expect("tempdir");
        let state_file = write_state_file(
            &dir,
            serde_json::json!({
                "updated_at": "not-a-timestamp",
                "components": {}
            }),
        );

        let lines = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect("daemon health lines");

        assert!(lines.iter().any(|line| line.contains("State file:")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("invalid daemon timestamp"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("gateway component missing"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("scheduler component missing"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("no channel components tracked in state yet"))
        );
    }

    #[test]
    fn daemon_health_reports_future_heartbeat_as_failure() {
        let dir = TempDir::new().expect("tempdir");
        let state_file = write_state_file(
            &dir,
            serde_json::json!({
                "updated_at": (Utc::now() + Duration::seconds(30)).to_rfc3339(),
                "components": {
                    "gateway": { "status": "ok" },
                    "scheduler": { "status": "ok", "last_ok": Utc::now().to_rfc3339() }
                }
            }),
        );

        let lines = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect("daemon health lines");

        assert!(
            lines
                .iter()
                .any(|line| { line.contains("daemon heartbeat timestamp is in the future") })
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("daemon heartbeat fresh"))
        );
    }

    #[test]
    fn daemon_health_missing_components_key_reports_component_failures() {
        let dir = TempDir::new().expect("tempdir");
        let state_file = write_state_file(
            &dir,
            serde_json::json!({
                "updated_at": Utc::now().to_rfc3339()
            }),
        );

        let lines = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect("daemon health lines");

        assert!(
            lines
                .iter()
                .any(|line| line.contains("gateway component missing"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("scheduler component missing"))
        );
    }

    #[test]
    fn daemon_health_reports_gateway_scheduler_and_channel_states() {
        let dir = TempDir::new().expect("tempdir");
        let fresh = Utc::now().to_rfc3339();
        let stale_scheduler =
            (Utc::now() - Duration::seconds(SCHEDULER_STALE_SECONDS + 5)).to_rfc3339();
        let stale_channel =
            (Utc::now() - Duration::seconds(CHANNEL_STALE_SECONDS + 5)).to_rfc3339();
        let state_file = write_state_file(
            &dir,
            serde_json::json!({
                "updated_at": fresh,
                "components": {
                    "gateway": { "status": "ok" },
                    "scheduler": { "status": "ok", "last_ok": stale_scheduler },
                    "channel:slack": { "status": "ok", "last_ok": Utc::now().to_rfc3339() },
                    "channel:discord": { "status": "ok", "last_ok": stale_channel }
                }
            }),
        );

        let lines = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect("daemon health lines");

        assert!(
            lines
                .iter()
                .any(|line| line.contains("daemon heartbeat fresh"))
        );
        assert!(lines.iter().any(|line| line.contains("gateway healthy")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("scheduler unhealthy/stale"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("channel:slack fresh"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("channel:discord stale/unhealthy"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Channel summary: 2 total, 1 stale"))
        );
    }

    #[test]
    fn daemon_health_reports_scheduler_unsupported_without_missing_failure() {
        let dir = TempDir::new().expect("tempdir");
        let state_file = write_state_file(
            &dir,
            serde_json::json!({
                "updated_at": Utc::now().to_rfc3339(),
                "components": {
                    "gateway": { "status": "ok" }
                }
            }),
        );

        let lines = daemon_health_lines(
            &state_file,
            &unsupported_cron_snapshot("markdown memory backend has no scheduler"),
        )
        .expect("daemon health lines");

        assert!(lines.iter().any(|line| {
            line.contains("scheduler unsupported: markdown memory backend has no scheduler")
        }));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("scheduler component missing"))
        );
    }

    #[test]
    fn daemon_health_treats_future_scheduler_and_channel_timestamps_as_unhealthy() {
        let dir = TempDir::new().expect("tempdir");
        let future = (Utc::now() + Duration::seconds(60)).to_rfc3339();
        let state_file = write_state_file(
            &dir,
            serde_json::json!({
                "updated_at": Utc::now().to_rfc3339(),
                "components": {
                    "gateway": { "status": "ok" },
                    "scheduler": { "status": "ok", "last_ok": future },
                    "channel:slack": { "status": "ok", "last_ok": (Utc::now() + Duration::seconds(60)).to_rfc3339() }
                }
            }),
        );

        let lines = daemon_health_lines(&state_file, &supported_operational_snapshot())
            .expect("daemon health lines");

        assert!(
            lines
                .iter()
                .any(|line| line.contains("scheduler unhealthy/stale"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("channel:slack stale/unhealthy"))
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("channel:slack fresh"))
        );
    }

    #[test]
    fn channel_component_health_ignores_non_channels_and_marks_invalid_timestamps_stale() {
        let components = serde_json::json!({
            "gateway": { "status": "ok" },
            "channel:slack": { "status": "ok", "last_ok": "invalid" },
            "channel:discord": { "status": "error" }
        });

        let (lines, channel_count, stale_channels) =
            channel_component_health_lines(components.as_object().expect("components object"));

        assert_eq!(channel_count, 2);
        assert_eq!(stale_channels, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.contains("stale/unhealthy")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("channel:slack stale/unhealthy"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("channel:discord stale/unhealthy"))
        );
    }
}
