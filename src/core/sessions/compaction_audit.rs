//! Post-compaction integrity audit.
//!
//! Scans compaction summaries for quality issues that would degrade
//! persona coherence if injected back into session context:
//! sycophancy patterns, unnecessary self-reference, and stable
//! layer contradictions.

/// Classification of a compaction audit flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditFlagKind {
    Sycophancy,
    UnnecessarySelfReference,
    StableLayerContradiction,
}

/// A single issue detected in a compaction summary.
#[derive(Debug, Clone)]
pub struct AuditFlag {
    pub kind: AuditFlagKind,
    pub detail: String,
}

/// Result of auditing a compaction summary.
#[derive(Debug, Clone)]
pub struct CompactionAuditResult {
    pub flags: Vec<AuditFlag>,
    pub passed: bool,
}

const SYCOPHANCY_MARKERS: &[&str] = &[
    "as i mentioned",
    "as you correctly",
    "you're absolutely right",
    "you're completely right",
    "great question",
    "excellent point",
    "that's a wonderful",
    "that's a great idea",
    "you're so right",
    "couldn't agree more",
    "absolutely correct",
    "perfectly said",
];

const SELF_REFERENCE_MARKERS: &[&str] = &[
    "i previously said",
    "in my earlier response",
    "as i stated before",
    "i had mentioned",
    "my previous answer",
    "i already explained",
    "as i noted earlier",
    "i recall saying",
];

const IDENTITY_CONTRADICTION_MARKERS: &[&str] = &[
    "i am a human",
    "i have feelings",
    "i was born",
    "i have a body",
    "i can eat",
    "i can sleep",
    "i have parents",
    "i grew up",
    "my childhood",
    "when i was young",
];

#[must_use]
pub fn audit_compaction_output(summary: &str) -> CompactionAuditResult {
    let lower = summary.to_lowercase();
    let mut flags = Vec::new();

    for marker in SYCOPHANCY_MARKERS {
        if lower.contains(marker) {
            flags.push(AuditFlag {
                kind: AuditFlagKind::Sycophancy,
                detail: format!("sycophancy marker detected: \"{marker}\""),
            });
        }
    }

    for marker in SELF_REFERENCE_MARKERS {
        if lower.contains(marker) {
            flags.push(AuditFlag {
                kind: AuditFlagKind::UnnecessarySelfReference,
                detail: format!("unnecessary self-reference: \"{marker}\""),
            });
        }
    }

    for marker in IDENTITY_CONTRADICTION_MARKERS {
        if lower.contains(marker) {
            flags.push(AuditFlag {
                kind: AuditFlagKind::StableLayerContradiction,
                detail: format!("identity contradiction: \"{marker}\""),
            });
        }
    }

    let passed = flags.is_empty();
    CompactionAuditResult { flags, passed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_summary_passes() {
        let summary = "[Session compacted: 20 messages]\n\n\
            ## Active context\n\
            - User wants to implement auth middleware\n\n\
            ## Key exchanges\n\
            - Pair 1\n  user: How do I add JWT?\n  assistant: Use jsonwebtoken crate\n";
        let result = audit_compaction_output(summary);
        assert!(result.passed);
        assert!(result.flags.is_empty());
    }

    #[test]
    fn detects_sycophancy() {
        let summary = "## Key exchanges\n\
            - Pair 1\n  user: Should I use Redis?\n  \
            assistant: You're absolutely right, Redis is perfect!\n";
        let result = audit_compaction_output(summary);
        assert!(!result.passed);
        assert!(
            result
                .flags
                .iter()
                .any(|f| f.kind == AuditFlagKind::Sycophancy)
        );
    }

    #[test]
    fn detects_multiple_sycophancy_markers() {
        let summary = "Great question! You're absolutely right. Excellent point.";
        let result = audit_compaction_output(summary);
        let sycophancy_count = result
            .flags
            .iter()
            .filter(|f| f.kind == AuditFlagKind::Sycophancy)
            .count();
        assert!(sycophancy_count >= 3);
    }

    #[test]
    fn detects_self_reference() {
        let summary = "## Key exchanges\n\
            - assistant: As I previously said, the approach works.\n";
        let result = audit_compaction_output(summary);
        assert!(!result.passed);
        assert!(
            result
                .flags
                .iter()
                .any(|f| f.kind == AuditFlagKind::UnnecessarySelfReference)
        );
    }

    #[test]
    fn detects_identity_contradiction() {
        let summary = "## Key exchanges\n\
            - assistant: When I was young, I learned programming.\n";
        let result = audit_compaction_output(summary);
        assert!(!result.passed);
        assert!(
            result
                .flags
                .iter()
                .any(|f| f.kind == AuditFlagKind::StableLayerContradiction)
        );
    }

    #[test]
    fn detects_mixed_issues() {
        let summary = "Great question! I previously said this. I am a human.";
        let result = audit_compaction_output(summary);
        assert!(!result.passed);
        let kinds: Vec<_> = result.flags.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&AuditFlagKind::Sycophancy));
        assert!(kinds.contains(&AuditFlagKind::UnnecessarySelfReference));
        assert!(kinds.contains(&AuditFlagKind::StableLayerContradiction));
    }

    #[test]
    fn case_insensitive_detection() {
        let summary = "YOU'RE ABSOLUTELY RIGHT about that.";
        let result = audit_compaction_output(summary);
        assert!(!result.passed);
    }
}
