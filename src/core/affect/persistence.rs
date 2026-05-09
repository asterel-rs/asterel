//! Persistence and trend analysis for the affect arc.
//!
//! Loads and saves the rolling window of affect readings to memory,
//! and computes trend direction, volatility, and dominant label.

use std::collections::HashMap;

use num_traits::ToPrimitive;

use super::types::{AffectArc, AffectLabel, AffectReading};
use super::{EmotionalMemory, SessionMood};
use crate::contracts::strings::data_model::{
    SLOT_PERSONA_AFFECT_ARC_V1, SLOT_PERSONA_EMOTIONAL_IDENTITY_V1,
    SLOT_PERSONA_EMOTIONAL_MEMORY_V1, SLOT_PERSONA_SESSION_MOOD_V1,
};
use crate::core::memory::{
    MemoryEventInput, MemoryEventType, MemoryProvenance, MemoryReader, MemorySource, MemoryWriter,
    PrivacyLevel, SourceKind,
};

/// Direction of the user's emotional trend over the recent window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrendDirection {
    /// Valence is trending upward (second half > first half + 0.1).
    Improving,
    /// Valence is roughly stable within the threshold.
    Stable,
    /// Valence is trending downward (second half < first half - 0.1).
    Declining,
}

/// Summary statistics of the affect arc's recent trend.
#[derive(Debug, Clone)]
pub(crate) struct AffectTrend {
    /// Whether the user's valence is improving, stable, or declining.
    pub direction: TrendDirection,
    /// Mean valence across all readings in the arc.
    pub avg_valence: f64,
    /// Standard deviation of valence (emotional volatility).
    pub volatility: f64,
    /// Most frequently occurring label in the arc.
    pub dominant_label: AffectLabel,
}

/// Load the persisted affect arc from memory, returning a fresh
/// arc if none exists.
///
/// # Errors
///
/// Returns an error if the memory read or JSON deserialization fails.
pub(crate) async fn load_affect_arc(
    mem: &(dyn MemoryReader + Sync),
    entity_id: &str,
) -> anyhow::Result<AffectArc> {
    let Some(slot) = mem
        .resolve_slot(entity_id, SLOT_PERSONA_AFFECT_ARC_V1)
        .await?
    else {
        return Ok(AffectArc::new());
    };
    Ok(serde_json::from_str(&slot.value)?)
}

/// Serialize and persist the affect arc to memory.
///
/// # Errors
///
/// Returns an error if JSON serialization or memory write fails.
pub(crate) async fn persist_affect_arc(
    mem: &(dyn MemoryWriter + Sync),
    entity_id: &str,
    arc: &AffectArc,
) -> anyhow::Result<()> {
    let input = MemoryEventInput::new(
        entity_id,
        SLOT_PERSONA_AFFECT_ARC_V1,
        MemoryEventType::FactUpdated,
        serde_json::to_string(arc)?,
        MemorySource::Inferred,
        PrivacyLevel::Private,
    )
    .with_confidence(0.8)
    .with_importance(0.4)
    .with_source_kind(SourceKind::Manual)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::Inferred,
        "affect.persistence.arc_update",
    ));
    mem.append_event(input).await?;
    Ok(())
}

/// Load the persisted session mood snapshot, if any.
pub(crate) async fn load_session_mood(
    mem: &(dyn MemoryReader + Sync),
    entity_id: &str,
) -> anyhow::Result<Option<SessionMood>> {
    let Some(slot) = mem
        .resolve_slot(entity_id, SLOT_PERSONA_SESSION_MOOD_V1)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_str(&slot.value)?))
}

/// Persist the current session mood snapshot.
pub(crate) async fn persist_session_mood(
    mem: &(dyn MemoryWriter + Sync),
    entity_id: &str,
    mood: &SessionMood,
) -> anyhow::Result<()> {
    let input = MemoryEventInput::new(
        entity_id,
        SLOT_PERSONA_SESSION_MOOD_V1,
        MemoryEventType::FactUpdated,
        serde_json::to_string(mood)?,
        MemorySource::Inferred,
        PrivacyLevel::Private,
    )
    .with_confidence(0.8)
    .with_importance(0.5)
    .with_source_kind(SourceKind::Manual)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::Inferred,
        "affect.persistence.session_mood",
    ));
    mem.append_event(input).await?;
    Ok(())
}

pub(crate) async fn load_emotional_memories(
    mem: &(dyn MemoryReader + Sync),
    entity_id: &str,
) -> anyhow::Result<Vec<EmotionalMemory>> {
    let Some(slot) = mem
        .resolve_slot(entity_id, SLOT_PERSONA_EMOTIONAL_MEMORY_V1)
        .await?
    else {
        return Ok(Vec::new());
    };
    Ok(serde_json::from_str(&slot.value)?)
}

pub(crate) async fn persist_emotional_memories(
    mem: &(dyn MemoryWriter + Sync),
    entity_id: &str,
    memories: &[EmotionalMemory],
) -> anyhow::Result<()> {
    let input = MemoryEventInput::new(
        entity_id,
        SLOT_PERSONA_EMOTIONAL_MEMORY_V1,
        MemoryEventType::FactUpdated,
        serde_json::to_string(memories)?,
        MemorySource::Inferred,
        PrivacyLevel::Private,
    )
    .with_confidence(0.8)
    .with_importance(0.55)
    .with_source_kind(SourceKind::Manual)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::Inferred,
        "affect.persistence.emotional_memory",
    ));
    mem.append_event(input).await?;
    Ok(())
}

