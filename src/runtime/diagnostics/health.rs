//! Global health registry for runtime components.
//!
//! Tracks per-component status (ok/error), last-ok timestamps, and
//! restart counts. Provides a JSON-serializable health snapshot.

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::security::scrub::sanitize_api_error;

/// Default freshness window for readiness-critical component heartbeats.
pub const DEFAULT_READINESS_MAX_LAST_OK_AGE_SECONDS: i64 = 300;

/// Health state for a single runtime component.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentHealth {
    /// Current status label (e.g. "ok", "error", "starting").
    pub status: String,
    /// RFC 3339 timestamp of the last status update.
    pub updated_at: String,
    /// RFC 3339 timestamp of the last successful health check.
    pub last_ok: Option<String>,
    /// Human-readable description of the last error, if any.
    pub last_error: Option<String>,
    /// Number of times this component has been restarted.
    pub restart_count: u64,
}

/// Point-in-time snapshot of the runtime's overall health.
#[derive(Debug, Clone, Serialize)]
pub struct HealthSnapshot {
    /// OS process ID of the daemon.
    pub pid: u32,
    /// RFC 3339 timestamp when this snapshot was taken.
    pub updated_at: String,
    /// Seconds since the daemon process started.
    pub uptime_seconds: u64,
    /// Per-component health keyed by component name.
    pub components: BTreeMap<String, ComponentHealth>,
}

/// Readiness view derived from the current health registry.
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessSnapshot {
    /// Whether all required components currently report `ok`.
    pub ready: bool,
    /// Component names checked for readiness.
    pub required_components: Vec<String>,
    /// Required components that are missing or not `ok`.
    pub failing_components: Vec<String>,
    /// Underlying point-in-time runtime health snapshot.
    pub runtime: HealthSnapshot,
}

struct HealthRegistry {
    started_at: Instant,
    components: RwLock<BTreeMap<String, ComponentHealth>>,
}

static REGISTRY: OnceLock<HealthRegistry> = OnceLock::new();

fn registry() -> &'static HealthRegistry {
    REGISTRY.get_or_init(|| HealthRegistry {
        started_at: Instant::now(),
        components: RwLock::new(BTreeMap::new()),
    })
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn upsert_component<F>(component: &str, update: F)
where
    F: FnOnce(&mut ComponentHealth),
{
    if let Ok(mut map) = registry().components.write() {
        let now = now_rfc3339();
        let entry = map
            .entry(component.to_string())
            .or_insert_with(|| ComponentHealth {
                status: "starting".into(),
                updated_at: now.clone(),
                last_ok: None,
                last_error: None,
                restart_count: 0,
            });
        update(entry);
        entry.updated_at = now;
    }
}

/// Record a healthy status for the named component.
pub fn mark_component_ok(component: &str) {
    upsert_component(component, |entry| {
        entry.status = "ok".into();
        entry.last_ok = Some(now_rfc3339());
        entry.last_error = None;
    });
}

/// Record an error status for the named component.
#[allow(clippy::needless_pass_by_value)] // impl ToString consumed immediately; by-value is idiomatic
pub fn mark_component_error(component: &str, error: impl ToString) {
    let err = sanitize_api_error(&error.to_string());
    upsert_component(component, move |entry| {
        entry.status = "error".into();
        entry.last_error = Some(err);
    });
}

/// Increment the restart counter for the named component.
pub fn bump_component_restart(component: &str) {
    upsert_component(component, |entry| {
        entry.restart_count = entry.restart_count.saturating_add(1);
    });
}

/// Capture a point-in-time health snapshot of all components.
#[must_use]
pub fn snapshot() -> HealthSnapshot {
    let components = registry()
        .components
        .read()
        .map_or_else(|_| BTreeMap::new(), |map| map.clone());

    HealthSnapshot {
        pid: std::process::id(),
        updated_at: now_rfc3339(),
        uptime_seconds: registry().started_at.elapsed().as_secs(),
        components,
    }
}

/// Capture a health snapshot and serialize it to a JSON value.
#[must_use]
pub fn snapshot_json() -> serde_json::Value {
    serde_json::to_value(snapshot()).unwrap_or_else(|_| {
        serde_json::json!({
            "status": "error",
            "message": "failed to serialize health snapshot"
        })
    })
}

/// Return whether a component is both healthy and fresh enough for readiness.
#[must_use]
pub fn component_is_ready(health: &ComponentHealth) -> bool {
    if health.status != "ok" {
        return false;
    }
    let Some(last_ok) = health.last_ok.as_deref() else {
        return false;
    };
    let Ok(last_ok) = DateTime::parse_from_rfc3339(last_ok) else {
        return false;
    };
    let age_seconds = Utc::now()
        .signed_duration_since(last_ok.with_timezone(&Utc))
        .num_seconds();
    (0..=DEFAULT_READINESS_MAX_LAST_OK_AGE_SECONDS).contains(&age_seconds)
}

