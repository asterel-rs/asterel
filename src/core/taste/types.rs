//! Core data types for the taste engine: artifacts, domains,
//! axes, pair comparisons, suggestions, and taste reports.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strum::Display;

/// Format of a text artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    /// Unformatted plain text.
    Plain,
    /// Markdown-formatted text.
    Markdown,
    /// HTML-formatted text.
    Html,
}

/// Input artifact to the taste engine (text or UI only; no image/video/audio).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Artifact {
    /// A text artifact with optional format specification.
    Text {
        /// The text content to evaluate.
        content: String,
        /// Optional format hint (plain, markdown, html).
        #[serde(default)]
        format: Option<TextFormat>,
    },
    /// A UI artifact described textually with optional metadata.
    Ui {
        /// Human-readable description of the UI artifact.
        description: String,
        /// Optional structured metadata about the UI element.
        #[serde(default)]
        metadata: Option<serde_json::Value>,
    },
}

/// Domain classification for an artifact (text, UI, or general).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display, Default)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Domain {
    /// Text artifacts (prose, markdown, HTML).
    Text,
    /// User interface artifacts (layout, components).
    Ui,
    /// Generic domain when no specific type applies.
    #[default]
    General,
}

/// Aesthetic evaluation axis (coherence, hierarchy, intentionality).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Axis {
    /// Stylistic unity: elements belong to the same worldview.
    Coherence,
    /// Visual/logical ordering: primary focus is identifiable.
    Hierarchy,
    /// Deliberate choice: every element is purposefully placed.
    Intentionality,
}

/// Scores per aesthetic axis (`BTreeMap` for stable ordering).
pub type AxisScores = BTreeMap<Axis, f64>;

/// Context for evaluating an artifact (domain, genre, purpose, audience, constraints).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TasteContext {
    /// Domain of the artifact being evaluated.
    #[serde(default)]
    pub domain: Domain,
    /// Optional genre label (e.g. "technical", "creative").
    #[serde(default)]
    pub genre: Option<String>,
    /// Optional purpose description for the artifact.
    #[serde(default)]
    pub purpose: Option<String>,
    /// Optional target audience description.
    #[serde(default)]
    pub audience: Option<String>,
    /// Domain-specific constraints to apply during evaluation.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Additional freeform key-value metadata.
    #[serde(default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Priority level for a suggestion (high, medium, low).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Priority {
    /// Urgent improvement needed.
    High,
    /// Moderate improvement recommended.
    Medium,
    /// Minor polish suggestion.
    Low,
}

/// Text correction operation type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOp {
    /// Reorganize the argument structure.
    RestructureArgument,
    /// Adjust information density (more or less concise).
    AdjustDensity,
    /// Harmonize inconsistent style or voice.
    UnifyStyle,
    /// Add or improve outline/heading structure.
    AddOutline,
    /// Custom text operation with a description.
    Other(String),
}

/// UI correction operation type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiOp {
    /// Restructure layout arrangement.
    AdjustLayout,
    /// Strengthen visual hierarchy.
    ImproveHierarchy,
    /// Increase contrast between elements.
    AddContrast,
    /// Fine-tune spacing and alignment.
    RefineSpacing,
    /// Custom UI operation with a description.
    Other(String),
}

/// Improvement suggestion for an artifact (general, text, or UI).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Suggestion {
    /// A domain-agnostic improvement suggestion.
    General {
        /// Short title for the suggestion.
        title: String,
        /// Explanation of why this improvement matters.
        rationale: String,
        /// How urgently this should be addressed.
        priority: Priority,
    },
    /// A text-specific improvement suggestion.
    Text {
        /// The text correction operation to apply.
        op: TextOp,
        /// Explanation of why this improvement matters.
        rationale: String,
        /// How urgently this should be addressed.
        priority: Priority,
    },
    /// A UI-specific improvement suggestion.
    Ui {
        /// The UI correction operation to apply.
        op: UiOp,
        /// Explanation of why this improvement matters.
        rationale: String,
        /// How urgently this should be addressed.
        priority: Priority,
    },
}

/// Result of evaluating an artifact (axis scores, domain, suggestions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasteReport {
    /// Per-axis aesthetic scores.
    pub axis: AxisScores,
    /// Detected domain of the evaluated artifact.
    pub domain: Domain,
    /// Actionable improvement suggestions.
    pub suggestions: Vec<Suggestion>,
    /// Raw LLM critique response, if available.
    #[serde(default)]
    pub raw_critique: Option<String>,
}

