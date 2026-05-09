//! User value profile: learns and persists per-user value
//! preferences (brevity, detail, caution, autonomy, etc.) via
//! EMA-smoothed signal accumulation across turns.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::value_signals::ValueSignal;
use crate::contracts::person_identity::{person_entity_id, sanitize_person_id};
use crate::contracts::strings::data_model::SOURCE_REF_TASTE_VALUE_PROFILE_UPDATE;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};

/// EMA smoothing factor (higher = more weight on recent signals).
const EMA_ALPHA: f64 = 0.15;

/// A profile of learned user value preferences, updated via EMA.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ValueProfile {
    /// Signal → strength mapping in `[0.0, 1.0]`.
    pub scores: HashMap<ValueSignal, f64>,
}

impl ValueProfile {
    /// Update the profile with new signals from a turn.
    pub(crate) fn update(&mut self, signals: &[ValueSignal]) {
        // Reinforce observed signals.
        for signal in signals {
            let current = self.scores.get(signal).copied().unwrap_or(0.0);
            let updated = current * (1.0 - EMA_ALPHA) + EMA_ALPHA;
            self.scores.insert(*signal, updated.min(1.0));
        }

        // Decay unobserved signals slightly.
        let observed: std::collections::HashSet<&ValueSignal> = signals.iter().collect();
        for (signal, score) in &mut self.scores {
            if !observed.contains(signal) {
                *score *= 1.0 - EMA_ALPHA * 0.3;
            }
        }
    }

    /// Get the strength of a particular value signal.
    pub(crate) fn strength(&self, signal: ValueSignal) -> f64 {
        self.scores.get(&signal).copied().unwrap_or(0.0)
    }
}

fn value_profile_slot_key(person_id: &str) -> String {
    format!("persona/{}/value_profile/v1", sanitize_person_id(person_id))
}

/// Load a value profile from memory.
///
/// # Errors
///
/// Returns an error if the memory lookup or deserialisation fails.
pub(crate) async fn load_value_profile(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<ValueProfile>> {
    let entity_id = person_entity_id(person_id);
    let slot_key = value_profile_slot_key(person_id);
    let Some(slot) = mem.resolve_slot(&entity_id, &slot_key).await? else {
        return Ok(None);
    };
    let profile: ValueProfile = serde_json::from_str(&slot.value)?;
    Ok(Some(profile))
}

/// Persist a value profile to memory.
///
/// # Errors
///
/// Returns an error if serialisation or the memory append fails.
pub(crate) async fn persist_value_profile(
    mem: &dyn Memory,
    person_id: &str,
    profile: &ValueProfile,
) -> Result<()> {
    let entity_id = person_entity_id(person_id);
    let slot_key = value_profile_slot_key(person_id);
    let payload = serde_json::to_string(profile)?;

    let input = MemoryEventInput::new(
        entity_id,
        slot_key,
        MemoryEventType::FactUpdated,
        payload,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(0.8)
    .with_importance(0.5)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(SOURCE_REF_TASTE_VALUE_PROFILE_UPDATE)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        "taste.value_profile.ema",
    ));

    mem.append_event(input).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::taste::presenter::render_value_guidance;

    use super::*;

    #[test]
    fn update_reinforces_observed_signals() {
        let mut profile = ValueProfile::default();
        profile.update(&[ValueSignal::PrefersBrevity]);
        assert!(profile.strength(ValueSignal::PrefersBrevity) > 0.0);
    }

    #[test]
    fn update_decays_unobserved_signals() {
        let mut profile = ValueProfile::default();
        profile.scores.insert(ValueSignal::PrefersDetail, 0.8);
        profile.update(&[ValueSignal::PrefersBrevity]);
        assert!(profile.strength(ValueSignal::PrefersDetail) < 0.8);
    }

    #[test]
    fn render_empty_profile_produces_no_block() {
        let profile = ValueProfile::default();
        assert!(render_value_guidance(&profile).is_empty());
    }

    #[test]
    fn render_active_values_produces_block() {
        let mut profile = ValueProfile::default();
        profile.scores.insert(ValueSignal::PrefersBrevity, 0.7);
        profile.scores.insert(ValueSignal::PrefersCaution, 0.5);
        let block = render_value_guidance(&profile);
        assert!(block.contains("[Value Guidance]"));
        assert!(block.contains("concise"));
    }

    #[test]
    fn ema_converges_with_repeated_signals() {
        let mut profile = ValueProfile::default();
        for _ in 0..20 {
            profile.update(&[ValueSignal::PrefersStructure]);
        }
        assert!(profile.strength(ValueSignal::PrefersStructure) > 0.5);
    }
}
