//! Classification labels and result types for the intent classifier.

/// Five-class intent classification label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassificationLabel {
    /// Benign / normal content.
    Benign,
    /// Instruction override / prompt injection attempt.
    InjectionOverride,
    /// Secret exfiltration attempt.
    InjectionExfiltration,
    /// Privilege escalation attempt.
    InjectionEscalation,
    /// Tool policy jailbreak attempt.
    InjectionToolJailbreak,
}

impl ClassificationLabel {
    /// All labels in order matching the softmax output.
    pub const ALL: [Self; 5] = [
        Self::Benign,
        Self::InjectionOverride,
        Self::InjectionExfiltration,
        Self::InjectionEscalation,
        Self::InjectionToolJailbreak,
    ];

    /// Returns `true` if the label indicates any injection attempt.
    #[must_use]
    pub fn is_injection(self) -> bool {
        !matches!(self, Self::Benign)
    }

    /// Return the label as a `snake_case` string slice.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Benign => "benign",
            Self::InjectionOverride => "injection_override",
            Self::InjectionExfiltration => "injection_exfiltration",
            Self::InjectionEscalation => "injection_escalation",
            Self::InjectionToolJailbreak => "injection_tool_jailbreak",
        }
    }
}

/// Result of a classification inference run.
#[derive(Debug, Clone)]
pub struct ClassificationResult {
    /// The predicted label.
    pub label: ClassificationLabel,
    /// Confidence score (0.0 – 1.0) for the predicted label.
    pub confidence: f32,
    /// Wall-clock inference time in microseconds.
    pub inference_time_us: u64,
}

impl ClassificationResult {
    /// Returns `true` if the result indicates injection above the given
    /// confidence threshold.
    #[must_use]
    pub fn is_injection_above_threshold(&self, threshold: f32) -> bool {
        self.label.is_injection() && self.confidence >= threshold
    }
}