pub(crate) async fn persist_promoted_emotional_memories(
    mem: &(dyn MemoryWriter + Sync),
    entity_id: &str,
    memories: &[EmotionalMemory],
) -> anyhow::Result<()> {
    let input = MemoryEventInput::new(
        entity_id,
        SLOT_PERSONA_EMOTIONAL_IDENTITY_V1,
        MemoryEventType::FactUpdated,
        serde_json::to_string(memories)?,
        MemorySource::Inferred,
        PrivacyLevel::Private,
    )
    .with_confidence(0.82)
    .with_importance(0.75)
    .with_source_kind(SourceKind::Manual)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::Inferred,
        "affect.persistence.emotional_identity",
    ));
    mem.append_event(input).await?;
    Ok(())
}

/// Compute trend direction, average valence, volatility, and dominant label
/// from the current affect arc.
///
/// Trend direction is determined by splitting the arc in half and comparing the
/// mean valence of the first half against the second half. A difference of > 0.1
/// in either direction constitutes a trend; within ±0.1 is considered stable.
///
/// Returns [`TrendDirection::Stable`] when fewer than two readings are present.
pub(crate) fn compute_affect_trend(arc: &AffectArc) -> AffectTrend {
    let readings = &arc.readings;
    let avg_valence = if readings.is_empty() {
        0.0
    } else {
        arc.valence_mean()
    };
    let volatility = arc.valence_std_dev();
    let dominant_label = dominant_label(readings);
    let half_avg = |slice: &[AffectReading]| -> f64 {
        if slice.is_empty() {
            return 0.0;
        }
        slice.iter().map(|r| r.valence).sum::<f64>() / slice.len().to_f64().unwrap_or(1.0)
    };
    let direction = if readings.len() < 2 {
        TrendDirection::Stable
    } else {
        let mid = readings.len() / 2;
        let (fst, snd) = (half_avg(&readings[..mid]), half_avg(&readings[mid..]));
        if snd > fst + 0.1 {
            TrendDirection::Improving
        } else if snd < fst - 0.1 {
            TrendDirection::Declining
        } else {
            TrendDirection::Stable
        }
    };
    AffectTrend {
        direction,
        avg_valence,
        volatility,
        dominant_label,
    }
}

fn dominant_label(readings: &[AffectReading]) -> AffectLabel {
    if readings.is_empty() {
        return AffectLabel::Neutral;
    }
    let mut counts: HashMap<AffectLabel, usize> = HashMap::new();
    for r in readings {
        *counts.entry(r.label).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, c)| c)
        .map_or(AffectLabel::Neutral, |(l, _)| l)
}

#[cfg(test)]
mod tests {
    use super::{TrendDirection, compute_affect_trend};
    use crate::contracts::scores::Confidence;
    use crate::core::affect::{AffectArc, AffectLabel, AffectReading};

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    fn r(label: AffectLabel, valence: f64) -> AffectReading {
        AffectReading {
            label,
            valence,
            arousal: 0.5,
            dominance: 0.5,
            confidence: Confidence::new(0.9),
        }
    }

    fn arc_from(pairs: &[(AffectLabel, f64)]) -> AffectArc {
        let mut arc = AffectArc::new();
        for &(label, v) in pairs {
            arc.readings.push(r(label, v));
        }
        if let Some(&(l, _)) = pairs.last() {
            arc.current_label = l;
        }
        arc
    }

    #[test]
    fn compute_trend_covers_all_directions_and_fields() {
        use AffectLabel::{Excited, Frustrated, Neutral, Sad};
        let t = compute_affect_trend(&AffectArc::new());
        assert_eq!(t.direction, TrendDirection::Stable);
        assert_eq!(t.dominant_label, Neutral);
        assert_f64_eq(t.avg_valence, 0.0);

        let up = arc_from(&[(Sad, -0.5), (Sad, -0.4), (Neutral, 0.2), (Excited, 0.5)]);
        assert_eq!(
            compute_affect_trend(&up).direction,
            TrendDirection::Improving
        );
        let dn = arc_from(&[(Excited, 0.6), (Excited, 0.5), (Sad, -0.3), (Sad, -0.4)]);
        assert_eq!(
            compute_affect_trend(&dn).direction,
            TrendDirection::Declining
        );
        let fl = arc_from(&[(Neutral, 0.0), (Neutral, 0.05)]);
        assert_eq!(compute_affect_trend(&fl).direction, TrendDirection::Stable);
        let fr = arc_from(&[(Frustrated, -0.3), (Frustrated, -0.4), (Neutral, 0.0)]);
        assert_eq!(compute_affect_trend(&fr).dominant_label, Frustrated);
        let v = arc_from(&[(Sad, -0.8), (Excited, 0.8)]);
        assert!(compute_affect_trend(&v).volatility > 0.0);
    }
}
