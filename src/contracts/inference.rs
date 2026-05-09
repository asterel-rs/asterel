//! Inference configuration contracts shared between `core` and `config`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "off" | "none" | "false" | "0" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Off => Self::Medium,
            Self::Low | Self::Medium | Self::High => Self::Off,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExplainabilityMode {
    #[default]
    Off,
    Minimal,
    Verbose,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_level_serde_roundtrip_all_variants() {
        let cases = [
            (ThinkingLevel::Off, "off"),
            (ThinkingLevel::Low, "low"),
            (ThinkingLevel::Medium, "medium"),
            (ThinkingLevel::High, "high"),
        ];

        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let parsed: ThinkingLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn thinking_level_parse_handles_valid_and_invalid_inputs() {
        assert_eq!(ThinkingLevel::parse("off"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("none"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("false"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("0"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("low"), Some(ThinkingLevel::Low));
        assert_eq!(ThinkingLevel::parse("medium"), Some(ThinkingLevel::Medium));
        assert_eq!(ThinkingLevel::parse("med"), Some(ThinkingLevel::Medium));
        assert_eq!(ThinkingLevel::parse("high"), Some(ThinkingLevel::High));

        assert_eq!(ThinkingLevel::parse(""), None);
        assert_eq!(ThinkingLevel::parse("unknown"), None);
        assert_eq!(ThinkingLevel::parse("1"), None);
    }

    #[test]
    fn thinking_level_toggled_switches_off_and_medium() {
        assert_eq!(ThinkingLevel::Off.toggled(), ThinkingLevel::Medium);
        assert_eq!(ThinkingLevel::Low.toggled(), ThinkingLevel::Off);
        assert_eq!(ThinkingLevel::Medium.toggled(), ThinkingLevel::Off);
        assert_eq!(ThinkingLevel::High.toggled(), ThinkingLevel::Off);
    }

    #[test]
    fn thinking_level_as_str_returns_expected_values() {
        assert_eq!(ThinkingLevel::Off.as_str(), "off");
        assert_eq!(ThinkingLevel::Low.as_str(), "low");
        assert_eq!(ThinkingLevel::Medium.as_str(), "medium");
        assert_eq!(ThinkingLevel::High.as_str(), "high");
    }

    #[test]
    fn explainability_mode_serde_roundtrip_all_variants() {
        let cases = [
            (ExplainabilityMode::Off, "off"),
            (ExplainabilityMode::Minimal, "minimal"),
            (ExplainabilityMode::Verbose, "verbose"),
        ];

        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let parsed: ExplainabilityMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn defaults_match_expected_variants() {
        assert_eq!(ThinkingLevel::default(), ThinkingLevel::Off);
        assert_eq!(ExplainabilityMode::default(), ExplainabilityMode::Off);
    }
}
