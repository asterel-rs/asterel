//! Aggregation helpers for sleep consolidation groups.
//!
//! [`aggregate_group`] combines a slice of [`ConsolidationCandidate`]s into a
//! single [`GroupAggregate`]: content is concatenated, importance takes the
//! maximum, reliability is averaged, visibility takes the most-restrictive
//! level, and signal tier escalates to the highest observed tier.
//!
//! [`topic_prefix_from_slot_key`] extracts the first two dot-separated segments
//! of a slot key to form the grouping key used by the sleep consolidation scan.

use super::{ConsolidationCandidate, GroupAggregate};

pub(super) fn topic_prefix_from_slot_key(slot_key: &str) -> String {
    let mut parts = slot_key.split('.');
    let first = parts.next().unwrap_or_default();
    let second = parts.next().unwrap_or_default();

    if first.is_empty() {
        return String::new();
    }

    if second.is_empty() {
        return first.to_string();
    }

    format!("{first}.{second}")
}

pub(super) fn aggregate_group(candidates: &[ConsolidationCandidate]) -> GroupAggregate {
    let mut combined_content = String::new();
    let mut highest_signal = "raw";
    let mut max_importance = f64::MIN;
    let mut reliability_sum = 0.0_f64;
    let mut max_visibility_rank = 1_u8;

    for candidate in candidates {
        if !combined_content.is_empty() {
            combined_content.push('\n');
        }
        combined_content.push_str(&candidate.content);
        if signal_tier_rank(&candidate.signal_tier) > signal_tier_rank(highest_signal) {
            highest_signal = &candidate.signal_tier;
        }
        if candidate.importance > max_importance {
            max_importance = candidate.importance;
        }
        reliability_sum += candidate.reliability;
        max_visibility_rank = max_visibility_rank.max(visibility_rank(&candidate.visibility));
    }

    #[allow(clippy::cast_precision_loss)]
    let reliability_avg = reliability_sum / candidates.len() as f64;

    GroupAggregate {
        combined_content,
        signal_tier: highest_signal.to_string(),
        importance: max_importance,
        reliability_avg,
        visibility: visibility_from_rank(max_visibility_rank).to_string(),
    }
}

fn visibility_rank(visibility: &str) -> u8 {
    match visibility {
        "secret" => 3,
        "public" => 1,
        // "private" and any unknown visibility default to mid-rank.
        _ => 2,
    }
}

fn visibility_from_rank(rank: u8) -> &'static str {
    match rank {
        3 => "secret",
        2 => "private",
        _ => "public",
    }
}

fn signal_tier_rank(signal_tier: &str) -> u8 {
    match signal_tier {
        "governance" => 4,
        "promoted" => 3,
        "candidate" => 2,
        "raw" => 1,
        _ => 0,
    }
}
