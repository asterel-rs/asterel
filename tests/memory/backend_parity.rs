use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, fs};

use anyhow::{Result, anyhow};
use asterel::core::memory::embeddings::NoopEmbedding;
use asterel::core::memory::{
    CapabilitySupport, ForgetMode, ForgetStatus, Memory, MemoryCategory, PostgresMemory,
    capability_matrix_for_memory, ensure_forget_mode_supported,
};

use super::memory_harness;

const REPORT_PATH_ENV: &str = "ASTEREL_PARITY_REPORT_PATH";

fn report_path() -> PathBuf {
    if let Ok(path) = std::env::var(REPORT_PATH_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    std::env::temp_dir()
        .join("asterel")
        .join("evidence")
        .join("task-19-parity-report.csv")
}

#[derive(Debug, Clone)]
struct LifecycleBaseline {
    resolve_present: bool,
    recall_contains_slot: bool,
}

#[derive(Debug, Clone)]
struct ReportRow {
    backend: &'static str,
    scenario: &'static str,
    mode: &'static str,
    support: &'static str,
    verdict: &'static str,
    status: &'static str,
    applied: bool,
    complete: bool,
    degraded: bool,
    detail: String,
}

impl ReportRow {
    fn to_csv_line(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{}",
            self.backend,
            self.scenario,
            self.mode,
            self.support,
            self.verdict,
            self.status,
            self.applied,
            self.complete,
            self.degraded,
            csv_escape(&self.detail)
        )
    }
}

fn csv_escape(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn support_label(support: CapabilitySupport) -> &'static str {
    match support {
        CapabilitySupport::Supported => "SUPPORTED",
        CapabilitySupport::Degraded => "DEGRADED",
        CapabilitySupport::Unsupported => "UNSUPPORTED",
    }
}

fn mode_label(mode: ForgetMode) -> &'static str {
    match mode {
        ForgetMode::Soft => "soft",
        ForgetMode::Hard => "hard",
        ForgetMode::Tombstone => "tombstone",
    }
}

fn status_label(status: ForgetStatus) -> &'static str {
    match status {
        ForgetStatus::Complete => "complete",
        ForgetStatus::Incomplete => "incomplete",
        ForgetStatus::DegradedNonComplete => "degraded_non_complete",
        ForgetStatus::NotApplied => "not_applied",
    }
}

fn ensure_explicit_contract(
    backend: &'static str,
    mode: ForgetMode,
    support: CapabilitySupport,
    supported_preflight: bool,
    outcome: &asterel::core::memory::ForgetOutcome,
) -> Result<&'static str> {
    let mode_name = mode_label(mode);
    match support {
        CapabilitySupport::Supported => {
            if !supported_preflight {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=supported preflight_rejected"
                ));
            }
            if outcome.is_degraded || !outcome.was_applied || !outcome.is_complete {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=supported degraded={} applied={} complete={} status={}",
                    outcome.is_degraded,
                    outcome.was_applied,
                    outcome.is_complete,
                    status_label(outcome.status)
                ));
            }
            if outcome.status != ForgetStatus::Complete {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=supported status={} expected=complete",
                    status_label(outcome.status)
                ));
            }
            Ok("PASS")
        }
        CapabilitySupport::Degraded => {
            if !supported_preflight {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=degraded preflight_rejected"
                ));
            }
            if !outcome.is_degraded
                || outcome.is_complete
                || outcome.status != ForgetStatus::DegradedNonComplete
            {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=degraded degraded={} complete={} status={} expected=degraded_non_complete",
                    outcome.is_degraded,
                    outcome.is_complete,
                    status_label(outcome.status)
                ));
            }
            Ok("DEGRADED")
        }
        CapabilitySupport::Unsupported => {
            if supported_preflight {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=unsupported preflight_allowed"
                ));
            }
            if !outcome.is_degraded
                || outcome.is_complete
                || outcome.status != ForgetStatus::DegradedNonComplete
            {
                return Err(anyhow!(
                    "UNEXPECTED_DRIFT backend={backend} mode={mode_name} support=unsupported degraded={} complete={} status={} expected=degraded_non_complete",
                    outcome.is_degraded,
                    outcome.is_complete,
                    status_label(outcome.status)
                ));
            }
            Ok("UNSUPPORTED")
        }
    }
}

