//! Compaction context injector: captures companion state before compaction
//! and rehydrates it afterwards (WP-H1).
//!
//! Without this, affect topology activation, session control state, and
//! working memory keys are lost when the context window is flushed.
//! This module ensures the companion's "current mood and conversational
//! awareness" survive compaction.
//!
//! Design source: ecosystem survey 2026-04-03 (oh-my-openagent
//! `CompactionContextInjector`, Moltis silent memory turn,
//! claw-code-parity session fork metadata).

use std::fmt::Write;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::types::{
    SESSION_COMPANION_AFFECT_SCHEMA_VERSION, SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY,
    SessionCompanionAffectState,
};
use crate::contracts::session_control::SessionControlState;

const COMPANION_AFFECT_MAX_AGE_MINUTES: i64 = 120;
const COMPANION_AFFECT_FUTURE_SKEW_MINUTES: i64 = 5;

/// Snapshot of companion state captured before compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompanionStateSnapshot {
    /// Session control: conversation mode, density, avoidance.
    #[serde(default)]
    pub session_control: Option<SessionControlState>,
    /// Top surfaced affect nodes (name + intensity).
    #[serde(default)]
    pub affect_surface: Vec<(String, f32)>,
    /// Suppressed affect nodes (name + internal intensity).
    #[serde(default)]
    pub affect_suppressed: Vec<(String, f32)>,
    /// Compaction generation counter (increments each compaction).
    #[serde(default)]
    pub generation: u32,
}

impl CompanionStateSnapshot {
    /// Create an empty snapshot (no companion state available).
    #[must_use]
    // Wired (P-5): used as fallback in orchestrator::maybe_compact_session.
    pub(crate) fn empty(generation: u32) -> Self {
        Self {
            session_control: None,
            affect_surface: Vec::new(),
            affect_suppressed: Vec::new(),
            generation,
        }
    }

    /// Build a snapshot from persisted session state and the current transcript.
    #[must_use]
    pub(crate) fn from_session_context(
        session_control: Option<SessionControlState>,
        companion_affect: Option<SessionCompanionAffectState>,
        generation: u32,
    ) -> Self {
        Self::from_session_context_at(session_control, companion_affect, generation, Utc::now())
    }

    #[must_use]
    pub(crate) fn from_session_context_at(
        session_control: Option<SessionControlState>,
        companion_affect: Option<SessionCompanionAffectState>,
        generation: u32,
        now: DateTime<Utc>,
    ) -> Self {
        let (affect_surface, affect_suppressed) = companion_affect.map_or_else(
            || (Vec::new(), Vec::new()),
            |state| {
                if is_current_topology_affect_state(&state, now) {
                    (
                        decode_affect_cues(state.affect_surface),
                        decode_affect_cues(state.affect_suppressed),
                    )
                } else {
                    (Vec::new(), Vec::new())
                }
            },
        );
        Self {
            session_control,
            affect_surface,
            affect_suppressed,
            generation,
        }
    }

    /// Whether this snapshot contains any non-trivial state.
    #[must_use]
    pub(crate) fn has_content(&self) -> bool {
        self.session_control.is_some()
            || !self.affect_surface.is_empty()
            || !self.affect_suppressed.is_empty()
    }
}

fn is_current_topology_affect_state(
    state: &SessionCompanionAffectState,
    now: DateTime<Utc>,
) -> bool {
    if state.schema_version != SESSION_COMPANION_AFFECT_SCHEMA_VERSION {
        return false;
    }
    if state.source.as_deref() != Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY) {
        return false;
    }
    let Some(captured_at) = state.captured_at.as_deref() else {
        return false;
    };
    let Ok(captured_at) = DateTime::parse_from_rfc3339(captured_at) else {
        return false;
    };
    let age = now.signed_duration_since(captured_at.with_timezone(&Utc));
    if age < -Duration::minutes(COMPANION_AFFECT_FUTURE_SKEW_MINUTES) {
        return false;
    }
    if let Some(expires_at) = state.expires_at.as_deref() {
        let Ok(expires_at) = DateTime::parse_from_rfc3339(expires_at) else {
            return false;
        };
        return now < expires_at.with_timezone(&Utc);
    }
    age <= Duration::minutes(COMPANION_AFFECT_MAX_AGE_MINUTES)
}

fn decode_affect_cues(cues: Vec<(String, u16)>) -> Vec<(String, f32)> {
    cues.into_iter()
        .map(|(name, intensity_per_mille)| {
            (name, f32::from(intensity_per_mille.min(1_000)) / 1_000.0)
        })
        .collect()
}