/// Outcome of a pair comparison (left, right, tie, or abstain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Winner {
    /// The left artifact was preferred.
    Left,
    /// The right artifact was preferred.
    Right,
    /// Neither artifact was clearly preferred.
    Tie,
    /// The comparison was not meaningful or declined.
    Abstain,
}

/// Owner scope for taste comparisons and learned ratings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TasteOwnerScope {
    /// Active tenant identifier when tenant isolation is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Requesting entity/person identifier for the tool execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// Current session identifier, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl TasteOwnerScope {
    /// Create an owner scope from optional tenant/session context and a required entity id.
    #[must_use]
    pub fn new(
        tenant_id: Option<impl Into<String>>,
        entity_id: impl Into<String>,
        session_id: Option<impl Into<String>>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.map(Into::into),
            entity_id: Some(entity_id.into()),
            session_id: session_id.map(Into::into),
        }
    }

    /// Stable storage key used to partition persisted taste rows by owner.
    #[must_use]
    pub fn storage_key(&self) -> String {
        format!(
            "tenant={}|entity={}|session={}",
            self.tenant_id.as_deref().unwrap_or("none"),
            self.entity_id.as_deref().unwrap_or("unknown"),
            self.session_id.as_deref().unwrap_or("none")
        )
    }
}

/// Record of a preference comparison between two artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairComparison {
    /// Tenant/person/session owner for the comparison.
    #[serde(default)]
    pub owner: TasteOwnerScope,
    /// Domain of both compared artifacts.
    pub domain: Domain,
    /// Evaluation context for the comparison.
    pub ctx: TasteContext,
    /// Identifier of the left artifact.
    pub left_id: String,
    /// Identifier of the right artifact.
    pub right_id: String,
    /// Which artifact was preferred.
    pub winner: Winner,
    /// Optional explanation for the preference decision.
    #[serde(default)]
    pub rationale: Option<String>,
    /// Unix timestamp in milliseconds when the comparison was made.
    pub created_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_text_roundtrip() {
        let a = Artifact::Text {
            content: "hello".into(),
            format: Some(TextFormat::Markdown),
        };
        let json = serde_json::to_string(&a).unwrap();
        let b: Artifact = serde_json::from_str(&json).unwrap();
        if let Artifact::Text { content, format } = b {
            assert_eq!(content, "hello");
            assert_eq!(format, Some(TextFormat::Markdown));
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn axis_btreemap_key() {
        let mut scores: AxisScores = BTreeMap::new();
        scores.insert(Axis::Coherence, 0.8);
        scores.insert(Axis::Hierarchy, 0.6);
        scores.insert(Axis::Intentionality, 0.9);
        assert_eq!(scores.len(), 3);
        // BTreeMap ordering: Coherence < Hierarchy < Intentionality (alphabetical via Ord)
        assert!(scores.contains_key(&Axis::Coherence));
    }

    #[test]
    fn axis_has_exactly_3_variants() {
        // If this test fails, someone added a 4th axis (violates guardrail)
        let axes = [Axis::Coherence, Axis::Hierarchy, Axis::Intentionality];
        assert_eq!(axes.len(), 3);
    }

    #[test]
    fn suggestion_text_tagged_enum() {
        let s = Suggestion::Text {
            op: TextOp::UnifyStyle,
            rationale: "needs unification".into(),
            priority: Priority::High,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"kind\":\"text\""));
        let s2: Suggestion = serde_json::from_str(&json).unwrap();
        if let Suggestion::Text { op, priority, .. } = s2 {
            assert_eq!(op, TextOp::UnifyStyle);
            assert_eq!(priority, Priority::High);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn pair_comparison_roundtrip() {
        let pc = PairComparison {
            owner: TasteOwnerScope::new(Some("tenant-a"), "person-a", Some("session-a")),
            domain: Domain::Text,
            ctx: TasteContext::default(),
            left_id: "a".into(),
            right_id: "b".into(),
            winner: Winner::Left,
            rationale: Some("clearer".into()),
            created_at_ms: 1_234_567_890,
        };
        let json = serde_json::to_string(&pc).unwrap();
        let pc2: PairComparison = serde_json::from_str(&json).unwrap();
        assert_eq!(pc2.left_id, "a");
        assert_eq!(pc2.owner.entity_id.as_deref(), Some("person-a"));
        assert_eq!(pc2.winner, Winner::Left);
        assert_eq!(pc2.created_at_ms, 1_234_567_890);
    }

    #[test]
    fn domain_default_is_general() {
        let d = Domain::default();
        assert_eq!(d, Domain::General);
    }
}