async fn collect_backend_report(
    backend: &'static str,
    memory: &dyn Memory,
    baseline: &LifecycleBaseline,
) -> Result<Vec<ReportRow>> {
    let mut rows = Vec::new();
    let matrix = capability_matrix_for_memory(memory);

    let entity = format!("task19-{backend}");
    let lifecycle_key = format!("{backend}.lifecycle");
    memory_harness::append_test_event(
        memory,
        &entity,
        &lifecycle_key,
        "backend parity lifecycle payload",
        MemoryCategory::Core,
    )
    .await;

    let resolved = memory_harness::resolve_slot_value(memory, &entity, &lifecycle_key).await;
    let resolve_present = resolved.as_deref() == Some("backend parity lifecycle payload");
    if resolve_present != baseline.resolve_present {
        return Err(anyhow!(
            "UNEXPECTED_DRIFT backend={backend} scenario=resolve_present got={resolve_present} expected={}",
            baseline.resolve_present
        ));
    }
    rows.push(ReportRow {
        backend,
        scenario: "store_resolve",
        mode: "n/a",
        support: "AUTHORITATIVE",
        verdict: "PASS",
        status: "n/a",
        applied: true,
        complete: true,
        degraded: false,
        detail: "resolve_slot matched authoritative behavior".to_string(),
    });

    let recalled =
        memory_harness::recall_scoped_items(memory, &entity, "lifecycle payload", 10).await;
    let recall_contains_slot = recalled
        .iter()
        .any(|item| item.value.contains("backend parity lifecycle payload"));
    if recall_contains_slot != baseline.recall_contains_slot {
        return Err(anyhow!(
            "UNEXPECTED_DRIFT backend={backend} scenario=recall_contains_slot got={recall_contains_slot} expected={}",
            baseline.recall_contains_slot
        ));
    }
    rows.push(ReportRow {
        backend,
        scenario: "recall_scoped",
        mode: "n/a",
        support: "AUTHORITATIVE",
        verdict: "PASS",
        status: "n/a",
        applied: true,
        complete: true,
        degraded: false,
        detail: "recall returned authoritative lifecycle key".to_string(),
    });

    for mode in [ForgetMode::Soft, ForgetMode::Hard, ForgetMode::Tombstone] {
        let slot_key = format!("{backend}.forget.{}", mode_label(mode));
        memory_harness::append_test_event(
            memory,
            &entity,
            &slot_key,
            "erase-me",
            MemoryCategory::Core,
        )
        .await;

        let support = matrix.support_for_forget_mode(mode);
        let preflight = ensure_forget_mode_supported(memory, mode).is_ok();
        let outcome = memory
            .forget_slot(&entity, &slot_key, mode, "task-19 parity contract")
            .await?;
        let verdict = ensure_explicit_contract(backend, mode, support, preflight, &outcome)?;

        rows.push(ReportRow {
            backend,
            scenario: "forget_mode",
            mode: mode_label(mode),
            support: support_label(support),
            verdict,
            status: status_label(outcome.status),
            applied: outcome.was_applied,
            complete: outcome.is_complete,
            degraded: outcome.is_degraded,
            detail: format!(
                "mode={} contract={} checks={}",
                mode_label(mode),
                support_label(support),
                outcome.artifact_checks.len()
            ),
        });
    }

    Ok(rows)
}

async fn markdown_baseline(memory: &dyn Memory) -> Result<LifecycleBaseline> {
    let entity = "task19-baseline";
    let slot = "markdown.baseline.lifecycle";
    memory_harness::append_test_event(
        memory,
        entity,
        slot,
        "baseline payload",
        MemoryCategory::Core,
    )
    .await;

    let resolve_present = memory_harness::resolve_slot_value(memory, entity, slot)
        .await
        .as_deref()
        == Some("baseline payload");
    let recall_contains_slot =
        memory_harness::recall_scoped_items(memory, entity, "baseline payload", 10)
            .await
            .iter()
            .any(|item| item.value.contains("baseline payload"));
    Ok(LifecycleBaseline {
        resolve_present,
        recall_contains_slot,
    })
}