/// Capture a readiness snapshot based on required component health.
#[must_use]
pub fn readiness_snapshot(required_components: &[&str]) -> ReadinessSnapshot {
    let runtime = snapshot();
    let required_components = required_components
        .iter()
        .map(|component| (*component).to_string())
        .collect::<Vec<_>>();
    let failing_components = required_components
        .iter()
        .filter(|component| {
            runtime
                .components
                .get(component.as_str())
                .is_none_or(|health| !component_is_ready(health))
        })
        .cloned()
        .collect::<Vec<_>>();

    ReadinessSnapshot {
        ready: failing_components.is_empty(),
        required_components,
        failing_components,
        runtime,
    }
}

/// Capture a readiness snapshot and serialize it to JSON.
#[must_use]
pub fn readiness_json(required_components: &[&str]) -> serde_json::Value {
    serde_json::to_value(readiness_snapshot(required_components)).unwrap_or_else(|_| {
        serde_json::json!({
            "ready": false,
            "message": "failed to serialize readiness snapshot"
        })
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use chrono::Duration;

    use super::*;

    fn unique_component(prefix: &str) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{id}")
    }

    #[test]
    fn mark_component_ok_sets_ok_state() {
        let component = unique_component("health-ok");
        mark_component_ok(&component);

        let snap = snapshot();
        let state = snap
            .components
            .get(&component)
            .expect("component should exist in snapshot");

        assert_eq!(state.status, "ok");
        assert!(state.last_ok.is_some());
        assert_eq!(state.last_error, None);
    }

    #[test]
    fn mark_component_error_sets_error_and_preserves_last_ok() {
        let component = unique_component("health-error");
        mark_component_ok(&component);
        mark_component_error(&component, "boom");

        let snap = snapshot();
        let state = snap
            .components
            .get(&component)
            .expect("component should exist in snapshot");

        assert_eq!(state.status, "error");
        assert_eq!(state.last_error.as_deref(), Some("boom"));
        assert!(state.last_ok.is_some());
    }

    #[test]
    fn mark_component_error_sanitizes_secret_like_error_text() {
        let component = unique_component("health-error-sanitized");
        mark_component_error(
            &component,
            "provider echoed sk-leaked-secret-token in diagnostic body",
        );

        let snap = snapshot();
        let state = snap
            .components
            .get(&component)
            .expect("component should exist in snapshot");
        let last_error = state
            .last_error
            .as_deref()
            .expect("last_error should be set");

        assert!(!last_error.contains("sk-leaked-secret-token"));
        assert!(last_error.contains("[REDACTED]"));
    }

    #[test]
    fn bump_component_restart_increments_counter() {
        let component = unique_component("health-restart");
        bump_component_restart(&component);
        bump_component_restart(&component);

        let snap = snapshot();
        let state = snap
            .components
            .get(&component)
            .expect("component should exist in snapshot");

        assert_eq!(state.restart_count, 2);
    }

    #[test]
    fn snapshot_json_includes_component_data() {
        let component = unique_component("health-json");
        mark_component_ok(&component);

        let json = snapshot_json();
        let status = json
            .get("components")
            .and_then(|components| components.get(&component))
            .and_then(|entry| entry.get("status"))
            .and_then(serde_json::Value::as_str);

        assert_eq!(status, Some("ok"));
    }

    #[test]
    fn readiness_snapshot_requires_ok_components() {
        let ok_component = unique_component("health-ready-ok");
        let error_component = unique_component("health-ready-error");
        mark_component_ok(&ok_component);
        mark_component_error(&error_component, "degraded");

        let readiness = readiness_snapshot(&[ok_component.as_str(), error_component.as_str()]);

        assert!(!readiness.ready);
        assert_eq!(readiness.required_components.len(), 2);
        assert_eq!(readiness.failing_components, vec![error_component]);
    }

    #[test]
    fn readiness_snapshot_rejects_stale_ok_components() {
        let component = unique_component("health-ready-stale");
        upsert_component(&component, |entry| {
            entry.status = "ok".to_string();
            entry.last_ok = Some(
                (Utc::now() - Duration::seconds(DEFAULT_READINESS_MAX_LAST_OK_AGE_SECONDS + 1))
                    .to_rfc3339(),
            );
            entry.last_error = None;
        });

        let readiness = readiness_snapshot(&[component.as_str()]);

        assert!(!readiness.ready);
        assert_eq!(readiness.failing_components, vec![component]);
    }

    #[test]
    fn readiness_snapshot_is_ready_when_required_components_are_ok() {
        let component = unique_component("health-ready");
        mark_component_ok(&component);

        let readiness = readiness_snapshot(&[component.as_str()]);

        assert!(readiness.ready);
        assert!(readiness.failing_components.is_empty());
    }
}
