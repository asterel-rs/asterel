//! Taste evaluation configuration: backend selection, evaluation axes,
//! and text/UI evaluation toggles.

use serde::{Deserialize, Serialize};

/// Taste evaluation backend.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TasteBackend {
    /// LLM-based taste evaluation (default).
    #[default]
    Llm,
}

/// Taste evaluation configuration for output quality scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasteConfig {
    /// Whether taste evaluation is enabled. Default: true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Backend used for taste evaluation.
    #[serde(default)]
    pub backend: TasteBackend,
    /// Evaluation axes (e.g. coherence, hierarchy). Default: 3 axes.
    #[serde(default = "default_axes")]
    pub axes: Vec<String>,
    /// Enable text quality evaluation. Default: true.
    #[serde(default = "default_true")]
    pub text_enabled: bool,
    /// Enable UI/visual quality evaluation. Default: true.
    #[serde(default = "default_true")]
    pub ui_enabled: bool,
}

fn default_axes() -> Vec<String> {
    vec![
        "coherence".into(),
        "hierarchy".into(),
        "intentionality".into(),
    ]
}

use super::default_true;

impl Default for TasteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: TasteBackend::default(),
            axes: default_axes(),
            text_enabled: default_true(),
            ui_enabled: default_true(),
        }
    }
}

impl TasteConfig {
    /// # Errors
    ///
    /// Returns an error when taste configuration advertises runtime choices
    /// that the current taste engine does not implement.
    pub fn validate(&self) -> anyhow::Result<()> {
        let expected_axes = default_axes();
        if self.axes != expected_axes {
            anyhow::bail!(
                "taste.axes is not a runtime extension point yet; expected {:?}, got {:?}",
                expected_axes,
                self.axes
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taste_config_default() {
        let cfg = TasteConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.backend, TasteBackend::Llm);
        assert_eq!(cfg.axes.len(), 3);
        assert!(cfg.text_enabled);
        assert!(cfg.ui_enabled);
    }

    #[test]
    fn taste_config_toml_roundtrip() {
        let cfg = TasteConfig::default();
        let serialized = toml::to_string(&cfg).expect("serialize");
        let deserialized: TasteConfig = toml::from_str(&serialized).expect("deserialize");
        assert!(deserialized.enabled);
        assert_eq!(deserialized.backend, TasteBackend::Llm);
        assert_eq!(deserialized.axes.len(), 3);
        assert!(deserialized.text_enabled);
        assert!(deserialized.ui_enabled);
    }

    #[test]
    fn taste_config_rejects_custom_axes_until_runtime_supported() {
        let cfg = TasteConfig {
            axes: vec!["novelty".to_string()],
            ..TasteConfig::default()
        };

        let err = cfg.validate().expect_err("custom axes should fail closed");
        assert!(err.to_string().contains("taste.axes"));
    }
}
