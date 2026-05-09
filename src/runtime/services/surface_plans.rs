use std::sync::Arc;

use anyhow::{Context, Result};

use super::bootstrap::{
    RuntimeServiceBootstrapOptions, SharedRuntimeServices, bootstrap_runtime_services,
};
use super::surface::{RuntimeSurfaceAssembly, RuntimeSurfaceResources};
use crate::config::Config;
use crate::contracts::channels::ChannelCapabilities;
use crate::runtime::RuntimeSandboxClass;

/// Canonical bind request for runtime network surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBindAddress {
    /// Host or interface to bind to.
    pub host: String,
    /// TCP port to bind to. `0` requests a random available port.
    pub port: u16,
}

impl RuntimeBindAddress {
    /// Build a bind address from host and port.
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// Render a short human-readable display string.
    #[must_use]
    pub fn display(&self) -> String {
        if self.port == 0 {
            format!("{} (random port)", self.host)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Runtime-owned startup plan for the HTTP gateway surface.
pub struct GatewaySurfacePlan {
    services: SharedRuntimeServices,
}

impl GatewaySurfacePlan {
    #[must_use]
    pub fn provider_name(&self) -> &str {
        self.services.provider_name()
    }

    #[must_use]
    pub fn model_name(&self) -> &str {
        self.services.model_name()
    }

    /// Compose the gateway runtime surface using runtime-owned startup policy.
    ///
    /// # Errors
    ///
    /// Returns an error if gateway runtime surface composition fails.
    pub async fn compose(&self, config: &Config) -> Result<RuntimeSurfaceResources> {
        let system_prompt =
            crate::transport::channels::gateway_base_prompt(Some(config.workspace_dir.as_path()));
        self.services
            .assemble_surface(
                config,
                RuntimeSurfaceAssembly {
                    system_prompt: &system_prompt,
                    temperature: config.default_temperature,
                    channel_capabilities: None,
                    warm_provider: false,
                    session_log_label: Some("gateway runtime"),
                },
            )
            .await
            .context("compose gateway runtime surface")
    }
}

/// Runtime-owned startup plan for real-time channel surfaces.
pub struct ChannelsSurfacePlan {
    services: SharedRuntimeServices,
    runtime_sandbox_class: RuntimeSandboxClass,
}

impl ChannelsSurfacePlan {
    #[must_use]
    pub fn provider_name(&self) -> &str {
        self.services.provider_name()
    }

    #[must_use]
    pub fn model_name(&self) -> &str {
        self.services.model_name()
    }

    #[must_use]
    pub fn auth_broker(&self) -> &crate::security::auth::AuthBroker {
        &self.services.auth_broker
    }

    #[must_use]
    pub fn security(&self) -> &Arc<crate::security::SecurityPolicy> {
        &self.services.security
    }

    #[must_use]
    pub fn runtime_sandbox_class(&self) -> RuntimeSandboxClass {
        self.runtime_sandbox_class
    }

    /// Compose the channels runtime surface using runtime-owned startup policy.
    ///
    /// # Errors
    ///
    /// Returns an error if channel runtime surface composition fails.
    pub async fn compose(
        &self,
        config: &Config,
        system_prompt: &str,
        channel_capabilities: Option<&ChannelCapabilities>,
    ) -> Result<RuntimeSurfaceResources> {
        self.services
            .assemble_surface(
                config,
                RuntimeSurfaceAssembly {
                    system_prompt,
                    temperature: config.default_temperature,
                    channel_capabilities,
                    warm_provider: true,
                    session_log_label: Some("channel metadata persistence"),
                },
            )
            .await
            .context("compose channel runtime surface")
    }
}

/// Prepare runtime-owned gateway startup services before transport wiring.
///
/// # Errors
///
/// Returns an error when gateway runtime bootstrap fails.
pub async fn prepare_gateway_surface_plan(config: &Config) -> Result<GatewaySurfacePlan> {
    let services = bootstrap_runtime_services(config, RuntimeServiceBootstrapOptions::default())
        .await
        .context("bootstrap gateway runtime services")?;
    tracing::info!(
        backend = services.memory.name(),
        "Gateway memory initialized"
    );
    Ok(GatewaySurfacePlan { services })
}

/// Prepare runtime-owned channels startup services before listener wiring.
///
/// # Errors
///
/// Returns an error when channels runtime bootstrap fails.
pub async fn prepare_channels_surface_plan(config: &Config) -> Result<ChannelsSurfacePlan> {
    let runtime_adapter =
        crate::runtime::create_runtime(&config.runtime).context("initialize channels runtime")?;
    let services = bootstrap_runtime_services(config, RuntimeServiceBootstrapOptions::default())
        .await
        .context("bootstrap channels runtime services")?;
    tracing::info!(
        backend = services.memory.name(),
        "Channel memory initialized"
    );
    Ok(ChannelsSurfacePlan {
        services,
        runtime_sandbox_class: runtime_adapter.sandbox_class(),
    })
}

/// Thin runtime surface entrypoint for agent execution.
///
/// # Errors
///
/// Returns an error if agent execution fails.
pub async fn run_agent_surface(
    config: Arc<Config>,
    request: crate::core::agent::RunRequest,
) -> Result<()> {
    let _runtime =
        crate::runtime::create_runtime(&config.runtime).context("initialize agent runtime")?;
    let services = bootstrap_runtime_services(
        &config,
        RuntimeServiceBootstrapOptions {
            provider_override: request.provider_override.as_deref(),
            model_override: request.model_override.as_deref(),
        },
    )
    .await
    .context("bootstrap agent runtime services")?;
    tracing::info!(backend = services.memory.name(), "Memory initialized");
    let observer = Arc::clone(&services.observer);

    let answer_provider = services
        .create_answer_provider(&config)
        .context("create resilient answer provider")?;
    let reflect_provider = services
        .create_reflect_provider(&config)
        .context("create reflect provider")?;
    let augmentor_provider = (config.persona.enable_llm_affect
        || config.persona.enable_llm_user_model)
        .then(|| services.create_auxiliary_provider(&config))
        .transpose()
        .context("create augmentor auxiliary provider")?;
    let surface = services
        .assemble_surface(
            &config,
            RuntimeSurfaceAssembly {
                system_prompt: &request.system_prompt,
                temperature: request.temperature,
                channel_capabilities: None,
                warm_provider: false,
                session_log_label: None,
            },
        )
        .await
        .context("compose agent runtime surface")?;

    crate::core::agent::run(
        config,
        request,
        crate::core::agent::RunContext {
            observer,
            provider_name: surface.provider_name,
            model_name: surface.model_name,
            security: surface.security,
            memory: surface.memory,
            answer_provider,
            reflect_provider,
            augmentor_provider,
            registry: surface.registry,
            rate_limiter: surface.rate_limiter,
            permission_store: surface.permission_store,
            subagent_manager: surface.subagents,
            skill_metadata_provider: surface.skill_metadata_provider,
        },
    )
    .await
}

/// Thin runtime surface entrypoint for gateway execution.
///
/// # Errors
///
/// Returns an error if gateway startup or serving fails.
pub async fn run_gateway_surface(config: Arc<Config>, bind: RuntimeBindAddress) -> Result<()> {
    crate::transport::gateway::run_gateway(&bind.host, bind.port, config).await
}

/// # Errors
/// Returns an error if channel startup fails.
pub async fn run_channels_surface(config: Arc<Config>) -> Result<()> {
    crate::transport::channels::run_channels_surface(config).await
}

/// Thin runtime surface entrypoint for daemon execution.
///
/// # Errors
///
/// Returns an error if daemon startup or supervision fails.
pub async fn run_daemon_surface(config: Arc<Config>, bind: RuntimeBindAddress) -> Result<()> {
    crate::platform::daemon::run(config, bind.host, bind.port).await
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{RuntimeBindAddress, prepare_channels_surface_plan, prepare_gateway_surface_plan};
    use crate::config::{Config, MemoryBackend};

    #[test]
    fn runtime_bind_address_display_formats_random_port() {
        assert_eq!(
            RuntimeBindAddress::new("127.0.0.1", 0).display(),
            "127.0.0.1 (random port)"
        );
        assert_eq!(
            RuntimeBindAddress::new("127.0.0.1", 3000).display(),
            "127.0.0.1:3000"
        );
    }

    #[tokio::test]
    async fn prepare_gateway_surface_plan_keeps_resolved_model_selection() {
        let temp = TempDir::new().expect("tempdir");
        let mut config = Config {
            workspace_dir: temp.path().join("workspace"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
        config.config_path = temp.path().join("config.toml");
        config.memory.backend = MemoryBackend::None;

        let expected = config.resolve_model(None, None);
        let plan = prepare_gateway_surface_plan(&config)
            .await
            .expect("prepare gateway surface plan");

        assert_eq!(plan.provider_name(), expected.provider);
        assert_eq!(plan.model_name(), expected.model);
    }

    #[tokio::test]
    async fn prepare_channels_surface_plan_keeps_runtime_sandbox_class() {
        let temp = TempDir::new().expect("tempdir");
        let mut config = Config {
            workspace_dir: temp.path().join("workspace"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
        config.config_path = temp.path().join("config.toml");
        config.memory.backend = MemoryBackend::None;

        let expected_runtime =
            crate::runtime::create_runtime(&config.runtime).expect("create runtime");
        let expected_selection = config.resolve_model(None, None);
        let plan = prepare_channels_surface_plan(&config)
            .await
            .expect("prepare channels surface plan");

        assert_eq!(plan.provider_name(), expected_selection.provider);
        assert_eq!(plan.model_name(), expected_selection.model);
        assert_eq!(
            plan.runtime_sandbox_class(),
            expected_runtime.sandbox_class()
        );
    }
}
