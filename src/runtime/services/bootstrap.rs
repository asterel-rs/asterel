use std::sync::Arc;

use anyhow::{Context, Result};

use crate::config::{Config, MemoryConfig};
use crate::contracts::observability::Observer;
use crate::core::memory::{self, Memory};
use crate::runtime::observability::create_observer;
use crate::security::auth::AuthBroker;
use crate::security::policy::EntityRateLimiter;
use crate::security::{PermissionStore, SecurityPolicy};

use super::SelfAmendmentCandidateReviewStore;

/// Serializable snapshot of a resolved provider/model choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeModelSelection {
    /// Selected provider name.
    pub provider: String,
    /// Selected model identifier.
    pub model: String,
    /// Optional model-registry API key override.
    pub api_key: Option<String>,
    /// Optional model-registry API base override.
    pub api_base: Option<String>,
}

/// Shared bootstrap options for runtime surfaces.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeServiceBootstrapOptions<'a> {
    /// Optional provider override.
    pub provider_override: Option<&'a str>,
    /// Optional model override.
    pub model_override: Option<&'a str>,
}

/// Common runtime services shared by the agent, gateway, and channels.
#[derive(Clone)]
pub struct SharedRuntimeServices {
    /// Loaded auth broker for provider and memory key resolution.
    pub auth_broker: AuthBroker,
    /// Resolved provider/model choice for this surface.
    pub model_selection: RuntimeModelSelection,
    /// Security policy derived from config/runtime settings.
    pub security: Arc<SecurityPolicy>,
    /// Shared memory backend.
    pub memory: Arc<dyn Memory>,
    /// Shared observability observer.
    pub observer: Arc<dyn Observer>,
    /// Shared entity/action rate limiter.
    pub rate_limiter: Arc<EntityRateLimiter>,
    /// Shared permission grant cache/store.
    pub permission_store: Arc<PermissionStore>,
    /// Ephemeral dry-run self-amendment review buffer.
    pub self_amendment_candidate_review: SelfAmendmentCandidateReviewStore,
}

impl SharedRuntimeServices {
    /// Borrow the selected provider name.
    #[must_use]
    pub fn provider_name(&self) -> &str {
        self.model_selection.provider.as_str()
    }

    /// Borrow the selected model name.
    #[must_use]
    pub fn model_name(&self) -> &str {
        self.model_selection.model.as_str()
    }

    /// Borrow the selected API key override, if any.
    #[must_use]
    pub fn preferred_api_key(&self) -> Option<&str> {
        self.model_selection.api_key.as_deref()
    }

    /// Borrow the selected API base override, if any.
    #[must_use]
    pub fn preferred_api_base(&self) -> Option<&str> {
        self.model_selection.api_base.as_deref()
    }
}

/// Bootstrap the shared services required by a runtime surface.
///
/// # Errors
///
/// Returns an error when auth broker or memory initialization fails.
pub async fn bootstrap_runtime_services(
    config: &Config,
    options: RuntimeServiceBootstrapOptions<'_>,
) -> Result<SharedRuntimeServices> {
    crate::utils::http::sync_runtime_http_proxy(config.network.proxy.as_deref())
        .context("apply runtime network proxy")?;
    let auth_broker = AuthBroker::load_or_init(config).context("load auth broker")?;
    let resolved = config.resolve_model(options.provider_override, options.model_override);
    let model_selection = RuntimeModelSelection {
        provider: resolved.provider,
        model: resolved.model,
        api_key: resolved.api_key,
        api_base: resolved.api_base,
    };
    let security = Arc::new(SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    ));
    let memory = init_memory(config, &auth_broker, &config.memory)
        .await
        .context("initialize runtime memory backend")?;
    let observer: Arc<dyn Observer> = Arc::from(create_observer(&config.observability));
    let rate_limiter = Arc::new(EntityRateLimiter::new_with_scopes(
        config.autonomy.max_actions_per_hour,
        config.autonomy.max_actions_per_entity_per_hour,
        config.autonomy.max_actions_per_conversation_per_hour,
        config.autonomy.max_actions_per_workspace_per_hour,
        config.autonomy.burst_max_per_entity,
        config.autonomy.burst_window_secs,
    ));
    let permission_store = Arc::new(PermissionStore::load(&config.workspace_dir));

    Ok(SharedRuntimeServices {
        auth_broker,
        model_selection,
        security,
        memory,
        observer,
        rate_limiter,
        permission_store,
        self_amendment_candidate_review: SelfAmendmentCandidateReviewStore::for_workspace(
            &config.workspace_dir,
            100,
        ),
    })
}

/// Bootstrap only the shared runtime memory backend using the canonical auth-broker path.
///
/// # Errors
///
/// Returns an error when auth-broker or memory initialization fails.
pub async fn bootstrap_runtime_memory(config: &Config) -> Result<Arc<dyn Memory>> {
    let auth_broker = AuthBroker::load_or_init(config).context("load auth broker")?;
    init_memory(config, &auth_broker, &config.memory)
        .await
        .context("initialize runtime memory backend")
}

async fn init_memory(
    config: &Config,
    auth_broker: &AuthBroker,
    memory_config: &MemoryConfig,
) -> Result<Arc<dyn Memory>> {
    let memory_api_key = auth_broker.resolve_memory_api_key(memory_config);
    Ok(Arc::from(
        memory::create_memory(
            memory_config,
            &config.workspace_dir,
            memory_api_key.as_deref(),
        )
        .await
        .context("create memory backend")?,
    ))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{RuntimeServiceBootstrapOptions, bootstrap_runtime_services};
    use crate::config::{Config, MemoryBackend};

    #[tokio::test]
    async fn bootstrap_runtime_services_respects_model_overrides() {
        let temp = TempDir::new().expect("tempdir");
        let mut config = Config {
            workspace_dir: temp.path().join("workspace"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
        config.config_path = temp.path().join("config.toml");
        config.memory.backend = MemoryBackend::None;

        let services = bootstrap_runtime_services(
            &config,
            RuntimeServiceBootstrapOptions {
                provider_override: Some("openai"),
                model_override: Some("gpt-5-mini"),
            },
        )
        .await
        .expect("bootstrap runtime services");

        assert_eq!(services.provider_name(), "openai");
        assert_eq!(services.model_name(), "gpt-5-mini");
        assert_eq!(services.security.workspace_dir, config.workspace_dir);
    }
}
