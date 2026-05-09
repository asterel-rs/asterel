//! Bradley-Terry preference learner: updates item ratings from
//! pairwise comparison outcomes using logistic regression.

use std::collections::HashMap;

use super::types::{PairComparison, Winner};

const LEARNING_RATE: f64 = 4.0;
const L2_REGULARIZATION: f64 = 0.5;
const SIGMOID_CLAMP_MIN: f64 = -35.0;
const SIGMOID_CLAMP_MAX: f64 = 35.0;

/// Preference learner that maintains item ratings from pairwise outcomes.
pub(crate) trait TasteLearner: Send + Sync {
    /// Update ratings after one comparison (outcome in `[0.0, 1.0]`).
    fn update(&mut self, winner_id: &str, loser_id: &str, outcome: f64);
    /// Return the current (rating, `comparison_count`) for an item.
    fn get_rating(&self, item_id: &str) -> Option<(f64, u32)>;
    /// Return the rating only if the item has enough comparisons.
    fn get_rating_if_sufficient(&self, item_id: &str, min_comparisons: u32) -> Option<f64>;
    /// Reconstruct a learner by replaying historical comparisons.
    #[allow(dead_code)]
    fn from_comparisons(comparisons: &[PairComparison]) -> Self
    where
        Self: Sized;
}

/// Bradley-Terry logistic preference learner with L2 regularisation.
#[derive(Clone)]
pub(crate) struct BradleyTerryLearner {
    /// Item ID to (rating, `comparison_count`) mapping.
    ratings: HashMap<String, (f64, u32)>,
}

impl BradleyTerryLearner {
    /// Create a new learner with no prior ratings.
    pub(crate) fn new() -> Self {
        Self {
            ratings: HashMap::new(),
        }
    }

    fn sigmoid(x: f64) -> f64 {
        1.0 / (1.0 + (-x).exp())
    }
}

impl TasteLearner for BradleyTerryLearner {
    fn update(&mut self, winner_id: &str, loser_id: &str, outcome: f64) {
        let (winner_rating, winner_count) =
            self.ratings.entry(winner_id.to_owned()).or_insert((0.0, 0));
        let winner_rating_before = *winner_rating;
        let winner_count_before = *winner_count;

        let (loser_rating, loser_count) =
            self.ratings.entry(loser_id.to_owned()).or_insert((0.0, 0));
        let loser_rating_before = *loser_rating;
        let loser_count_before = *loser_count;

        let logit = (winner_rating_before - loser_rating_before)
            .clamp(SIGMOID_CLAMP_MIN, SIGMOID_CLAMP_MAX);
        let winner_probability = Self::sigmoid(logit);
        let loser_probability = 1.0 - winner_probability;

        let winner_rating_after = winner_rating_before
            + LEARNING_RATE * (outcome - winner_probability)
            - L2_REGULARIZATION * winner_rating_before;
        let loser_rating_after = loser_rating_before
            + LEARNING_RATE * ((1.0 - outcome) - loser_probability)
            - L2_REGULARIZATION * loser_rating_before;

        self.ratings.insert(
            winner_id.to_owned(),
            (winner_rating_after, winner_count_before + 1),
        );
        self.ratings.insert(
            loser_id.to_owned(),
            (loser_rating_after, loser_count_before + 1),
        );
    }

    fn get_rating(&self, item_id: &str) -> Option<(f64, u32)> {
        self.ratings.get(item_id).copied()
    }

    fn get_rating_if_sufficient(&self, item_id: &str, min_comparisons: u32) -> Option<f64> {
        self.get_rating(item_id)
            .and_then(|(rating, comparisons)| (comparisons >= min_comparisons).then_some(rating))
    }

    fn from_comparisons(comparisons: &[PairComparison]) -> Self {
        let mut learner = Self::new();

        for comparison in comparisons {
            match comparison.winner {
                Winner::Left => learner.update(&comparison.left_id, &comparison.right_id, 1.0),
                Winner::Right => learner.update(&comparison.right_id, &comparison.left_id, 1.0),
                Winner::Tie => learner.update(&comparison.left_id, &comparison.right_id, 0.5),
                Winner::Abstain => {}
            }
        }

        learner
    }
}

#[cfg(test)]
mod tests {
    use super::{BradleyTerryLearner, TasteLearner};
    use crate::core::taste::types::{
        Domain, PairComparison, TasteContext, TasteOwnerScope, Winner,
    };

    fn comparison(
        left_id: &str,
        right_id: &str,
        winner: Winner,
        created_at_ms: u64,
    ) -> PairComparison {
        PairComparison {
            owner: TasteOwnerScope::default(),
            domain: Domain::General,
            ctx: TasteContext::default(),
            left_id: left_id.to_owned(),
            right_id: right_id.to_owned(),
            winner,
            rationale: None,
            created_at_ms,
        }
    }

    #[test]
    fn threshold_gating_respects_minimum_comparisons() {
        let mut learner = BradleyTerryLearner::new();

        for _ in 0..4 {
            learner.update("alpha", "beta", 1.0);
        }

        assert!(learner.get_rating_if_sufficient("alpha", 5).is_none());

        learner.update("alpha", "beta", 1.0);

        assert!(learner.get_rating_if_sufficient("alpha", 5).is_some());
    }

    #[test]
    fn replay_determinism_matches_incremental_updates() {
        let comparisons = vec![
            comparison("left", "right", Winner::Left, 1),
            comparison("left", "right", Winner::Right, 2),
            comparison("left", "right", Winner::Tie, 3),
            comparison("left", "right", Winner::Abstain, 4),
            comparison("left", "right", Winner::Left, 5),
        ];

        let mut incremental = BradleyTerryLearner::new();
        incremental.update("left", "right", 1.0);
        incremental.update("right", "left", 1.0);
        incremental.update("left", "right", 0.5);
        incremental.update("left", "right", 1.0);

        let replayed = BradleyTerryLearner::from_comparisons(&comparisons);

        let (incremental_left_rating, incremental_left_count) =
            incremental.get_rating("left").expect("left should exist");
        let (replayed_left_rating, replayed_left_count) =
            replayed.get_rating("left").expect("left should exist");
        let (incremental_right_rating, incremental_right_count) =
            incremental.get_rating("right").expect("right should exist");
        let (replayed_right_rating, replayed_right_count) =
            replayed.get_rating("right").expect("right should exist");

        assert!((incremental_left_rating - replayed_left_rating).abs() < f64::EPSILON);
        assert_eq!(incremental_left_count, replayed_left_count);
        assert!((incremental_right_rating - replayed_right_rating).abs() < f64::EPSILON);
        assert_eq!(incremental_right_count, replayed_right_count);
    }

    #[test]
    fn repeated_wins_raise_winner_above_loser() {
        let mut learner = BradleyTerryLearner::new();

        for _ in 0..10 {
            learner.update("winner", "loser", 1.0);
        }

        let winner_rating = learner.get_rating("winner").expect("winner should exist").0;
        let loser_rating = learner.get_rating("loser").expect("loser should exist").0;

        assert!(winner_rating > loser_rating);
    }

    #[test]
    fn new_items_start_with_zero_rating() {
        let mut learner = BradleyTerryLearner::new();

        assert!(learner.get_rating("fresh").is_none());

        learner.update("fresh", "other", 1.0);

        let fresh_rating = learner.get_rating("fresh").expect("fresh should exist").0;
        let other_rating = learner.get_rating("other").expect("other should exist").0;

        assert!((fresh_rating - 2.0).abs() < 1e-12);
        assert!((other_rating + 2.0).abs() < 1e-12);
    }
}
