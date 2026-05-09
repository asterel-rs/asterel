//! Security subsystem configuration types.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level security configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// ML-based intent classifier configuration.
    #[serde(default)]
    pub intent_classifier: IntentClassifierConfig,

    /// Trust-score policy for untrusted external knowledge ingress.
    #[serde(default)]
    pub external_knowledge_trust: ExternalKnowledgeTrustConfig,
}

/// Configuration for the ML-based intent classifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentClassifierConfig {
    /// Whether the intent classifier is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,

    /// Confidence threshold for classification. Predictions below this
    /// threshold are treated as benign. Default: `0.85`.
    #[serde(default = "default_threshold")]
    pub threshold: f32,

    /// Directory for ONNX model files. Defaults to `~/.asterel/models/`.
    #[serde(default)]
    pub models_dir: Option<PathBuf>,

    /// Whether to automatically download models from `HuggingFace` Hub on
    /// first use. Default: `true`.
    #[serde(default = "default_true")]
    pub auto_download: bool,
}

fn default_threshold() -> f32 {
    0.85
}

use super::default_true;

/// Trust-score policy for external-content sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalKnowledgeTrustConfig {
    /// Enable trust-score adjustments on top of injection detection.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Baseline score when no override matches.
    #[serde(default = "default_external_trust_score")]
    pub default_score: f32,

    /// Minimum score to keep content as-is when detector says `Allow`.
    #[serde(default = "default_external_trust_min_allow")]
    pub min_allow_score: f32,

    /// Scores below this are blocked.
    #[serde(default = "default_external_trust_min_sanitize")]
    pub min_sanitize_score: f32,

    /// Source-prefix score overrides (longest-prefix match).
    #[serde(default)]
    pub source_overrides: BTreeMap<String, f32>,
}

fn default_external_trust_score() -> f32 {
    0.60
}

fn default_external_trust_min_allow() -> f32 {
    0.70
}

fn default_external_trust_min_sanitize() -> f32 {
    0.30
}

const BUILTIN_SOURCE_PROFILES: [(&str, f32); 16] = [
    ("gateway:", 0.72),
    ("gateway:a2a", 0.78),
    ("gateway:webhook", 0.74),
    ("gateway:whatsapp", 0.73),
    ("channel:", 0.76),
    ("channel:imessage", 0.84),
    ("channel:slack", 0.82),
    ("channel:discord", 0.80),
    ("channel:telegram", 0.78),
    ("channel:matrix", 0.76),
    ("channel:email", 0.72),
    ("channel:irc", 0.68),
    ("tool:", 0.74),
    ("tool:web", 0.64),
    ("tool:http", 0.63),
    ("tool:browser", 0.62),
];

fn builtin_source_profile_score(source_normalized: &str) -> Option<f32> {
    BUILTIN_SOURCE_PROFILES
        .iter()
        .filter(|(prefix, _)| source_normalized.starts_with(*prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, score)| *score)
}

impl Default for IntentClassifierConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: default_threshold(),
            models_dir: None,
            auto_download: true,
        }
    }
}

impl Default for ExternalKnowledgeTrustConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_score: default_external_trust_score(),
            min_allow_score: default_external_trust_min_allow(),
            min_sanitize_score: default_external_trust_min_sanitize(),
            source_overrides: BTreeMap::new(),
        }
    }
}

impl IntentClassifierConfig {
    /// Resolve the models directory, falling back to
    /// `~/.asterel/models/`.
    #[must_use]
    pub fn resolved_models_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.models_dir {
            return dir.clone();
        }
        crate::utils::dirs::asterel_home_dir_or_local().join("models")
    }

    /// Check for environment variable overrides.
    #[must_use]
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(val) = std::env::var("ASTEREL_INTENT_CLASSIFIER_ENABLED") {
            self.enabled = val.eq_ignore_ascii_case("true") || val == "1";
        }
        if let Ok(val) = std::env::var("ASTEREL_INTENT_CLASSIFIER_THRESHOLD")
            && let Ok(threshold) = val.parse::<f32>()
        {
            if (0.0..=1.0).contains(&threshold) {
                self.threshold = threshold;
            } else {
                tracing::warn!(
                    "ASTEREL_INTENT_CLASSIFIER_THRESHOLD={threshold} out of [0,1]; ignoring"
                );
            }
        }
        if let Ok(val) = std::env::var("ASTEREL_INTENT_CLASSIFIER_MODELS_DIR") {
            self.models_dir = Some(PathBuf::from(val));
        }
        self
    }
}

