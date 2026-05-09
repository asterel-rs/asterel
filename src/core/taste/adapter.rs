//! Domain adapters that translate critique axis scores into
//! actionable text or UI improvement suggestions.

use super::critic::CritiqueResult;
use super::types::{Axis, Domain, Priority, Suggestion, TasteContext, TextOp, UiOp};

/// Translates axis scores into domain-specific improvement suggestions.
pub(crate) trait DomainAdapter: Send + Sync {
    /// Return the domain this adapter handles.
    fn domain(&self) -> Domain;
    /// Generate suggestions from critique scores and context.
    fn suggest(&self, critique: &CritiqueResult, ctx: &TasteContext) -> Vec<Suggestion>;
}

fn score_to_priority(score: f64) -> Priority {
    if score < 0.3 {
        Priority::High
    } else if score < 0.5 {
        Priority::Medium
    } else {
        Priority::Low
    }
}

// ── TextAdapter ──────────────────────────────────────────────

/// Domain adapter for text artifacts (prose, markdown, HTML).
pub(crate) struct TextAdapter;

impl DomainAdapter for TextAdapter {
    fn domain(&self) -> Domain {
        Domain::Text
    }

    fn suggest(&self, critique: &CritiqueResult, _ctx: &TasteContext) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();
        for (axis, &score) in &critique.axis_scores {
            if score >= 0.6 {
                continue;
            }
            let priority = score_to_priority(score);
            let (op, rationale) = match axis {
                Axis::Coherence => (TextOp::UnifyStyle, "Low coherence detected"),
                Axis::Hierarchy => (TextOp::AddOutline, "Low hierarchy detected"),
                Axis::Intentionality => (TextOp::AdjustDensity, "Low intentionality detected"),
            };
            suggestions.push(Suggestion::Text {
                op,
                rationale: rationale.to_string(),
                priority,
            });
        }
        suggestions
    }
}

// ── UiAdapter ────────────────────────────────────────────────

/// Domain adapter for UI artifacts (layout, components, spacing).
pub(crate) struct UiAdapter;

impl DomainAdapter for UiAdapter {
    fn domain(&self) -> Domain {
        Domain::Ui
    }

    fn suggest(&self, critique: &CritiqueResult, _ctx: &TasteContext) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();
        for (axis, &score) in &critique.axis_scores {
            if score >= 0.6 {
                continue;
            }
            let priority = score_to_priority(score);
            let (op, rationale) = match axis {
                Axis::Coherence => (UiOp::RefineSpacing, "Low coherence detected"),
                Axis::Hierarchy => (UiOp::ImproveHierarchy, "Low hierarchy detected"),
                Axis::Intentionality => (UiOp::AdjustLayout, "Low intentionality detected"),
            };
            suggestions.push(Suggestion::Ui {
                op,
                rationale: rationale.to_string(),
                priority,
            });
        }
        suggestions
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn make_critique(coherence: f64, hierarchy: f64, intentionality: f64) -> CritiqueResult {
        let mut axis_scores = BTreeMap::new();
        axis_scores.insert(Axis::Coherence, coherence);
        axis_scores.insert(Axis::Hierarchy, hierarchy);
        axis_scores.insert(Axis::Intentionality, intentionality);
        CritiqueResult {
            axis_scores,
            raw_response: String::new(),
            confidence: 0.7,
        }
    }

    // -- score_to_priority -------------------------------------------------

    #[test]
    fn score_to_priority_high() {
        assert_eq!(score_to_priority(0.0), Priority::High);
        assert_eq!(score_to_priority(0.2), Priority::High);
        assert_eq!(score_to_priority(0.29), Priority::High);
    }

    #[test]
    fn score_to_priority_medium() {
        assert_eq!(score_to_priority(0.3), Priority::Medium);
        assert_eq!(score_to_priority(0.4), Priority::Medium);
        assert_eq!(score_to_priority(0.49), Priority::Medium);
    }

    #[test]
    fn score_to_priority_low() {
        assert_eq!(score_to_priority(0.5), Priority::Low);
        assert_eq!(score_to_priority(0.59), Priority::Low);
    }

    // -- TextAdapter -------------------------------------------------------

    #[test]
    fn text_all_low_scores_yields_three_high_suggestions() {
        let adapter = TextAdapter;
        let critique = make_critique(0.2, 0.2, 0.2);
        let ctx = TasteContext::default();
        let suggestions = adapter.suggest(&critique, &ctx);
        assert_eq!(suggestions.len(), 3);
        for s in &suggestions {
            if let Suggestion::Text { priority, .. } = s {
                assert_eq!(*priority, Priority::High);
            } else {
                panic!("expected Suggestion::Text");
            }
        }
    }

