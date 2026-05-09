//! Shared onboarding config draft + finalization helpers.
//!
//! Centralizes config assembly and post-actions so CLI onboarding and
//! quick setup do not duplicate config defaults and persistence.

use std::path::PathBuf;

use anyhow::Result;

use super::auth_profile::upsert_onboard_auth_profile;
use super::prompts::ProjectContext;
use super::scaffold::scaffold_workspace;
use crate::config::{
    ChannelsConfig, ComposioConfig, Config, MemoryConfig, SecretsConfig, TunnelConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardingAuthProfileDraft {
    pub provider: String,
    pub api_key: String,
    pub oauth_source: Option<String>,
}

impl OnboardingAuthProfileDraft {
    #[must_use]
    pub(crate) fn from_optional(
        provider: impl Into<String>,
        api_key: impl Into<String>,
        oauth_source: Option<String>,
    ) -> Option<Self> {
        let api_key = api_key.into();
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            return None;
        }

        Some(Self {
            provider: provider.into(),
            api_key: trimmed.to_string(),
            oauth_source,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OnboardingConfigDraft {
    pub workspace_dir: PathBuf,
    pub config_path: PathBuf,
    pub api_key: Option<String>,
    pub default_provider: String,
    pub default_model: String,
    pub channels_config: ChannelsConfig,
    pub memory: MemoryConfig,
    pub tunnel: TunnelConfig,
    pub composio: ComposioConfig,
    pub secrets: SecretsConfig,
    pub locale: String,
}

impl OnboardingConfigDraft {
    #[must_use]
    pub(crate) fn build_config(self) -> Config {
        Config {
            workspace_dir: self.workspace_dir,
            config_path: self.config_path,
            api_key: self.api_key,
            default_provider: Some(self.default_provider),
            default_model: Some(self.default_model),
            channels_config: self.channels_config,
            memory: self.memory,
            tunnel: self.tunnel,
            composio: self.composio,
            secrets: self.secrets,
            locale: self.locale,
            ..Config::default()
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OnboardingPlan {
    pub draft: OnboardingConfigDraft,
    pub project_context: ProjectContext,
    pub auth_profile: Option<OnboardingAuthProfileDraft>,
}

impl OnboardingPlan {
    /// # Errors
    ///
    /// Returns an error when scaffolding, config persistence, or auth-profile
    /// persistence fails.
    pub(crate) fn finalize(self) -> Result<Config> {
        std::fs::create_dir_all(&self.draft.workspace_dir)?;
        scaffold_workspace(&self.draft.workspace_dir, &self.project_context)?;

        let config = self.draft.build_config();
        config.save()?;

        if let Some(auth_profile) = self.auth_profile {
            upsert_onboard_auth_profile(
                &config,
                &auth_profile.provider,
                &auth_profile.api_key,
                auth_profile.oauth_source,
            )?;
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{OnboardingAuthProfileDraft, OnboardingConfigDraft, OnboardingPlan};
    use crate::config::{ComposioConfig, Config, MemoryConfig, SecretsConfig, TunnelConfig};
    use crate::onboard::prompts::ProjectContext;
    use crate::security::auth::AuthProfileStore;

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    fn project_context() -> ProjectContext {
        ProjectContext {
            user_name: "User".to_string(),
            timezone: "UTC".to_string(),
            agent_name: "Asterel".to_string(),
            communication_style: "Be clear.".to_string(),
        }
    }

    #[test]
    fn onboarding_draft_builds_config_from_default_base() {
        let tmp = TempDir::new().expect("temp dir");
        let workspace_dir = tmp.path().join("workspace");
        let config_path = tmp.path().join("config.toml");
        let default_config = Config::default();

        let config = OnboardingConfigDraft {
            workspace_dir: workspace_dir.clone(),
            config_path: config_path.clone(),
            api_key: Some("sk-test".to_string()),
            default_provider: "openai".to_string(),
            default_model: "gpt-5.4".to_string(),
            channels_config: crate::config::ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            tunnel: TunnelConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            locale: "ja".to_string(),
        }
        .build_config();

        assert_eq!(config.workspace_dir, workspace_dir);
        assert_eq!(config.config_path, config_path);
        assert_eq!(config.default_provider.as_deref(), Some("openai"));
        assert_eq!(config.default_model.as_deref(), Some("gpt-5.4"));
        assert_eq!(config.locale, "ja");
        assert_f64_eq(
            config.default_temperature,
            default_config.default_temperature,
        );
        assert_eq!(
            config.gateway.allow_public_bind,
            default_config.gateway.allow_public_bind
        );
    }

    #[test]
    fn finalize_persists_config_and_auth_profile() {
        let tmp = TempDir::new().expect("temp dir");
        let workspace_dir = tmp.path().join("workspace");
        let config_path = tmp.path().join("config.toml");

        let plan = OnboardingPlan {
            draft: OnboardingConfigDraft {
                workspace_dir: workspace_dir.clone(),
                config_path: config_path.clone(),
                api_key: Some("sk-test".to_string()),
                default_provider: "openai".to_string(),
                default_model: "gpt-5.4".to_string(),
                channels_config: crate::config::ChannelsConfig::default(),
                memory: MemoryConfig::default(),
                tunnel: TunnelConfig::default(),
                composio: ComposioConfig::default(),
                secrets: SecretsConfig {
                    encrypt: false,
                    ..SecretsConfig::default()
                },
                locale: "en".to_string(),
            },
            project_context: project_context(),
            auth_profile: OnboardingAuthProfileDraft::from_optional("openai", "sk-test", None),
        };

        let config = plan.finalize().expect("finalize should succeed");

        assert!(config_path.exists());
        assert!(workspace_dir.join("BOOTSTRAP.md").exists());

        let store = AuthProfileStore::load_or_init_cfg(&config).expect("load auth store");
        assert!(
            store
                .profiles
                .iter()
                .any(|profile| profile.provider == "openai"
                    && profile.api_key.as_deref() == Some("sk-test"))
        );
    }
}
