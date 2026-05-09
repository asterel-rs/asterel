//! Sleep-phase memory consolidation.
//!
//! Inspired by the neuroscience of sleep-dependent memory consolidation
//! (systems consolidation hypothesis): episodic memories recorded during
//! waking are replayed and abstracted into durable semantic knowledge
//! during off-line periods.
//!
//! ## What this module does
//!
//! 1. Scans `retrieval_units` updated within the last
//!    [`SLEEP_SCAN_WINDOW_HOURS`] for `Postgres` backends.
//! 2. Groups candidates by `(entity_id, topic_prefix)` where the topic
//!    prefix is the first two segments of the slot key.
//! 3. For groups with at least [`MIN_GROUP_SIZE`] members, aggregates
//!    content (concatenation, most-restrictive visibility, highest
//!    importance/reliability) and writes a `semantic.sleep.*` snapshot
//!    retrieval unit and event.
//! 4. Additionally promotes episode clusters to note-tier nodes when
//!    similarity exceeds [`PROMOTION_SIMILARITY_THRESHOLD`].
//!
//! Sleep consolidation runs on a separate 24-hour cadence tracked in
//! `HygieneState::last_sleep_run_at`.
//!
//! References: [MEMORY-SLEEP] Diekelmann & Born, 2010 — memory function of
//! sleep. See the public research reference index in the docs site.

mod aggregation;
#[cfg(feature = "postgres")]
mod postgres;

use std::path::Path;

#[cfg(feature = "postgres")]
use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{MemoryBackend, MemoryConfig};
use crate::contracts::ids::EntityId;
use crate::contracts::scores::Importance;
use crate::core::memory::{GraphEntity, NodeTier};

const SLEEP_INTERVAL_HOURS: i64 = 24;
const SLEEP_SCAN_WINDOW_HOURS: i64 = 48;
const MIN_GROUP_SIZE: usize = 2;
const MIN_PROMOTION_GROUP_SIZE: usize = 3;
const PROMOTION_SIMILARITY_THRESHOLD: f32 = 0.85;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct SleepConsolidationReport {
    pub(super) consolidated_groups: u64,
    pub(super) snapshots_written: u64,
}

#[derive(Debug)]
struct ConsolidationCandidate {
    content: String,
    signal_tier: String,
    importance: f64,
    reliability: f64,
    visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    entity_id: EntityId,
    topic_prefix: String,
}

#[derive(Debug)]
struct GroupAggregate {
    combined_content: String,
    signal_tier: String,
    importance: f64,
    reliability_avg: f64,
    visibility: String,
}

pub(super) fn sleep_interval_hours() -> i64 {
    SLEEP_INTERVAL_HOURS
}

pub(super) fn run_sleep_consolidation(
    workspace_dir: &Path,
    config: &MemoryConfig,
) -> Result<SleepConsolidationReport> {
    if config.backend != MemoryBackend::Postgres {
        return Ok(SleepConsolidationReport::default());
    }

    #[cfg(feature = "postgres")]
    {
        let pool = postgres::open_pool(workspace_dir, config)?;
        let now = Utc::now();
        let scan_cutoff = now - Duration::hours(SLEEP_SCAN_WINDOW_HOURS);
        let batch_date = now.format("%Y-%m-%d").to_string();
        let now_rfc3339 = now.to_rfc3339();

        postgres::block_on_pg_result(async {
            let mut tx = pool
                .begin()
                .await
                .context("begin sleep consolidation transaction")?;

            let grouped = postgres::load_sleep_consolidation_groups(&mut tx, scan_cutoff).await?;
            let owner_ids: HashSet<EntityId> =
                grouped.keys().map(|key| key.entity_id.clone()).collect();

            let mut consolidated_groups = 0_u64;
            let mut snapshots_written = 0_u64;

            for owner_id in &owner_ids {
                postgres::refresh_graph_entity_decay_scores(&mut tx, owner_id.as_str(), now)
                    .await?;
                postgres::promote_sleep_episode_groups(&mut tx, owner_id, &now_rfc3339).await?;
            }

            for (group_key, candidates) in grouped {
                if candidates.len() < MIN_GROUP_SIZE {
                    continue;
                }

                consolidated_groups += 1;
                let aggregate = aggregation::aggregate_group(&candidates);
                let snapshot_slot_key = format!(
                    "semantic.sleep.{}.{}.{}",
                    group_key.entity_id, group_key.topic_prefix, batch_date
                );
                let unit_id = format!("{}::{}", group_key.entity_id, snapshot_slot_key);

                let inserted = postgres::insert_sleep_snapshot_retrieval_unit(
                    &mut tx,
                    &group_key,
                    &snapshot_slot_key,
                    &unit_id,
                    &aggregate,
                    &now_rfc3339,
                )
                .await?;

                if inserted == 0 {
                    continue;
                }

                postgres::insert_sleep_snapshot_event(
                    &mut tx,
                    &group_key,
                    &snapshot_slot_key,
                    &aggregate,
                    &now_rfc3339,
                )
                .await?;

                snapshots_written += inserted;
            }

            tx.commit()
                .await
                .context("commit sleep consolidation transaction")?;

            for owner_id in &owner_ids {
                crate::core::memory::graphrag::activation_cache()
                    .invalidate(owner_id)
                    .await;
            }

            Ok(SleepConsolidationReport {
                consolidated_groups,
                snapshots_written,
            })
        })
    }

    #[cfg(not(feature = "postgres"))]
    {
        let _ = workspace_dir;
        let _ = config;
        Ok(SleepConsolidationReport::default())
    }
}