impl ExternalKnowledgeTrustConfig {
    /// Resolve trust score for a source using longest-prefix override.
    #[must_use]
    pub fn score_for_source(&self, source: &str) -> f32 {
        if !self.enabled {
            return 1.0;
        }

        let normalized = source.trim().to_ascii_lowercase();
        let mut best_match: Option<(usize, f32)> = None;

        for (prefix, score) in &self.source_overrides {
            let prefix_normalized = prefix.trim().to_ascii_lowercase();
            if prefix_normalized.is_empty() {
                continue;
            }
            if normalized.starts_with(&prefix_normalized) {
                let candidate = (prefix_normalized.len(), *score);
                if best_match.is_none_or(|current| candidate.0 > current.0) {
                    best_match = Some(candidate);
                }
            }
        }

        best_match
            .map(|(_, score)| score)
            .or_else(|| builtin_source_profile_score(&normalized))
            .unwrap_or(self.default_score)
    }

    /// Validate threshold ranges and ordering.
    ///
    /// # Errors
    ///
    /// Returns an error when scores are outside `[0.0, 1.0]` or threshold
    /// ordering is invalid.
    pub fn validate(&self) -> anyhow::Result<()> {
        let in_range = |value: f32| (0.0..=1.0).contains(&value);
        if !in_range(self.default_score) {
            anyhow::bail!("security.external_knowledge_trust.default_score must be in [0.0, 1.0]");
        }
        if !in_range(self.min_allow_score) {
            anyhow::bail!(
                "security.external_knowledge_trust.min_allow_score must be in [0.0, 1.0]"
            );
        }
        if !in_range(self.min_sanitize_score) {
            anyhow::bail!(
                "security.external_knowledge_trust.min_sanitize_score must be in [0.0, 1.0]"
            );
        }
        if self.min_sanitize_score > self.min_allow_score {
            anyhow::bail!(
                "security.external_knowledge_trust.min_sanitize_score must be <= min_allow_score"
            );
        }
        for (prefix, score) in &self.source_overrides {
            if prefix.trim().is_empty() {
                anyhow::bail!(
                    "security.external_knowledge_trust.source_overrides contains empty prefix"
                );
            }
            if !in_range(*score) {
                anyhow::bail!(
                    "security.external_knowledge_trust.source_overrides[{prefix}] must be in [0.0, 1.0]"
                );
            }
        }
        Ok(())
    }
}

impl SecurityConfig {
    /// Validate security sub-configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when security thresholds are invalid.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.external_knowledge_trust.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::{ExternalKnowledgeTrustConfig, SecurityConfig};

    #[test]
    fn security_defaults_enable_trust() {
        let cfg = SecurityConfig::default();
        assert!(cfg.external_knowledge_trust.enabled);
        assert!((cfg.external_knowledge_trust.default_score - 0.60).abs() < f32::EPSILON);
    }

    #[test]
    fn trust_score_prefers_longest_prefix_match() {
        let cfg = ExternalKnowledgeTrustConfig {
            source_overrides: [
                ("gateway".to_string(), 0.4),
                ("gateway:webhook".to_string(), 0.9),
            ]
            .into_iter()
            .collect(),
            ..ExternalKnowledgeTrustConfig::default()
        };
        let score = cfg.score_for_source("gateway:webhook:partner");
        assert!((score - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn trust_score_uses_builtin_source_profile_when_no_override_matches() {
        let cfg = ExternalKnowledgeTrustConfig::default();
        let slack_score = cfg.score_for_source("channel:slack:workspace");
        assert!(slack_score > cfg.default_score);
        assert!((slack_score - 0.82).abs() < f32::EPSILON);
    }

    #[test]
    fn trust_config_rejects_invalid_threshold_order() {
        let cfg = ExternalKnowledgeTrustConfig {
            min_allow_score: 0.3,
            min_sanitize_score: 0.8,
            ..ExternalKnowledgeTrustConfig::default()
        };
        assert!(cfg.validate().is_err());
    }
}
