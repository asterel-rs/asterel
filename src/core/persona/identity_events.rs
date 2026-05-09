//! Identity event emission: records identity transitions as memory events.
//!
//! When the agent's identity state changes (objective, commitments, drift),
//! these functions create `MemoryEventInput` payloads for the memory ledger.

use crate::core::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemorySource, PrivacyLevel,
};

#[derive(serde::Serialize)]
struct ObjectiveChangedPayload<'a> {
    old: &'a str,
    new: &'a str,
}

#[derive(serde::Serialize)]
struct DriftDetectedPayload<'a> {
    description: &'a str,
    severity: f64,
}

#[must_use]
pub fn build_objective_changed_event(
    entity_id: &str,
    old_objective: &str,
    new_objective: &str,
) -> MemoryEventInput {
    MemoryEventInput::new(
        entity_id,
        "identity.objective",
        MemoryEventType::IdentityObjectiveChanged,
        serde_json::to_string(&ObjectiveChangedPayload {
            old: old_objective,
            new: new_objective,
        })
        .unwrap_or_default(),
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Identity)
    .with_importance(0.7)
}

#[must_use]
pub fn build_commitment_added_event(entity_id: &str, commitment: &str) -> MemoryEventInput {
    MemoryEventInput::new(
        entity_id,
        "identity.commitment",
        MemoryEventType::IdentityCommitmentAdded,
        commitment,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Identity)
    .with_importance(0.6)
}

#[must_use]
pub fn build_commitment_completed_event(entity_id: &str, commitment: &str) -> MemoryEventInput {
    MemoryEventInput::new(
        entity_id,
        "identity.commitment",
        MemoryEventType::IdentityCommitmentCompleted,
        commitment,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Identity)
    .with_importance(0.5)
}

#[must_use]
// Wired (P-4): called from reflect.rs check_continuity_gate() on Warning/Critical severity.
pub fn build_drift_detected_event(
    entity_id: &str,
    description: &str,
    severity: f64,
) -> MemoryEventInput {
    let severity = sanitize_unit_interval(severity);
    MemoryEventInput::new(
        entity_id,
        "identity.drift",
        MemoryEventType::IdentityDriftDetected,
        serde_json::to_string(&DriftDetectedPayload {
            description,
            severity,
        })
        .unwrap_or_default(),
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Identity)
    .with_importance(0.9)
}

fn sanitize_unit_interval(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objective_changed_event_has_identity_layer() {
        let event = build_objective_changed_event("agent-1", "old", "new");
        assert_eq!(event.layer, MemoryLayer::Identity);
    }

    #[test]
    fn objective_changed_event_contains_old_and_new() {
        let event = build_objective_changed_event("agent-1", "old objective", "new objective");
        let parsed: serde_json::Value = serde_json::from_str(&event.value).unwrap();

        assert_eq!(parsed["old"], "old objective");
        assert_eq!(parsed["new"], "new objective");
    }

    #[test]
    fn commitment_added_event_has_correct_slot_key() {
        let event = build_commitment_added_event("agent-1", "Keep promises");
        assert_eq!(event.slot_key.as_str(), "identity.commitment");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn drift_detected_event_has_high_importance() {
        let event = build_drift_detected_event("agent-1", "voice changed", 0.8);
        assert_eq!(event.importance.get(), 0.9);
    }

    #[test]
    fn drift_detected_event_sanitizes_non_finite_severity() {
        let nan_event = build_drift_detected_event("agent-1", "voice changed", f64::NAN);
        let nan_payload: serde_json::Value = serde_json::from_str(&nan_event.value).unwrap();
        assert_eq!(nan_payload["severity"], 0.0);

        let inf_event = build_drift_detected_event("agent-1", "voice changed", f64::INFINITY);
        let inf_payload: serde_json::Value = serde_json::from_str(&inf_event.value).unwrap();
        assert_eq!(inf_payload["severity"], 1.0);
    }

    #[test]
    fn all_identity_events_use_system_source() {
        let objective = build_objective_changed_event("agent-1", "old", "new");
        let added = build_commitment_added_event("agent-1", "Keep promises");
        let completed = build_commitment_completed_event("agent-1", "Keep promises");
        let drift = build_drift_detected_event("agent-1", "voice changed", 0.8);

        for event in [objective, added, completed, drift] {
            assert_eq!(event.source, MemorySource::System);
        }
    }
}
