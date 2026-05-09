//! Inference-level configuration, currently the default thinking
//! level for extended-thinking capable providers.

use serde::{Deserialize, Serialize};

use crate::contracts::inference::ThinkingLevel;

/// Inference-level configuration for provider behavior.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InferenceConfig {
    /// Default thinking level for extended-thinking providers.
    #[serde(default)]
    pub default_thinking_level: ThinkingLevel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_inference_config_uses_off() {
        let cfg = InferenceConfig::default();
        assert_eq!(cfg.default_thinking_level, ThinkingLevel::Off);
    }

    #[test]
    fn toml_roundtrip_preserves_thinking_level() {
        let cfg = InferenceConfig {
            default_thinking_level: ThinkingLevel::Medium,
        };
        let serialized = toml::to_string(&cfg).expect("serialize");
        let deserialized: InferenceConfig = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.default_thinking_level, ThinkingLevel::Medium);
    }

    #[test]
    fn toml_missing_field_falls_back_to_default() {
        let deserialized: InferenceConfig = toml::from_str("").expect("deserialize empty");
        assert_eq!(deserialized.default_thinking_level, ThinkingLevel::Off);
    }

    #[test]
    fn toml_parses_all_levels() {
        for (input, expected) in [
            ("default_thinking_level = \"off\"", ThinkingLevel::Off),
            ("default_thinking_level = \"low\"", ThinkingLevel::Low),
            ("default_thinking_level = \"medium\"", ThinkingLevel::Medium),
            ("default_thinking_level = \"high\"", ThinkingLevel::High),
        ] {
            let cfg: InferenceConfig = toml::from_str(input).expect("parse level");
            assert_eq!(cfg.default_thinking_level, expected);
        }
    }
}
