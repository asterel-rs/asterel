//! Data types for the consistency subsystem: claims and contradiction
//! findings.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::SlotKey;
use crate::contracts::scores::Confidence;

/// A factual claim extracted from a message, ready for contradiction checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Claim {
    /// Memory slot key the claim targets.
    pub slot_key: SlotKey,
    /// The claimed value.
    pub new_value: String,
    /// Confidence that this is a genuine factual claim (0.0-1.0)
    pub extraction_confidence: Confidence,
}

/// Result of comparing a claim against existing memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ContradictionFinding {
    /// Memory slot key where the contradiction was found.
    pub slot_key: SlotKey,
    /// Value currently stored in memory.
    pub existing_value: String,
    /// Incoming value that conflicts with the existing one.
    pub new_value: String,
    /// Confidence that this is a genuine contradiction (0.0-1.0)
    pub contradiction_confidence: Confidence,
}

#[cfg(test)]
mod tests {
    use super::{Claim, ContradictionFinding};
    use crate::contracts::ids::SlotKey;

    #[test]
    fn claim_constructs_with_expected_values() {
        let claim = Claim {
            slot_key: SlotKey::new("profile.language"),
            new_value: "Rust".to_string(),
            extraction_confidence: crate::contracts::scores::Confidence::new(0.9),
        };

        assert_eq!(claim.slot_key.as_str(), "profile.language");
        assert_eq!(claim.new_value, "Rust");
        assert_eq!(
            claim.extraction_confidence,
            crate::contracts::scores::Confidence::new(0.9)
        );
    }

    #[test]
    fn contradiction_finding_constructs_with_expected_values() {
        let finding = ContradictionFinding {
            slot_key: SlotKey::new("profile.language"),
            existing_value: "Python".to_string(),
            new_value: "Rust".to_string(),
            contradiction_confidence: crate::contracts::scores::Confidence::new(0.75),
        };

        assert_eq!(finding.slot_key.as_str(), "profile.language");
        assert_eq!(finding.existing_value, "Python");
        assert_eq!(finding.new_value, "Rust");
        assert_eq!(
            finding.contradiction_confidence,
            crate::contracts::scores::Confidence::new(0.75)
        );
    }
}