/// Render a companion state snapshot as a rehydration message.
///
/// This is injected after the compaction summary to restore the
/// companion's awareness of its current emotional/conversational state.
#[must_use]
pub(crate) fn render_rehydration_block(snapshot: &CompanionStateSnapshot) -> String {
    if !snapshot.has_content() {
        return String::new();
    }

    let mut out = String::with_capacity(512);
    out.push_str("## Companion State (restored after compaction)\n\n");

    let _ = writeln!(out, "Compaction generation: {}\n", snapshot.generation);

    if let Some(ref control) = snapshot.session_control {
        let mode = match control.mode {
            crate::contracts::session_control::ConversationMode::Chitchat => "chitchat",
            crate::contracts::session_control::ConversationMode::Empathy => "empathy",
            crate::contracts::session_control::ConversationMode::Task => "task",
            crate::contracts::session_control::ConversationMode::DeepDive => "deep_dive",
        };
        let density = match control.density {
            crate::contracts::session_control::ExpectedDensity::Brief => "brief",
            crate::contracts::session_control::ExpectedDensity::Normal => "normal",
            crate::contracts::session_control::ExpectedDensity::Expanded => "expanded",
        };
        let _ = writeln!(out, "Conversation mode: {mode} (density: {density})");
        if !control.avoid.is_empty() {
            let mut avoidances = String::new();
            for a in &control.avoid {
                if !avoidances.is_empty() {
                    avoidances.push_str(", ");
                }
                avoidances.push_str(match a {
                    crate::contracts::session_control::AvoidBehavior::Overexplain => {
                        "over-explaining"
                    }
                    crate::contracts::session_control::AvoidBehavior::Preachy => "preachy tone",
                    crate::contracts::session_control::AvoidBehavior::SuddenOrganize => {
                        "sudden organizing"
                    }
                    crate::contracts::session_control::AvoidBehavior::AnalysisBeforeEmpathy => {
                        "analysis before empathy"
                    }
                });
            }
            let _ = writeln!(out, "Avoid: {avoidances}");
        }
        out.push('\n');
    }

    if !snapshot.affect_surface.is_empty() {
        let mut affect_str = String::new();
        for (name, intensity) in &snapshot.affect_surface {
            if !affect_str.is_empty() {
                affect_str.push_str(", ");
            }
            let _ = write!(affect_str, "{name} ({:.0}%)", intensity * 100.0);
        }
        let _ = writeln!(out, "Affect surface: {affect_str}");
    }

    if !snapshot.affect_suppressed.is_empty() {
        let mut suppressed_str = String::new();
        for (name, intensity) in &snapshot.affect_suppressed {
            if !suppressed_str.is_empty() {
                suppressed_str.push_str(", ");
            }
            let _ = write!(
                suppressed_str,
                "{name} ({:.0}% internal)",
                intensity * 100.0
            );
        }
        let _ = writeln!(out, "Affect held back: {suppressed_str}");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::session_control::{
        AvoidBehavior, ConversationMode, ExpectedDensity, SessionControlState,
    };

    fn fresh_affect_state() -> SessionCompanionAffectState {
        SessionCompanionAffectState {
            schema_version: SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
            source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
            captured_at: Some("2026-01-01T00:00:00Z".to_string()),
            expires_at: Some("2026-01-01T02:00:00Z".to_string()),
            affect_surface: vec![("attachment".to_string(), 620)],
            affect_suppressed: vec![("guardedness".to_string(), 310)],
        }
    }

    #[test]
    fn empty_snapshot_has_no_content() {
        let snapshot = CompanionStateSnapshot::empty(0);
        assert!(!snapshot.has_content());
        assert!(render_rehydration_block(&snapshot).is_empty());
    }

    #[test]
    fn snapshot_with_session_control_renders() {
        let snapshot = CompanionStateSnapshot {
            session_control: Some(SessionControlState {
                mode: ConversationMode::Empathy,
                density: ExpectedDensity::Brief,
                avoid: vec![AvoidBehavior::AnalysisBeforeEmpathy],
                mode_turns: 5,
            }),
            affect_surface: vec![],
            affect_suppressed: vec![],
            generation: 3,
        };
        let block = render_rehydration_block(&snapshot);
        assert!(block.contains("empathy"));
        assert!(block.contains("brief"));
        assert!(block.contains("analysis before empathy"));
        assert!(block.contains("generation: 3"));
    }

    #[test]
    fn snapshot_with_affect_renders() {
        let snapshot = CompanionStateSnapshot {
            session_control: None,
            affect_surface: vec![("joy".to_string(), 0.6), ("curiosity".to_string(), 0.4)],
            affect_suppressed: vec![("anxiety".to_string(), 0.3)],
            generation: 1,
        };
        let block = render_rehydration_block(&snapshot);
        assert!(block.contains("joy (60%)"));
        assert!(block.contains("curiosity (40%)"));
        assert!(block.contains("anxiety (30% internal)"));
    }

    #[test]
    fn snapshot_from_session_context_preserves_persisted_topology_affect() {
        let snapshot = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(fresh_affect_state()),
            4,
            DateTime::parse_from_rfc3339("2026-01-01T00:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        assert_eq!(snapshot.generation, 4);
        assert!(snapshot.session_control.is_none());
        assert_eq!(
            snapshot.affect_surface,
            vec![("attachment".to_string(), 0.62)]
        );
        assert_eq!(
            snapshot.affect_suppressed,
            vec![("guardedness".to_string(), 0.31)]
        );
    }

    #[test]
    fn snapshot_from_session_context_rejects_stale_or_unversioned_affect() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let stale = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(fresh_affect_state()),
            0,
            now,
        );
        assert!(stale.affect_surface.is_empty());

        let unversioned = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(SessionCompanionAffectState {
                schema_version: 0,
                source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
                captured_at: Some("2026-01-01T00:00:00Z".to_string()),
                expires_at: Some("2026-01-01T02:00:00Z".to_string()),
                affect_surface: vec![("attachment".to_string(), 620)],
                affect_suppressed: Vec::new(),
            }),
            0,
            DateTime::parse_from_rfc3339("2026-01-01T00:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        assert!(unversioned.affect_surface.is_empty());
    }

    #[test]
    fn snapshot_from_session_context_rejects_expired_affect() {
        let snapshot = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(SessionCompanionAffectState {
                schema_version: SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
                source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
                captured_at: Some("2026-01-01T00:00:00Z".to_string()),
                expires_at: Some("2026-01-01T00:10:00Z".to_string()),
                affect_surface: vec![("attachment".to_string(), 620)],
                affect_suppressed: Vec::new(),
            }),
            0,
            DateTime::parse_from_rfc3339("2026-01-01T00:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        assert!(snapshot.affect_surface.is_empty());
        assert!(!snapshot.has_content());
    }

    #[test]
    fn snapshot_from_session_context_rejects_at_exact_expiry_boundary() {
        let snapshot = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(SessionCompanionAffectState {
                schema_version: SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
                source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
                captured_at: Some("2026-01-01T00:00:00Z".to_string()),
                expires_at: Some("2026-01-01T00:10:00Z".to_string()),
                affect_surface: vec![("attachment".to_string(), 620)],
                affect_suppressed: Vec::new(),
            }),
            0,
            DateTime::parse_from_rfc3339("2026-01-01T00:10:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        assert!(snapshot.affect_surface.is_empty());
    }

    #[test]
    fn snapshot_from_session_context_rejects_malformed_expiry() {
        let snapshot = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(SessionCompanionAffectState {
                schema_version: SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
                source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
                captured_at: Some("2026-01-01T00:00:00Z".to_string()),
                expires_at: Some("not a timestamp".to_string()),
                affect_surface: vec![("attachment".to_string(), 620)],
                affect_suppressed: Vec::new(),
            }),
            0,
            DateTime::parse_from_rfc3339("2026-01-01T00:05:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        assert!(snapshot.affect_surface.is_empty());
    }

    #[test]
    fn snapshot_from_session_context_accepts_legacy_state_without_expiry_by_age() {
        let snapshot = CompanionStateSnapshot::from_session_context_at(
            None,
            Some(SessionCompanionAffectState {
                schema_version: SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
                source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
                captured_at: Some("2026-01-01T00:00:00Z".to_string()),
                expires_at: None,
                affect_surface: vec![("attachment".to_string(), 620)],
                affect_suppressed: Vec::new(),
            }),
            0,
            DateTime::parse_from_rfc3339("2026-01-01T00:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        assert_eq!(
            snapshot.affect_surface,
            vec![("attachment".to_string(), 0.62)]
        );
    }

    #[test]
    fn snapshot_from_session_context_stays_empty_without_persisted_affect() {
        let snapshot = CompanionStateSnapshot::from_session_context(None, None, 0);

        assert!(snapshot.affect_surface.is_empty());
        assert!(!snapshot.has_content());
    }

    #[test]
    fn full_snapshot_roundtrips_via_json() {
        let snapshot = CompanionStateSnapshot {
            session_control: Some(SessionControlState {
                mode: ConversationMode::Task,
                density: ExpectedDensity::Normal,
                avoid: vec![AvoidBehavior::Overexplain],
                mode_turns: 2,
            }),
            affect_surface: vec![("guardedness".to_string(), 0.5)],
            affect_suppressed: vec![],
            generation: 7,
        };
        let json = serde_json::to_string(&snapshot).expect("serialize");
        let loaded: CompanionStateSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(loaded.generation, 7);
        let ctrl = loaded.session_control.as_ref().expect("session_control");
        assert_eq!(ctrl.mode, ConversationMode::Task);
        assert_eq!(ctrl.density, ExpectedDensity::Normal);
        assert_eq!(ctrl.avoid.len(), 1);
        assert_eq!(loaded.affect_surface.len(), 1);
        assert_eq!(loaded.affect_surface[0].0, "guardedness");
        assert!((loaded.affect_surface[0].1 - 0.5).abs() < f32::EPSILON);
    }
}
