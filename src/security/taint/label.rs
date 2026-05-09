//! Taint labels and taint sets for data contamination tracking.
//!
//! References: [TAINT-LATTICE] Denning, 1976 — lattice model of secure
//! information flow. See the public research reference index in the docs site.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A label describing the source or nature of data contamination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintLabel {
    /// Data originates from an external network source.
    ExternalNetwork,
    /// Data originates from direct user input.
    UserInput,
    /// Data contains or may contain personally identifiable information.
    Pii,
    /// Data contains or may contain secrets (API keys, tokens, etc.).
    Secret,
    /// Data was produced by an untrusted or unverified agent.
    UntrustedAgent,
}

impl fmt::Display for TaintLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExternalNetwork => write!(f, "external_network"),
            Self::UserInput => write!(f, "user_input"),
            Self::Pii => write!(f, "pii"),
            Self::Secret => write!(f, "secret"),
            Self::UntrustedAgent => write!(f, "untrusted_agent"),
        }
    }
}

/// A set of taint labels attached to a piece of data.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaintSet {
    labels: HashSet<TaintLabel>,
}

impl TaintSet {
    /// Create a new empty taint set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a taint set from an iterator of labels.
    pub fn from_labels(iter: impl IntoIterator<Item = TaintLabel>) -> Self {
        Self {
            labels: iter.into_iter().collect(),
        }
    }

    /// Add a label to the set.
    pub fn insert(&mut self, label: TaintLabel) {
        self.labels.insert(label);
    }

    /// Check whether a specific label is present.
    #[must_use]
    pub fn contains(&self, label: &TaintLabel) -> bool {
        self.labels.contains(label)
    }

    /// Merge another taint set into this one (union).
    pub fn merge(&mut self, other: &TaintSet) {
        self.labels.extend(&other.labels);
    }

    /// Return the union of two taint sets without modifying either.
    #[must_use]
    pub fn union(&self, other: &TaintSet) -> TaintSet {
        TaintSet {
            labels: self.labels.union(&other.labels).copied().collect(),
        }
    }

    /// Return whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Return the number of labels in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Convert the taint set to a sorted vector of label strings
    /// suitable for embedding in tool results.
    #[must_use]
    pub fn to_string_vec(&self) -> Vec<String> {
        let mut v: Vec<String> = self.labels.iter().map(ToString::to_string).collect();
        v.sort();
        v
    }
}

impl fmt::Display for TaintSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let labels = self.to_string_vec();
        if labels.is_empty() {
            return write!(f, "(none)");
        }
        let mut iter = labels.iter();
        if let Some(first) = iter.next() {
            write!(f, "{first}")?;
        }
        for label in iter {
            write!(f, ", {label}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_taint_set() {
        let ts = TaintSet::new();
        assert!(ts.is_empty());
        assert_eq!(ts.len(), 0);
        assert_eq!(ts.to_string(), "(none)");
    }

    #[test]
    fn insert_and_contains() {
        let mut ts = TaintSet::new();
        ts.insert(TaintLabel::Pii);
        assert!(ts.contains(&TaintLabel::Pii));
        assert!(!ts.contains(&TaintLabel::Secret));
    }

    #[test]
    fn merge_combines_labels() {
        let mut a = TaintSet::from_labels([TaintLabel::Pii]);
        let b = TaintSet::from_labels([TaintLabel::Secret, TaintLabel::ExternalNetwork]);
        a.merge(&b);
        assert_eq!(a.len(), 3);
        assert!(a.contains(&TaintLabel::Pii));
        assert!(a.contains(&TaintLabel::Secret));
        assert!(a.contains(&TaintLabel::ExternalNetwork));
    }

    #[test]
    fn union_does_not_mutate() {
        let a = TaintSet::from_labels([TaintLabel::Pii]);
        let b = TaintSet::from_labels([TaintLabel::Secret]);
        let c = a.union(&b);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn display_sorted() {
        let ts = TaintSet::from_labels([TaintLabel::Secret, TaintLabel::ExternalNetwork]);
        let display = ts.to_string();
        // Both labels present, sorted alphabetically
        assert!(display.contains("external_network"));
        assert!(display.contains("secret"));
    }

    #[test]
    fn to_string_vec_sorted() {
        let ts = TaintSet::from_labels([TaintLabel::UserInput, TaintLabel::ExternalNetwork]);
        let v = ts.to_string_vec();
        assert_eq!(v, vec!["external_network", "user_input"]);
    }

    #[test]
    fn taint_label_display() {
        assert_eq!(TaintLabel::ExternalNetwork.to_string(), "external_network");
        assert_eq!(TaintLabel::UserInput.to_string(), "user_input");
        assert_eq!(TaintLabel::Pii.to_string(), "pii");
        assert_eq!(TaintLabel::Secret.to_string(), "secret");
        assert_eq!(TaintLabel::UntrustedAgent.to_string(), "untrusted_agent");
    }

    #[test]
    fn taint_label_serde_roundtrip() {
        let label = TaintLabel::ExternalNetwork;
        let json = serde_json::to_string(&label).unwrap();
        assert_eq!(json, "\"external_network\"");
        let parsed: TaintLabel = serde_json::from_str(&json).unwrap();
        assert_eq!(label, parsed);
    }

    #[test]
    fn taint_set_serde_roundtrip() {
        let ts = TaintSet::from_labels([TaintLabel::Pii, TaintLabel::Secret]);
        let json = serde_json::to_string(&ts).unwrap();
        let parsed: TaintSet = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, parsed);
    }
}
