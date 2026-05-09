//! Keyword-based domain classifier for experience atoms.
//! Tags experiences into domains (`CodeDebugging`, `SystemAdmin`,
//! `DataAnalysis`, etc.) to enable cross-domain transfer learning.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DomainTag {
    CodeDebugging,
    SystemAdmin,
    DataAnalysis,
    Writing,
    Planning,
    Research,
    General,
}

const CODE_KW: &[&str] = &[
    "error",
    "bug",
    "compile",
    "debug",
    "stack trace",
    "exception",
    "segfault",
];
const ADMIN_KW: &[&str] = &[
    "server",
    "deploy",
    "docker",
    "kubernetes",
    "nginx",
    "ssh",
    "systemd",
];
const DATA_KW: &[&str] = &[
    "csv",
    "dataset",
    "plot",
    "statistics",
    "regression",
    "pandas",
    "sql query",
];
const WRITE_KW: &[&str] = &["essay", "article", "draft", "paragraph", "tone", "grammar"];
const PLAN_KW: &[&str] = &["schedule", "timeline", "milestone", "roadmap", "sprint"];
const RESEARCH_KW: &[&str] = &[
    "paper",
    "hypothesis",
    "methodology",
    "citation",
    "literature",
];

/// Infer the domain tag from free-text content via keyword matching.
pub(crate) fn infer_domain(text: &str) -> DomainTag {
    let lower = text.to_lowercase();
    let score = |kws: &[&str]| kws.iter().filter(|kw| lower.contains(*kw)).count();
    let candidates = [
        (score(CODE_KW), DomainTag::CodeDebugging),
        (score(ADMIN_KW), DomainTag::SystemAdmin),
        (score(DATA_KW), DomainTag::DataAnalysis),
        (score(WRITE_KW), DomainTag::Writing),
        (score(PLAN_KW), DomainTag::Planning),
        (score(RESEARCH_KW), DomainTag::Research),
    ];
    candidates
        .into_iter()
        .filter(|(s, _)| *s > 0)
        .max_by_key(|(s, _)| *s)
        .map_or(DomainTag::General, |(_, tag)| tag)
}

/// Return a similarity score between two domain tags in `[0.0, 1.0]`.
///
/// Same domains score 1.0, related pairs (e.g. Code/Admin) score 0.6,
/// and unrelated pairs score 0.2.
pub(crate) fn domain_similarity(a: DomainTag, b: DomainTag) -> f64 {
    use DomainTag::{CodeDebugging, DataAnalysis, Research, SystemAdmin, Writing};
    if a == b {
        return 1.0;
    }
    match (a, b) {
        (CodeDebugging | SystemAdmin, CodeDebugging | SystemAdmin)
        | (DataAnalysis | Research, DataAnalysis | Research)
        | (Writing, Research)
        | (Research, Writing) => 0.6,
        _ => 0.2,
    }
}

/// Filter principles to those with domain affinity at or above a threshold.
pub(crate) fn filter_by_domain_affinity(
    principles: &[super::distill_types::Principle],
    target: DomainTag,
    min_affinity: f64,
) -> Vec<&super::distill_types::Principle> {
    principles
        .iter()
        .filter(|p| domain_similarity(infer_domain(&p.statement), target) >= min_affinity)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::experience::distill_types::{Principle, PrincipleCategory};

    fn make_principle(stmt: &str) -> Principle {
        Principle {
            id: String::new(),
            category: PrincipleCategory::Heuristic,
            statement: stmt.into(),
            confidence: 0.5.into(),
            source_experience_ids: vec![],
            validation_count: 0,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }
    }

    #[test]
    fn infer_all_domains() {
        let cases: &[(&str, DomainTag)] = &[
            ("fix the compile error", DomainTag::CodeDebugging),
            ("deploy to kubernetes via ssh", DomainTag::SystemAdmin),
            ("load the csv dataset", DomainTag::DataAnalysis),
            ("revise the essay paragraph tone", DomainTag::Writing),
            ("update the roadmap milestone", DomainTag::Planning),
            ("review the paper methodology", DomainTag::Research),
            ("hello world", DomainTag::General),
        ];
        for (text, expected) in cases {
            assert_eq!(infer_domain(text), *expected, "failed for: {text}");
        }
    }

    #[test]
    fn infer_case_insensitive() {
        assert_eq!(infer_domain("DEBUG the SEGFAULT"), DomainTag::CodeDebugging);
    }

    #[test]
    fn similarity_same_domain() {
        let diff = (domain_similarity(DomainTag::Writing, DomainTag::Writing) - 1.0).abs();
        assert!(diff < f64::EPSILON);
    }

    #[test]
    fn similarity_related_pairs() {
        for (a, b) in [
            (DomainTag::CodeDebugging, DomainTag::SystemAdmin),
            (DomainTag::DataAnalysis, DomainTag::Research),
            (DomainTag::Writing, DomainTag::Research),
        ] {
            assert!((domain_similarity(a, b) - 0.6).abs() < f64::EPSILON);
            assert!((domain_similarity(b, a) - 0.6).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn similarity_unrelated() {
        let diff = (domain_similarity(DomainTag::Writing, DomainTag::CodeDebugging) - 0.2).abs();
        assert!(diff < f64::EPSILON);
    }

    #[test]
    fn filter_affinity() {
        let exact = [make_principle("fix the compile error and debug segfault")];
        assert_eq!(
            filter_by_domain_affinity(&exact, DomainTag::CodeDebugging, 1.0).len(),
            1
        );
        let related = [make_principle("deploy docker to server")];
        assert_eq!(
            filter_by_domain_affinity(&related, DomainTag::CodeDebugging, 0.5).len(),
            1
        );
        let unrelated = [make_principle("revise the essay grammar")];
        assert!(filter_by_domain_affinity(&unrelated, DomainTag::CodeDebugging, 0.5).is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let tag = DomainTag::DataAnalysis;
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, "\"data_analysis\"");
        let back: DomainTag = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tag);
    }
}
