use std::sync::Arc;

use anyhow::Result;

use super::SelfAmendmentCandidateReviewStore;
use super::bootstrap::SharedRuntimeServices;
use super::plugins::{runtime_extension_loader, runtime_skill_metadata_provider};
use super::provider_factory::{
    build_tool_registry, create_resilient_provider_box_with_credential_provider,
    create_resilient_provider_with_credential_provider, provider_selector_with_api_base,
};
use super::runtime_operational::load_runtime_operational_snapshot;
use crate::config::Config;
use crate::contracts::channels::ChannelCapabilities;
use crate::contracts::observability::Observer;
use crate::core::memory::Memory;
use crate::core::providers::Provider;
use crate::core::sessions::SessionOrchestrator;
use crate::core::sessions::types::SessionConfig;
use crate::core::subagents::{
    SkillMetadataProvider, SubagentDefaultRuntimeSpec, SubagentOrchestrator,
};
use crate::core::tools::ToolRegistry;
use crate::security::SecurityPolicy;

#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeSurfaceAssembly<'a> {
    pub system_prompt: &'a str,
    pub temperature: f64,
    pub channel_capabilities: Option<&'a ChannelCapabilities>,
    pub warm_provider: bool,
    pub session_log_label: Option<&'a str>,
}

#[derive(Clone)]
pub struct RuntimeSurfaceResources {
    pub provider_name: String,
    pub model_name: String,
    pub preferred_api_key: Option<String>,
    pub preferred_api_base: Option<String>,
    pub temperature: f64,
    pub security: Arc<SecurityPolicy>,
    pub memory: Arc<dyn Memory>,
    pub observer: Arc<dyn Observer>,
    pub rate_limiter: Arc<crate::security::policy::EntityRateLimiter>,
    pub permission_store: Arc<crate::security::PermissionStore>,
    pub provider: Arc<dyn Provider>,
    pub registry: Arc<ToolRegistry>,
    pub subagents: Arc<SubagentOrchestrator>,
    pub skill_metadata_provider: Arc<dyn SkillMetadataProvider>,
    pub sessions: Option<Arc<SessionOrchestrator>>,
    pub self_amendment_candidate_review: SelfAmendmentCandidateReviewStore,
}

impl RuntimeSurfaceResources {
    #[must_use]
    pub fn provider_name(&self) -> &str {
        self.provider_name.as_str()
    }

    #[must_use]
    pub fn model_name(&self) -> &str {
        self.model_name.as_str()
    }
}

impl SharedRuntimeServices {
    /// Build the primary resilient provider used by interactive/chat surfaces.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured provider cannot be created.
    pub fn create_resilient_provider(&self, config: &Config) -> Result<Arc<dyn Provider>> {
        let provider_selector =
            provider_selector_with_api_base(self.provider_name(), self.preferred_api_base());
        create_resilient_provider_with_credential_provider(
            config,
            &self.auth_broker,
            &self.security,
            &provider_selector,
            self.provider_name(),
            self.preferred_api_key(),
        )
    }

    /// Build a resilient provider as a boxed trait object.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured provider cannot be created.
    pub fn create_answer_provider(&self, config: &Config) -> Result<Box<dyn Provider>> {
        let provider_selector =
            provider_selector_with_api_base(self.provider_name(), self.preferred_api_base());
        create_resilient_provider_box_with_credential_provider(
            config,
            &self.auth_broker,
            &self.security,
            &provider_selector,
            self.provider_name(),
            self.preferred_api_key(),
        )
    }

    /// Build the resilient reflect provider for follow-up inference.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured provider cannot be created.
    pub fn create_reflect_provider(&self, config: &Config) -> Result<Box<dyn Provider>> {
        let provider_selector =
            provider_selector_with_api_base(self.provider_name(), self.preferred_api_base());
        create_resilient_provider_box_with_credential_provider(
            config,
            &self.auth_broker,
            &self.security,
            &provider_selector,
            self.provider_name(),
            self.preferred_api_key(),
        )
    }