fn write_report(rows: &[ReportRow], path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("report path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;

    let mut report = String::from(
        "backend,scenario,mode,support,verdict,status,applied,complete,degraded,detail\n",
    );
    for row in rows {
        report.push_str(&row.to_csv_line());
        report.push('\n');
    }

    fs::write(path, report)?;
    Ok(())
}

#[tokio::test]
async fn memory_backend_parity_matrix() {
    let (_tmp_markdown, markdown) = memory_harness::markdown_fixture();

    let baseline = markdown_baseline(&markdown)
        .await
        .expect("markdown baseline should be established");

    let mut rows = Vec::new();
    rows.extend(
        collect_backend_report("markdown", &markdown, &baseline)
            .await
            .expect("markdown parity scenarios should satisfy parity/degraded contract"),
    );

    let postgres_enabled = if let Some(database_url) = crate::test_env::postgres_url() {
        let postgres =
            PostgresMemory::connect(&database_url, Arc::new(NoopEmbedding), 0, false, 0.0)
                .await
                .expect("postgres parity backend should connect and migrate");
        rows.extend(
            collect_backend_report("postgres", &postgres, &baseline)
                .await
                .expect("postgres parity scenarios should satisfy declared capability contracts"),
        );
        true
    } else {
        false
    };

    let report_path = report_path();
    write_report(&rows, &report_path)
        .expect("parity report should be persisted for CI diagnostics");

    let row_count = rows.len();
    let expected_rows = if postgres_enabled { 10 } else { 5 };
    assert_eq!(row_count, expected_rows);

    let row_for = |scenario: &str, mode: &str| {
        rows.iter()
            .find(|row| row.scenario == scenario && row.mode == mode)
            .unwrap_or_else(|| panic!("missing row scenario={scenario} mode={mode}"))
    };

    let store_resolve = row_for("store_resolve", "n/a");
    assert_eq!(store_resolve.verdict, "PASS");
    assert_eq!(store_resolve.support, "AUTHORITATIVE");

    let recall_scoped = row_for("recall_scoped", "n/a");
    assert_eq!(recall_scoped.verdict, "PASS");
    assert_eq!(recall_scoped.support, "AUTHORITATIVE");

    let soft = row_for("forget_mode", "soft");
    assert_eq!(soft.support, "DEGRADED");
    assert_eq!(soft.status, "degraded_non_complete");

    let hard = row_for("forget_mode", "hard");
    assert_eq!(hard.support, "UNSUPPORTED");
    assert_eq!(hard.status, "degraded_non_complete");

    let tombstone = row_for("forget_mode", "tombstone");
    assert_eq!(tombstone.support, "DEGRADED");
    assert_eq!(tombstone.status, "degraded_non_complete");

    if postgres_enabled {
        for mode in ["soft", "hard", "tombstone"] {
            let row = rows
                .iter()
                .find(|row| {
                    row.backend == "postgres" && row.scenario == "forget_mode" && row.mode == mode
                })
                .unwrap_or_else(|| panic!("missing postgres forget row mode={mode}"));
            assert_eq!(row.support, "SUPPORTED");
            assert_eq!(row.status, "complete");
            assert!(row.applied);
            assert!(row.complete);
            assert!(!row.degraded);
        }
    }

    let report =
        fs::read_to_string(&report_path).expect("parity report should be readable after writing");
    assert_eq!(
        report.lines().count(),
        row_count + 1,
        "report should contain header + one line per row"
    );

    println!(
        "task19 parity report rows={row_count} path={}",
        report_path.display()
    );
}

#[test]
fn memory_backend_parity_detects_drift() {
    let simulated = asterel::core::memory::ForgetOutcome {
        entity_id: "drift".into(),
        slot_key: "drift.slot".into(),
        mode: ForgetMode::Soft,
        was_applied: true,
        is_complete: true,
        is_degraded: false,
        status: ForgetStatus::Complete,
        artifact_checks: Vec::new(),
    };

    let drift = ensure_explicit_contract(
        "markdown",
        ForgetMode::Soft,
        CapabilitySupport::Degraded,
        true,
        &simulated,
    )
    .expect_err("drift detector must fail loudly for undocumented behavior");

    let msg = drift.to_string();
    assert!(
        msg.contains("UNEXPECTED_DRIFT backend=markdown mode=soft support=degraded"),
        "drift detector should emit deterministic marker, got: {msg}"
    );
}

impl fmt::Display for ReportRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_csv_line())
    }
}
