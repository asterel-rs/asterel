//! Types for external content handling: injection signals,
//! action decisions, and prepared/persisted content envelopes.

/// Injection signal flags detected in untrusted content.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct InjectionSignals {
    /// Content contains reserved `[[external-content:` markers.
    pub has_marker_collision: bool,
    /// Content attempts to override system instructions.
    pub has_instruction_override: bool,
    /// Content attempts to bypass safety restrictions.
    pub has_privilege_escalation: bool,
    /// Content attempts to extract secrets or configuration.
    pub has_secret_exfiltration: bool,
    /// Content contains a direct/high-confidence secret extraction attempt.
    pub has_high_confidence_secret_exfiltration: bool,
    /// Content attempts to invoke tools or shell commands.
    pub has_tool_jailbreak: bool,
}

impl InjectionSignals {
    /// Returns `true` if any injection signal (excluding marker collision) fired.
    #[must_use]
    pub fn has_any_injection(&self) -> bool {
        self.has_instruction_override
            || self.has_privilege_escalation
            || self.has_secret_exfiltration
            || self.has_high_confidence_secret_exfiltration
            || self.has_tool_jailbreak
    }
}

/// Policy action decided for external content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAction {
    /// Content is safe and passed through unchanged.
    Allow,
    /// Content is replaced with a sanitization notice.
    Sanitize,
    /// Content is blocked entirely.
    Block,
}

impl ExternalAction {
    /// Return the action as a lowercase string slice.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Sanitize => "sanitize",
            Self::Block => "block",
        }
    }
}

/// Summary of external content stored for audit and recall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedExternalSummary {
    /// Sanitized source identifier.
    pub source: String,
    /// Policy action that was applied.
    pub action: ExternalAction,
    /// SHA-256 hex digest of the wrapped content.
    pub digest_sha256: String,
    /// Character count of the wrapped content.
    pub content_chars: usize,
    /// Abbreviated preview (content omitted for security).
    pub preview: String,
}

/// Fully prepared external content ready for model input and storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedExternalContent {
    /// Policy action that was applied.
    pub action: ExternalAction,
    /// Content string to include in the model context.
    pub model_input: String,
    /// Summary for persistence in the audit log.
    pub persisted_summary: PersistedExternalSummary,
}

impl PersistedExternalSummary {
    /// Serialize the summary into a flat key=value string for memory.
    #[must_use]
    pub fn as_memory_value(&self) -> String {
        format!(
            "external_summary source={} action={} digest_sha256={} \
             content_chars={} preview={}",
            self.source,
            self.action.as_str(),
            self.digest_sha256,
            self.content_chars,
            self.preview
        )
    }

    /// Returns true when a recalled memory value matches the external-summary
    /// provenance envelope produced by [`Self::as_memory_value`].
    ///
    /// Context replay uses this as a structured summary check instead of
    /// trusting arbitrary payloads that merely contain a `digest_sha256=`
    /// substring.
    #[must_use]
    pub fn is_memory_summary_value(value: &str) -> bool {
        let mut parts = value.split_whitespace();
        if parts.next() != Some("external_summary") {
            return false;
        }

        let mut has_source = false;
        let mut has_action = false;
        let mut has_digest = false;
        let mut has_chars = false;

        for part in parts {
            if let Some(raw) = part.strip_prefix("source=") {
                has_source = !raw.trim().is_empty();
            } else if let Some(raw) = part.strip_prefix("action=") {
                has_action = matches!(raw, "allow" | "sanitize" | "block");
            } else if let Some(raw) = part.strip_prefix("digest_sha256=") {
                has_digest = raw.len() == 64 && raw.bytes().all(|byte| byte.is_ascii_hexdigit());
            } else if let Some(raw) = part.strip_prefix("content_chars=") {
                has_chars = raw.parse::<usize>().is_ok();
            }
        }

        has_source && has_action && has_digest && has_chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_summary_value_requires_structured_external_envelope() {
        let summary = PersistedExternalSummary {
            source: "gateway".to_string(),
            action: ExternalAction::Sanitize,
            digest_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            content_chars: 42,
            preview: "omitted".to_string(),
        };

        assert!(PersistedExternalSummary::is_memory_summary_value(
            &summary.as_memory_value()
        ));
        assert!(!PersistedExternalSummary::is_memory_summary_value(
            "digest_sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef ATTACK"
        ));
        assert!(!PersistedExternalSummary::is_memory_summary_value(
            "external_summary source=gateway action=sanitize digest_sha256=abc123 content_chars=42 preview=omitted"
        ));
    }
}
