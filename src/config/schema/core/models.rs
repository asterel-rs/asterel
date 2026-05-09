//! Model list registry, skill source priorities, and provider
//! resolution logic for `resolve_model`.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::contracts::providers::{is_builtin_provider, normalize_provider_alias};

use super::types::{Config, DEFAULT_MODEL, DEFAULT_PROVIDER, default_true};

/// A named model alias in the model registry.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelListEntry {
    /// User-facing alias name (referenced by `default_model`).
    pub model_name: String,
    /// Provider/model identifier (e.g. `"openai/gpt-4o"`).
    pub model: String,
    /// Optional API key override for this model.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Optional API base URL for custom endpoints.
    #[serde(default)]
    pub api_base: Option<String>,
}

impl std::fmt::Debug for ModelListEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelListEntry")
            .field("model_name", &self.model_name)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("api_base", &self.api_base)
            .finish()
    }
}

/// Skill discovery source location.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// Skills from the workspace `skills/` directory.
    Workspace,
    /// Skills from configured extra directories.
    ExtraDirs,
    /// Skills from the `OpenSkills` registry.
    OpenSkills,
}

fn default_skill_source_priority() -> Vec<SkillSource> {
    vec![
        SkillSource::Workspace,
        SkillSource::ExtraDirs,
        SkillSource::OpenSkills,
    ]
}

fn default_skill_prompt_description_chars() -> usize {
    96
}

fn default_skill_turn_hint_limit() -> usize {
    4
}

/// Skills runtime discovery and loading configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillsRuntimeConfig {
    /// Ordered skill source search priority.
    #[serde(default = "default_skill_source_priority")]
    pub source_priority: Vec<SkillSource>,
    /// Additional directories to search for skills.
    #[serde(default)]
    pub extra_dirs: Vec<String>,
    /// Installed skills disabled by operator policy.
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    /// Enforce skill requirement declarations. Default: true.
    #[serde(default = "default_true")]
    pub enforce_requirements: bool,
    /// Watch skill directories for changes and auto-reload. Default: true.
    #[serde(default = "default_true")]
    pub watch_refresh: bool,
    /// Maximum description characters shown for each skill in prompt catalogs.
    #[serde(default = "default_skill_prompt_description_chars")]
    pub prompt_description_chars: usize,
    /// Maximum number of relevant skills highlighted per turn. `0` disables turn hints.
    #[serde(default = "default_skill_turn_hint_limit")]
    pub turn_hint_limit: usize,
}

impl Default for SkillsRuntimeConfig {
    fn default() -> Self {
        Self {
            source_priority: default_skill_source_priority(),
            extra_dirs: Vec::new(),
            disabled_skills: Vec::new(),
            enforce_requirements: true,
            watch_refresh: true,
            prompt_description_chars: default_skill_prompt_description_chars(),
            turn_hint_limit: default_skill_turn_hint_limit(),
        }
    }
}

/// Fully resolved model selection after alias and override resolution.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedModelSelection {
    /// Resolved provider name (may be `"custom:URL"` for custom endpoints).
    pub provider: String,
    /// Resolved model identifier.
    pub model: String,
    /// API key from the model registry entry, if any.
    pub api_key: Option<String>,
    /// API base URL from the model registry entry, if any.
    pub api_base: Option<String>,
}

impl std::fmt::Debug for ResolvedModelSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedModelSelection")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("api_base", &self.api_base)
            .finish()
    }
}

fn parse_model_ref(model_ref: &str) -> Option<(&str, &str)> {
    let trimmed = model_ref.trim();
    let (provider, model) = trimmed.split_once('/')?;
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider, model))
}

fn is_static_provider(provider: &str) -> bool {
    is_builtin_provider(provider) || matches!(provider, "custom" | "anthropic-custom")
}

fn canonical_provider_selector_backend(provider: &str) -> &str {
    normalize_provider_alias(provider)
}

/// Extracts the provider string from a model list entry.
///
/// # Errors
///
/// Returns an error if the model string is not in `provider/model`
/// format or if the provider is unknown and no `api_base` is set.
pub(super) fn provider_from_model_entry(entry: &ModelListEntry) -> Result<String> {
    let (provider, _) = parse_model_ref(&entry.model)
        .ok_or_else(|| anyhow::anyhow!("model_list model must use provider/model format"))?;

    if is_static_provider(provider) {
        return Ok(provider.to_string());
    }

    let Some(base_url) = entry
        .api_base
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        anyhow::bail!(
            "model_list provider '{provider}' requires api_base for custom openai-compatible routing"
        );
    };

    Ok(format!("custom:{base_url}"))
}

impl Config {
    /// Resolve the effective provider, model, API key, and base URL
    /// by applying overrides and model list alias lookups.
    pub fn resolve_model(
        &self,
        provider_override: Option<&str>,
        model_override: Option<&str>,
    ) -> ResolvedModelSelection {
        let default_provider = self.default_provider.as_deref().unwrap_or(DEFAULT_PROVIDER);
        let selected_provider = provider_override.unwrap_or(default_provider).to_string();
        let selected_model = model_override
            .or(self.default_model.as_deref())
            .unwrap_or(DEFAULT_MODEL)
            .to_string();

        let Some(entry) = self
            .model_list
            .iter()
            .find(|entry| entry.model_name == selected_model)
        else {
            return ResolvedModelSelection {
                provider: selected_provider,
                model: selected_model,
                api_key: None,
                api_base: None,
            };
        };

        let Some((_, model_id)) = parse_model_ref(&entry.model) else {
            return ResolvedModelSelection {
                provider: selected_provider,
                model: selected_model,
                api_key: None,
                api_base: None,
            };
        };

        let provider = if provider_override.is_some() {
            selected_provider
        } else {
            let alias_provider =
                provider_from_model_entry(entry).unwrap_or_else(|_| default_provider.to_string());
            if canonical_provider_selector_backend(&selected_provider)
                == canonical_provider_selector_backend(&alias_provider)
            {
                selected_provider
            } else {
                alias_provider
            }
        };

        ResolvedModelSelection {
            provider,
            model: model_id.to_string(),
            api_key: entry.api_key.clone(),
            api_base: entry
                .api_base
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        }
    }

    /// Validate all model list entries for correct format and
    /// consistency with `default_provider`.
    ///
    /// # Errors
    ///
    /// Returns an error if any entry has an empty name/model, uses
    /// an invalid provider/model format, or conflicts with
    /// `default_provider`.
    pub(super) fn validate_model_list_registry(&self) -> Result<()> {
        for entry in &self.model_list {
            if entry.model_name.trim().is_empty() {
                anyhow::bail!("model_list model_name cannot be empty");
            }
            if entry.model.trim().is_empty() {
                anyhow::bail!("model_list model cannot be empty");
            }
            let _ = provider_from_model_entry(entry)?;
        }

        if let Some(default_model) = self.default_model.as_deref()
            && let Some(entry) = self
                .model_list
                .iter()
                .find(|entry| entry.model_name == default_model)
        {
            let provider = provider_from_model_entry(entry)?;
            if let Some(default_provider) = self.default_provider.as_deref()
                && !default_provider.trim().is_empty()
                && !provider.starts_with("custom:")
                && canonical_provider_selector_backend(default_provider)
                    != canonical_provider_selector_backend(&provider)
            {
                anyhow::bail!(
                    "default_provider ('{default_provider}') conflicts with model_list alias provider ('{provider}')"
                );
            }
        }

        Ok(())
    }
}
