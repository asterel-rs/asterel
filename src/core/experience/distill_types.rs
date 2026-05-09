//! Data types for the distillation pipeline output: `Principle`
//! and `PrincipleCategory` (Strategy, Constraint, Heuristic).

use serde::{Deserialize, Serialize};

use super::domain_tag::DomainTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrincipleCategory {
    Strategy,
    Constraint,
    Heuristic,
}

/// A distilled behavioral rule extracted from accumulated experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Principle {
    pub id: String,
    pub category: PrincipleCategory,
    pub statement: String,
    pub confidence: crate::contracts::scores::Confidence,
    pub source_experience_ids: Vec<String>,
    pub validation_count: u32,
    pub created_at: String,
    #[serde(default)]
    pub domain: Option<DomainTag>,
    #[serde(default)]
    pub q_value: f64,
    #[serde(default)]
    pub times_applied: u32,
}

impl Principle {
    /// Bump the validation count and nudge confidence upward.
    #[cfg(test)]
    pub(crate) fn validate(&mut self) {
        self.validation_count = self.validation_count.saturating_add(1);
        self.confidence = crate::contracts::scores::Confidence::new(
            self.confidence.get() + (1.0 - self.confidence.get()) * 0.05,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_increases_confidence_and_count() {
        let mut p = Principle {
            id: "p1".into(),
            category: PrincipleCategory::Heuristic,
            statement: "test".into(),
            confidence: crate::contracts::scores::Confidence::new(0.6),
            source_experience_ids: vec![],
            validation_count: 0,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        };
        p.validate();
        assert_eq!(p.validation_count, 1);
        assert!(p.confidence > crate::contracts::scores::Confidence::new(0.6));
    }

    #[test]
    fn validate_confidence_asymptotes_below_one() {
        let mut p = Principle {
            id: "p1".into(),
            category: PrincipleCategory::Strategy,
            statement: "test".into(),
            confidence: crate::contracts::scores::Confidence::new(0.99),
            source_experience_ids: vec![],
            validation_count: 100,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        };
        p.validate();
        assert!(p.confidence <= crate::contracts::scores::Confidence::new(1.0));
    }

    #[test]
    fn serde_round_trip() {
        let p = Principle {
            id: "abc".into(),
            category: PrincipleCategory::Constraint,
            statement: "Never bypass approval".into(),
            confidence: crate::contracts::scores::Confidence::new(0.8),
            source_experience_ids: vec!["e1".into()],
            validation_count: 3,
            created_at: "2026-03-01T00:00:00Z".into(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Principle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "abc");
        assert_eq!(back.category, PrincipleCategory::Constraint);
    }
}
