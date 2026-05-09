//! Capability-based security for tool execution.
//!
//! Each tool declares the minimum capabilities it requires (filesystem,
//! network, shell, etc.). The middleware checks that the execution
//! context grants those capabilities before allowing execution.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub use crate::contracts::tools::Capability;

/// Set of capabilities granted to an execution context.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitySet {
    capabilities: HashSet<Capability>,
}

impl CapabilitySet {
    /// Create a new empty capability set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a capability set from a list of capabilities.
    pub fn from_capabilities(iter: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            capabilities: iter.into_iter().collect(),
        }
    }

    /// Create a capability set that grants all capabilities.
    #[must_use]
    pub fn all() -> Self {
        Self {
            capabilities: HashSet::from([
                Capability::Filesystem,
                Capability::Network,
                Capability::Shell,
                Capability::MemoryWrite,
                Capability::ExternalAction,
                Capability::CognitiveRead,
                Capability::CognitiveWrite,
                Capability::Unrestricted,
            ]),
        }
    }

    /// Add a capability to the set.
    pub fn insert(&mut self, cap: Capability) {
        self.capabilities.insert(cap);
    }

    /// Check whether a specific capability is granted.
    #[must_use]
    pub fn contains(&self, cap: &Capability) -> bool {
        self.capabilities.contains(&Capability::Unrestricted) || self.capabilities.contains(cap)
    }

    /// Return the number of capabilities in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Return whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }
}

/// Check that all required capabilities are present in the granted set.
///
/// Returns `Ok(())` if all required capabilities are granted, or
/// `Err(missing)` with the list of missing capabilities.
///
/// # Errors
///
/// Returns `Err(missing)` when one or more required capabilities are absent
/// from the granted set.
pub fn check_capabilities(
    required: &[Capability],
    granted: &CapabilitySet,
) -> Result<(), Vec<Capability>> {
    let missing: Vec<Capability> = required
        .iter()
        .filter(|cap| !granted.contains(cap))
        .copied()
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_required_always_passes() {
        let granted = CapabilitySet::new();
        assert!(check_capabilities(&[], &granted).is_ok());
    }

    #[test]
    fn missing_capability_reported() {
        let granted = CapabilitySet::from_capabilities([Capability::Filesystem]);
        let result = check_capabilities(&[Capability::Network], &granted);
        assert_eq!(result.unwrap_err(), vec![Capability::Network]);
    }

    #[test]
    fn multiple_missing_capabilities() {
        let granted = CapabilitySet::from_capabilities([Capability::Filesystem]);
        let result = check_capabilities(&[Capability::Network, Capability::Shell], &granted);
        let missing = result.unwrap_err();
        assert!(missing.contains(&Capability::Network));
        assert!(missing.contains(&Capability::Shell));
    }

    #[test]
    fn all_granted_passes() {
        let granted =
            CapabilitySet::from_capabilities([Capability::Filesystem, Capability::Network]);
        assert!(
            check_capabilities(&[Capability::Filesystem, Capability::Network], &granted).is_ok()
        );
    }

    #[test]
    fn unrestricted_grants_everything() {
        let granted = CapabilitySet::from_capabilities([Capability::Unrestricted]);
        assert!(
            check_capabilities(
                &[
                    Capability::Filesystem,
                    Capability::Network,
                    Capability::Shell,
                    Capability::MemoryWrite,
                    Capability::ExternalAction,
                ],
                &granted,
            )
            .is_ok()
        );
    }

    #[test]
    fn partial_overlap_reports_only_missing() {
        let granted = CapabilitySet::from_capabilities([Capability::Filesystem, Capability::Shell]);
        let result = check_capabilities(
            &[
                Capability::Filesystem,
                Capability::Network,
                Capability::Shell,
            ],
            &granted,
        );
        assert_eq!(result.unwrap_err(), vec![Capability::Network]);
    }

    #[test]
    fn capability_set_all_contains_every_variant() {
        let all = CapabilitySet::all();
        assert!(all.contains(&Capability::Filesystem));
        assert!(all.contains(&Capability::Network));
        assert!(all.contains(&Capability::Shell));
        assert!(all.contains(&Capability::MemoryWrite));
        assert!(all.contains(&Capability::ExternalAction));
        assert!(all.contains(&Capability::Unrestricted));
    }

    #[test]
    fn capability_display() {
        assert_eq!(Capability::Filesystem.to_string(), "filesystem");
        assert_eq!(Capability::Network.to_string(), "network");
        assert_eq!(Capability::Shell.to_string(), "shell");
        assert_eq!(Capability::MemoryWrite.to_string(), "memory_write");
        assert_eq!(Capability::ExternalAction.to_string(), "external_action");
        assert_eq!(Capability::Unrestricted.to_string(), "unrestricted");
    }

    #[test]
    fn capability_set_len_and_is_empty() {
        let empty = CapabilitySet::new();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let one = CapabilitySet::from_capabilities([Capability::Shell]);
        assert!(!one.is_empty());
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn capability_serde_roundtrip() {
        let cap = Capability::ExternalAction;
        let json = serde_json::to_string(&cap).unwrap();
        let parsed: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, parsed);
    }

    #[test]
    fn capability_set_serde_roundtrip() {
        let set = CapabilitySet::from_capabilities([Capability::Filesystem, Capability::Network]);
        let json = serde_json::to_string(&set).unwrap();
        let parsed: CapabilitySet = serde_json::from_str(&json).unwrap();
        assert!(parsed.contains(&Capability::Filesystem));
        assert!(parsed.contains(&Capability::Network));
        assert!(!parsed.contains(&Capability::Shell));
    }
}