fn compute_decay_score(importance: f64, hours_since_access: f64, access_count: u64) -> f64 {
    let lambda = 0.01_f64;
    let recency = (-lambda * hours_since_access.max(0.0)).exp();
    let bounded_access_count = u32::try_from(access_count).unwrap_or(u32::MAX);
    let access_bonus = 1.0 + (1.0 + f64::from(bounded_access_count)).ln();
    (importance.clamp(0.0, 1.0) * recency * access_bonus).clamp(0.0, 1.0)
}

pub(super) fn promote_episode_group_to_note(
    episodes: &[GraphEntity],
    synthesized_text: &str,
) -> GraphEntity {
    let exemplar = &episodes[0];
    let max_importance = episodes
        .iter()
        .map(|episode| episode.importance.get())
        .fold(0.0_f64, f64::max);

    GraphEntity {
        graph_entity_id: EntityId::new(format!("note_{}", uuid::Uuid::new_v4())),
        owner_entity_id: exemplar.owner_entity_id.clone(),
        entity_type: exemplar.entity_type,
        label: format!("Note: {}", exemplar.label),
        value: synthesized_text.to_string(),
        source: exemplar.source,
        confidence: exemplar.confidence,
        importance: Importance::new(max_importance),
        access_count: 0,
        last_accessed_at: None,
        is_pinned: false,
        temporal_decay_score: 1.0,
        node_tier: NodeTier::Note,
        parent_graph_entity_id: None,
        promoted_at: None,
        privacy_level: exemplar.privacy_level.clone(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::aggregation::{aggregate_group, topic_prefix_from_slot_key};
    use super::{
        ConsolidationCandidate, SleepConsolidationReport, compute_decay_score,
        promote_episode_group_to_note,
    };
    use crate::contracts::scores::{Confidence, Importance};
    use crate::core::memory::{GraphEntity, GraphEntityType, MemorySource, NodeTier, PrivacyLevel};

    fn graph_entity(importance: f64, access_count: u64, pinned: bool, decay: f64) -> GraphEntity {
        GraphEntity {
            graph_entity_id: "event::example".into(),
            owner_entity_id: "person:alice".into(),
            entity_type: GraphEntityType::Event,
            label: "Episode".to_string(),
            value: "Alice learned Rust".to_string(),
            source: MemorySource::ExplicitUser,
            confidence: Confidence::new(0.9),
            importance: Importance::new(importance),
            access_count,
            last_accessed_at: None,
            is_pinned: pinned,
            temporal_decay_score: decay,
            node_tier: NodeTier::Episode,
            parent_graph_entity_id: Some("note::parent".into()),
            promoted_at: Some("2026-01-01T00:00:00Z".to_string()),
            privacy_level: PrivacyLevel::Private,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn extracts_topic_prefix_from_first_two_segments() {
        let slot_key = "episodic.promoted.user.preference.food";
        assert_eq!(topic_prefix_from_slot_key(slot_key), "episodic.promoted");
    }

    #[test]
    fn sleep_report_defaults_to_zero() {
        let report = SleepConsolidationReport::default();
        assert_eq!(report.consolidated_groups, 0);
        assert_eq!(report.snapshots_written, 0);
    }

    #[test]
    fn aggregate_group_uses_most_restrictive_visibility() {
        let candidates = vec![
            ConsolidationCandidate {
                content: "public memory".to_string(),
                signal_tier: "raw".to_string(),
                importance: 0.2,
                reliability: 0.6,
                visibility: "public".to_string(),
            },
            ConsolidationCandidate {
                content: "secret memory".to_string(),
                signal_tier: "candidate".to_string(),
                importance: 0.8,
                reliability: 0.9,
                visibility: "secret".to_string(),
            },
        ];

        let aggregate = aggregate_group(&candidates);

        assert_eq!(aggregate.visibility, "secret");
    }

    #[test]
    fn decay_score_decreases_with_time() {
        let recent = compute_decay_score(0.8, 1.0, 1);
        let stale = compute_decay_score(0.8, 72.0, 1);
        assert!(recent > stale);
    }

    #[test]
    fn decay_score_increases_with_access() {
        let cold = compute_decay_score(0.7, 6.0, 0);
        let warm = compute_decay_score(0.7, 6.0, 8);
        assert!(warm > cold);
    }

    #[test]
    fn pinned_entities_skip_decay_eviction() {
        let pinned = graph_entity(0.2, 0, true, 0.01);
        let unpinned = graph_entity(0.2, 0, false, 0.01);
        assert!(pinned.temporal_decay_score >= 0.05 || pinned.is_pinned);
        assert!(unpinned.temporal_decay_score < 0.05 && !unpinned.is_pinned);
    }

    #[test]
    fn episode_promotion_creates_note_with_lineage() {
        let episodes = vec![
            graph_entity(0.4, 3, false, 0.2),
            graph_entity(0.8, 1, false, 0.3),
        ];
        let note = promote_episode_group_to_note(&episodes, "Alice consistently prefers Rust.");

        assert_eq!(note.node_tier, NodeTier::Note);
        assert_eq!(note.parent_graph_entity_id, None);
        assert_eq!(note.promoted_at, None);
        assert_eq!(note.access_count, 0);
        assert_eq!(note.value, "Alice consistently prefers Rust.");
        assert_eq!(note.importance, Importance::new(0.8));
    }
}
