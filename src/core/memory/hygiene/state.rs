//! Hygiene scheduler and state persistence.
//!
//! Tracks when the last hygiene pass ran and triggers a new cycle
//! (filesystem archival + `PostgreSQL` pruning) when the interval elapses.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::filesystem::{
    archive_daily_memory_files, archive_session_files, purge_memory_archives,
    purge_session_archives,
};
use super::promotion;
use super::prune::{LifecyclePruneReport, prune_conversation_rows, prune_lifecycle_rows};
use super::sleep;
use crate::config::MemoryConfig;

/// Minimum hours between successive hygiene runs.
pub(super) const HYGIENE_INTERVAL_HOURS: i64 = 12;
const STATE_FILE: &str = "memory_hygiene_state.json";

/// Combined report from a full hygiene cycle (filesystem + `PostgreSQL`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct HygieneReport {
    archived_memory_files: u64,
    archived_session_files: u64,
    purged_memory_archives: u64,
    purged_session_archives: u64,
    pruned_conversation_rows: u64,
    promoted_count: u64,
    sleep_consolidated_groups: u64,
    sleep_snapshots_written: u64,
    lifecycle: LifecyclePruneReport,
}

impl HygieneReport {
    fn total_actions(&self) -> u64 {
        self.archived_memory_files
            + self.archived_session_files
            + self.purged_memory_archives
            + self.purged_session_archives
            + self.pruned_conversation_rows
            + self.promoted_count
            + self.sleep_consolidated_groups
            + self.sleep_snapshots_written
            + self.lifecycle.total_actions()
    }
}

/// Persisted state tracking the last hygiene run timestamp.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)] // serialized to JSON; renaming would break existing state files
pub(super) struct HygieneState {
    last_run_at: Option<String>,
    last_sleep_run_at: Option<String>,
    last_report: HygieneReport,
}

/// Run memory/session hygiene if the cadence window has elapsed.
///
/// This function is intentionally best-effort: callers should log and continue on failure.
///
/// # Errors
///
/// Returns an error when hygiene actions or hygiene state persistence fails.
pub fn run_if_due(config: &MemoryConfig, workspace_dir: &Path) -> Result<()> {
    if !config.hygiene_enabled {
        return Ok(());
    }

    let previous_state = load_state(workspace_dir)?;

    if !should_run_now(previous_state.as_ref()) {
        return Ok(());
    }

    let lifecycle = prune_lifecycle_rows(workspace_dir, config)?;
    let promotion = promotion::promote_expiring_working_memories(workspace_dir, config)?;

    let mut report = HygieneReport {
        archived_memory_files: archive_daily_memory_files(workspace_dir, config.archive_after_days)
            .context("archive daily memory files")?,
        archived_session_files: archive_session_files(workspace_dir, config.archive_after_days)
            .context("archive session files")?,
        purged_memory_archives: purge_memory_archives(workspace_dir, config.purge_after_days)
            .context("purge expired memory archives")?,
        purged_session_archives: purge_session_archives(workspace_dir, config.purge_after_days)
            .context("purge expired session archives")?,
        pruned_conversation_rows: prune_conversation_rows(
            workspace_dir,
            config,
            config.conversation_retention_days,
        )
        .context("prune stale conversation rows")?,
        promoted_count: promotion.promoted_count,
        sleep_consolidated_groups: 0,
        sleep_snapshots_written: 0,
        lifecycle,
    };

    let mut last_sleep_run_at = previous_state
        .as_ref()
        .and_then(|state| state.last_sleep_run_at.clone());
    if should_run_sleep(previous_state.as_ref()) {
        let sleep_report = sleep::run_sleep_consolidation(workspace_dir, config)
            .context("run sleep consolidation")?;
        report.sleep_consolidated_groups = sleep_report.consolidated_groups;
        report.sleep_snapshots_written = sleep_report.snapshots_written;
        last_sleep_run_at = Some(Utc::now().to_rfc3339());
    }

    write_state(workspace_dir, &report, last_sleep_run_at).context("save hygiene state")?;

    if report.total_actions() > 0 {
        tracing::info!(
            "memory hygiene complete: archived_memory={} archived_sessions={} purged_memory={} purged_sessions={} pruned_conversation_rows={} promoted_count={} sleep_consolidated_groups={} sleep_snapshots_written={} ttl_slot_hard_deleted={} ttl_unit_purged={} low_confidence_demoted_total={} contradiction_auto_demoted_total={} stale_trend_purge_total={} recency_refresh_total={} layer_cleanup_total={} ledger_purged={}",
            report.archived_memory_files,
            report.archived_session_files,
            report.purged_memory_archives,
            report.purged_session_archives,
            report.pruned_conversation_rows,
            report.promoted_count,
            report.sleep_consolidated_groups,
            report.sleep_snapshots_written,
            report.lifecycle.ttl_slot_hard_deleted,
            report.lifecycle.ttl_unit_purged,
            report.lifecycle.low_confidence_demoted,
            report.lifecycle.contradiction_auto_demoted,
            report.lifecycle.stale_trend_demoted,
            report.lifecycle.recency_refreshed,
            report.lifecycle.layer_cleanup_actions,
            report.lifecycle.ledger_purged,
        );
    }

    Ok(())
}

fn load_state(workspace_dir: &Path) -> Result<Option<HygieneState>> {
    let raw = match fs::read_to_string(state_path(workspace_dir)) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::Error::new(e).context("read hygiene state file")),
    };

    Ok(serde_json::from_str(&raw).ok())
}

fn should_run_now(state: Option<&HygieneState>) -> bool {
    let Some(last_run_at) = state.and_then(|s| s.last_run_at.as_deref()) else {
        return true;
    };

    let last = match DateTime::parse_from_rfc3339(last_run_at) {
        Ok(ts) => ts.with_timezone(&Utc),
        Err(_) => return true,
    };

    Utc::now().signed_duration_since(last) >= Duration::hours(HYGIENE_INTERVAL_HOURS)
}

fn should_run_sleep(state: Option<&HygieneState>) -> bool {
    let Some(last_sleep_run_at) = state.and_then(|s| s.last_sleep_run_at.as_deref()) else {
        return true;
    };

    let last = match DateTime::parse_from_rfc3339(last_sleep_run_at) {
        Ok(ts) => ts.with_timezone(&Utc),
        Err(_) => return true,
    };

    Utc::now().signed_duration_since(last) >= Duration::hours(sleep::sleep_interval_hours())
}

fn write_state(
    workspace_dir: &Path,
    report: &HygieneReport,
    last_sleep_run_at: Option<String>,
) -> Result<()> {
    let path = state_path(workspace_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create hygiene state directory")?;
    }

    let state = HygieneState {
        last_run_at: Some(Utc::now().to_rfc3339()),
        last_sleep_run_at,
        last_report: report.clone(),
    };
    let json = serde_json::to_vec_pretty(&state).context("serialize hygiene state")?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json).context("write hygiene state to temp file")?;
    fs::rename(&tmp, &path).context("rename hygiene state temp file")?;
    Ok(())
}

fn state_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("state").join(STATE_FILE)
}