    #[test]
    fn text_all_high_scores_yields_no_suggestions() {
        let adapter = TextAdapter;
        let critique = make_critique(0.9, 0.9, 0.9);
        let ctx = TasteContext::default();
        let suggestions = adapter.suggest(&critique, &ctx);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn text_mixed_scores_yields_one_suggestion() {
        let adapter = TextAdapter;
        let critique = make_critique(0.8, 0.2, 0.8);
        let ctx = TasteContext::default();
        let suggestions = adapter.suggest(&critique, &ctx);
        assert_eq!(suggestions.len(), 1);
        if let Suggestion::Text { op, priority, .. } = &suggestions[0] {
            assert_eq!(*op, TextOp::AddOutline);
            assert_eq!(*priority, Priority::High);
        } else {
            panic!("expected Suggestion::Text");
        }
    }

    #[test]
    fn text_domain_returns_text() {
        assert_eq!(TextAdapter.domain(), Domain::Text);
    }

    #[test]
    fn text_axis_op_mapping() {
        let adapter = TextAdapter;
        let ctx = TasteContext::default();

        let critique = make_critique(0.1, 0.8, 0.8);
        let s = adapter.suggest(&critique, &ctx);
        assert_eq!(s.len(), 1);
        if let Suggestion::Text { op, .. } = &s[0] {
            assert_eq!(*op, TextOp::UnifyStyle);
        } else {
            panic!("expected UnifyStyle");
        }

        let critique = make_critique(0.8, 0.8, 0.1);
        let s = adapter.suggest(&critique, &ctx);
        assert_eq!(s.len(), 1);
        if let Suggestion::Text { op, .. } = &s[0] {
            assert_eq!(*op, TextOp::AdjustDensity);
        } else {
            panic!("expected AdjustDensity");
        }
    }

    // -- UiAdapter ---------------------------------------------------------

    #[test]
    fn ui_all_low_scores_yields_three_high_suggestions() {
        let adapter = UiAdapter;
        let critique = make_critique(0.2, 0.2, 0.2);
        let ctx = TasteContext::default();
        let suggestions = adapter.suggest(&critique, &ctx);
        assert_eq!(suggestions.len(), 3);
        for s in &suggestions {
            if let Suggestion::Ui { priority, .. } = s {
                assert_eq!(*priority, Priority::High);
            } else {
                panic!("expected Suggestion::Ui");
            }
        }
    }

    #[test]
    fn ui_all_high_scores_yields_no_suggestions() {
        let adapter = UiAdapter;
        let critique = make_critique(0.9, 0.9, 0.9);
        let ctx = TasteContext::default();
        let suggestions = adapter.suggest(&critique, &ctx);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn ui_mixed_scores_yields_one_suggestion() {
        let adapter = UiAdapter;
        let critique = make_critique(0.8, 0.2, 0.8);
        let ctx = TasteContext::default();
        let suggestions = adapter.suggest(&critique, &ctx);
        assert_eq!(suggestions.len(), 1);
        if let Suggestion::Ui { op, priority, .. } = &suggestions[0] {
            assert_eq!(*op, UiOp::ImproveHierarchy);
            assert_eq!(*priority, Priority::High);
        } else {
            panic!("expected Suggestion::Ui");
        }
    }

    #[test]
    fn ui_domain_returns_ui() {
        assert_eq!(UiAdapter.domain(), Domain::Ui);
    }

    #[test]
    fn ui_axis_op_mapping() {
        let adapter = UiAdapter;
        let ctx = TasteContext::default();

        let critique = make_critique(0.1, 0.8, 0.8);
        let s = adapter.suggest(&critique, &ctx);
        assert_eq!(s.len(), 1);
        if let Suggestion::Ui { op, .. } = &s[0] {
            assert_eq!(*op, UiOp::RefineSpacing);
        } else {
            panic!("expected RefineSpacing");
        }

        let critique = make_critique(0.8, 0.8, 0.1);
        let s = adapter.suggest(&critique, &ctx);
        assert_eq!(s.len(), 1);
        if let Suggestion::Ui { op, .. } = &s[0] {
            assert_eq!(*op, UiOp::AdjustLayout);
        } else {
            panic!("expected AdjustLayout");
        }
    }
}
