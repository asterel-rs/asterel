//! Inference options: thinking level, temperature, and per-request
//! provider configuration.

use serde::{Deserialize, Serialize};

pub use crate::contracts::inference::ThinkingLevel;

/// Per-request inference options sent to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct InferenceOpts {
    /// Thinking / chain-of-thought level for this request.
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
    /// Absolute `top_p` value (e.g. 0.92). `None` = provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    /// Multiplier applied to provider-default `max_tokens` (0.7–1.0). `None` = no adjustment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_factor: Option<f64>,
}

impl InferenceOpts {
    /// Create options with only a thinking level set.
    #[must_use]
    pub const fn from_thinking_level(thinking_level: ThinkingLevel) -> Self {
        Self {
            thinking_level,
            top_p: None,
            max_tokens_factor: None,
        }
    }
}

// ── Provider-specific thinking-level mappings ──────────────────

/// Map a `ThinkingLevel` to the `OpenAI` `reasoning_effort` parameter.
#[must_use]
pub(crate) const fn openai_reasoning_effort(level: ThinkingLevel) -> Option<&'static str> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Low => Some("low"),
        ThinkingLevel::Medium => Some("medium"),
        ThinkingLevel::High => Some("high"),
    }
}

/// Map a `ThinkingLevel` to the Anthropic budget token count.
#[must_use]
pub(crate) const fn anthropic_budget_tokens(level: ThinkingLevel) -> Option<u32> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Low => Some(1_024),
        ThinkingLevel::Medium => Some(2_048),
        ThinkingLevel::High => Some(3_072),
    }
}

/// Map a `ThinkingLevel` to the Gemini thinking budget token count.
#[must_use]
pub(crate) const fn gemini_thinking_budget(level: ThinkingLevel) -> Option<u32> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Low => Some(1_024),
        ThinkingLevel::Medium => Some(2_048),
        ThinkingLevel::High => Some(3_072),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InferenceOpts, ThinkingLevel, anthropic_budget_tokens, gemini_thinking_budget,
        openai_reasoning_effort,
    };

    #[test]
    fn parse_thinking_level_accepts_aliases() {
        assert_eq!(ThinkingLevel::parse("off"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("none"), Some(ThinkingLevel::Off));
        assert_eq!(ThinkingLevel::parse("low"), Some(ThinkingLevel::Low));
        assert_eq!(ThinkingLevel::parse("med"), Some(ThinkingLevel::Medium));
        assert_eq!(ThinkingLevel::parse("high"), Some(ThinkingLevel::High));
        assert_eq!(ThinkingLevel::parse(""), None);
        assert_eq!(ThinkingLevel::parse("extreme"), None);
    }

    #[test]
    fn toggled_flips_between_off_and_medium() {
        assert_eq!(ThinkingLevel::Off.toggled(), ThinkingLevel::Medium);
        assert_eq!(ThinkingLevel::Low.toggled(), ThinkingLevel::Off);
        assert_eq!(ThinkingLevel::Medium.toggled(), ThinkingLevel::Off);
        assert_eq!(ThinkingLevel::High.toggled(), ThinkingLevel::Off);
    }

    #[test]
    fn provider_inference_options_constructor_sets_level() {
        let options = InferenceOpts::from_thinking_level(ThinkingLevel::High);
        assert_eq!(options.thinking_level, ThinkingLevel::High);
    }

    #[test]
    fn openai_reasoning_effort_maps_expected_values() {
        assert_eq!(openai_reasoning_effort(ThinkingLevel::Off), None);
        assert_eq!(openai_reasoning_effort(ThinkingLevel::Low), Some("low"));
        assert_eq!(
            openai_reasoning_effort(ThinkingLevel::Medium),
            Some("medium")
        );
        assert_eq!(openai_reasoning_effort(ThinkingLevel::High), Some("high"));
    }

    #[test]
    fn anthropic_budget_tokens_maps_expected_values() {
        assert_eq!(anthropic_budget_tokens(ThinkingLevel::Off), None);
        assert_eq!(anthropic_budget_tokens(ThinkingLevel::Low), Some(1_024));
        assert_eq!(anthropic_budget_tokens(ThinkingLevel::Medium), Some(2_048));
        assert_eq!(anthropic_budget_tokens(ThinkingLevel::High), Some(3_072));
    }

    #[test]
    fn gemini_thinking_budget_maps_expected_values() {
        assert_eq!(gemini_thinking_budget(ThinkingLevel::Off), None);
        assert_eq!(gemini_thinking_budget(ThinkingLevel::Low), Some(1_024));
        assert_eq!(gemini_thinking_budget(ThinkingLevel::Medium), Some(2_048));
        assert_eq!(gemini_thinking_budget(ThinkingLevel::High), Some(3_072));
    }
}
