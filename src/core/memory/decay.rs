//! Temporal decay and composite scoring for memory retrieval.
//!
//! Provides exponential decay with a 30-day half-life and a
//! three-factor composite score (recency x importance x relevance).

/// Half-life in days for the exponential decay curve.
const HALF_LIFE_DAYS: f64 = 30.0;

/// Pre-computed decay constant: `ln(2) / HALF_LIFE_DAYS`.
const DECAY_LAMBDA: f64 = core::f64::consts::LN_2 / HALF_LIFE_DAYS;

/// Compute temporal decay factor for a memory of the given age.
///
/// Uses an exponential decay with a 30-day half-life:
///
/// ```text
/// decay = e^(-ln(2)/30 * age_days)
/// ```
///
/// Returns a value in `(0.0, 1.0]` — 1.0 at age 0, 0.5 at 30 days,
/// 0.25 at 60 days, etc.
///
/// Negative ages (future timestamps) are clamped to 0 (decay = 1.0).
#[must_use]
pub fn temporal_decay(age_days: f64) -> f64 {
    if age_days <= 0.0 {
        return 1.0;
    }
    (-DECAY_LAMBDA * age_days).exp().clamp(0.0, 1.0)
}

/// Compute the temporal decay factor from a stored `recency_score`.
///
/// The `recency_score` in retrieval units is stored as a linear
/// fraction of a 90-day window: `1.0 - age_days / 90`.  This
/// function converts that back to age and applies exponential decay.
///
/// Pinned memories always return 1.0 (no decay).
#[must_use]
pub fn recency_to_temporal_decay(recency_score: f64, pinned: bool) -> f64 {
    if pinned {
        return 1.0;
    }
    // recency_score = 1.0 - age_days / 90.0
    // => age_days = (1.0 - recency_score) * 90.0
    let age_days = (1.0 - recency_score.clamp(0.0, 1.0)) * 90.0;
    temporal_decay(age_days)
}

/// Composite three-factor score: recency x importance x relevance.
///
/// All inputs are expected in `[0.0, 1.0]`.  The result is their
/// product, which is also in `[0.0, 1.0]`.
#[must_use]
pub fn composite_score(recency: f64, importance: f64, relevance: f64) -> f64 {
    let r = recency.clamp(0.0, 1.0);
    let i = importance.clamp(0.0, 1.0);
    let v = relevance.clamp(0.0, 1.0);
    (r * i * v).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_at_zero_age_is_one() {
        let d = temporal_decay(0.0);
        assert!((d - 1.0).abs() < 1e-12, "decay at age 0 should be 1.0, got {d}");
    }

    #[test]
    fn decay_at_half_life_is_half() {
        let d = temporal_decay(30.0);
        assert!(
            (d - 0.5).abs() < 1e-6,
            "decay at 30 days should be ~0.5, got {d}"
        );
    }

    #[test]
    fn decay_at_two_half_lives_is_quarter() {
        let d = temporal_decay(60.0);
        assert!(
            (d - 0.25).abs() < 1e-6,
            "decay at 60 days should be ~0.25, got {d}"
        );
    }

    #[test]
    fn decay_negative_age_clamped_to_one() {
        assert!((temporal_decay(-5.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn decay_very_old_approaches_zero() {
        let d = temporal_decay(365.0);
        assert!(d < 0.001, "decay at 365 days should be near zero, got {d}");
        assert!(d > 0.0, "decay should never reach exactly zero");
    }

    #[test]
    fn pinned_memory_always_returns_one() {
        assert!((recency_to_temporal_decay(0.1, true) - 1.0).abs() < 1e-12);
        assert!((recency_to_temporal_decay(0.0, true) - 1.0).abs() < 1e-12);
        assert!((recency_to_temporal_decay(1.0, true) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn recency_score_one_means_age_zero() {
        let d = recency_to_temporal_decay(1.0, false);
        assert!(
            (d - 1.0).abs() < 1e-12,
            "recency_score=1.0 should yield decay=1.0, got {d}"
        );
    }

    #[test]
    fn recency_score_zero_means_age_ninety() {
        let d = recency_to_temporal_decay(0.0, false);
        let expected = temporal_decay(90.0);
        assert!(
            (d - expected).abs() < 1e-12,
            "recency_score=0.0 should map to age=90 days: expected {expected}, got {d}"
        );
    }

    #[test]
    fn composite_score_all_ones() {
        assert!((composite_score(1.0, 1.0, 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn composite_score_any_zero_is_zero() {
        assert!(composite_score(0.0, 1.0, 1.0).abs() < 1e-12);
        assert!(composite_score(1.0, 0.0, 1.0).abs() < 1e-12);
        assert!(composite_score(1.0, 1.0, 0.0).abs() < 1e-12);
    }

    #[test]
    fn composite_score_product() {
        let s = composite_score(0.5, 0.8, 0.6);
        let expected = 0.5 * 0.8 * 0.6;
        assert!(
            (s - expected).abs() < 1e-12,
            "expected {expected}, got {s}"
        );
    }

    #[test]
    fn composite_score_clamps_inputs() {
        let s = composite_score(1.5, -0.2, 0.5);
        // clamped: 1.0 * 0.0 * 0.5 = 0.0
        assert!(s.abs() < 1e-12);
    }
}