    /// Build the lighter-weight auxiliary provider used by augmentor-side
    /// LLM helpers such as affect detection and user modeling.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured provider cannot be created.
    pub fn create_auxiliary_provider(&self, config: &Config) -> Result<Arc<dyn Provider>> {
        Ok(Arc::from(self.create_reflect_provider(config)?))
    }

    /// Build the shared tool registry for a runtime surface.
    #[must_use]
    pub fn build_tool_registry(
        &self,
        config: &Config,
        channel_capabilities: Option<&ChannelCapabilities>,
    ) -> Arc<ToolRegistry> {
        build_tool_registry(
            config,
            &self.security,
            &self.memory,
            Some(&self.auth_broker),
            &self.model_selection,
            channel_capabilities,
        )
    }

    /// # Errors
    /// Returns an error if the surface provider or subagent runtime cannot be composed.
    pub async fn assemble_surface(
        &self,
        config: &Config,
        assembly: RuntimeSurfaceAssembly<'_>,
    ) -> Result<RuntimeSurfaceResources> {
        let skill_metadata_provider = runtime_skill_metadata_provider();
        let provider = self.create_resilient_provider(config)?;
        if assembly.warm_provider
            && let Err(error) = provider.warmup().await
        {
            tracing::warn!(%error, "Provider warmup failed (non-fatal)");
        }

        let registry = self.build_tool_registry(config, assembly.channel_capabilities);
        let subagents = self.build_subagents(
            config,
            assembly.system_prompt,
            assembly.temperature,
            Arc::clone(&provider),
            Arc::clone(&registry),
            Arc::clone(&skill_metadata_provider),
        )?;
        let sessions = match assembly.session_log_label {
            Some(label) => self.connect_sessions(config, label).await,
            None => None,
        };

        Ok(RuntimeSurfaceResources {
            provider_name: self.provider_name().to_string(),
            model_name: self.model_name().to_string(),
            preferred_api_key: self.model_selection.api_key.clone(),
            preferred_api_base: self.model_selection.api_base.clone(),
            temperature: assembly.temperature,
            security: Arc::clone(&self.security),
            memory: Arc::clone(&self.memory),
            observer: Arc::clone(&self.observer),
            rate_limiter: Arc::clone(&self.rate_limiter),
            permission_store: Arc::clone(&self.permission_store),
            provider,
            registry,
            subagents,
            skill_metadata_provider,
            sessions,
            self_amendment_candidate_review: self.self_amendment_candidate_review.clone(),
        })
    }

    fn build_subagents(
        &self,
        config: &Config,
        system_prompt: &str,
        temperature: f64,
        provider: Arc<dyn Provider>,
        registry: Arc<ToolRegistry>,
        skill_metadata_provider: Arc<dyn SkillMetadataProvider>,
    ) -> Result<Arc<SubagentOrchestrator>> {
        SubagentOrchestrator::configured_default(SubagentDefaultRuntimeSpec {
            config,
            system_prompt,
            model_name: self.model_name(),
            temperature,
            security: self.security.as_ref(),
            provider,
            registry,
            extension_loader: runtime_extension_loader(),
            skill_metadata_provider,
        })
    }

    async fn connect_sessions(
        &self,
        config: &Config,
        session_log_label: &str,
    ) -> Option<Arc<SessionOrchestrator>> {
        let session_support = load_runtime_operational_snapshot(config).session_persistence;
        if !session_support.is_runtime_required() {
            tracing::info!(
                surface = session_log_label,
                reason = session_support
                    .reason
                    .as_deref()
                    .unwrap_or("runtime session persistence unsupported"),
                "skipping runtime session persistence"
            );
            return None;
        }

        let sessions_db_path = config.workspace_dir.join("sessions.db");
        match SessionOrchestrator::connect(&sessions_db_path, SessionConfig::default()).await {
            Ok(sessions) => Some(Arc::new(sessions)),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %sessions_db_path.display(),
                    surface = session_log_label,
                    "failed to initialize runtime session persistence"
                );
                None
            }
        }
    }
}
