//! `ChannelRuntime` initialization: constructs the shared runtime context
//! (provider, memory, tools, sessions, security) for all channel listeners.
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use super::super::factory;
use super::super::policy::ChannelPolicy;
use super::super::traits::{Channel, ChannelCapabilities, SurfaceRealizationPolicy};
use super::prompt::{build_channel_capabilities_section, build_channel_system_prompt};
use crate::config::Config;
use crate::core::memory::Memory;
use crate::core::providers::{Provider, ThinkingLevel};
use crate::core::sessions::SessionOrchestrator;
use crate::core::subagents::SubagentOrchestrator;
use crate::core::tools::channel::ChannelActionBroker;
use crate::core::tools::registry::ToolRegistry;
use crate::media::MediaStore;
use crate::plugins::skills::{SkillMetadata, SkillMetadataSnapshot};
use crate::runtime::RuntimeSandboxClass;
use crate::runtime::services::{
    ChannelsSurfacePlan, create_resilient_provider_with_credential_provider,
    prepare_channels_surface_plan, provider_selector_with_api_base,
};
use crate::security::auth::AuthBroker;
use crate::security::policy::EntityRateLimiter;
use crate::security::{PermissionStore, SecurityPolicy};

#[derive(Debug, Clone, Copy)]
pub(in super::super) struct ChannelThinkingState {
    pub(in super::super) thinking_level: ThinkingLevel,
    pub(in super::super) show_reasoning: bool,
}

impl Default for ChannelThinkingState {
    fn default() -> Self {
        Self {
            thinking_level: ThinkingLevel::Off,
            show_reasoning: false,
        }
    }
}

impl ChannelThinkingState {
    pub(in super::super) fn from_config(config: &crate::config::Config) -> Self {
        Self {
            thinking_level: config.inference.default_thinking_level,
            show_reasoning: false,
        }
    }
}

pub(in super::super) struct ChannelRuntime {
    pub(in super::super) config: Arc<Config>,
    pub(in super::super) security: Arc<SecurityPolicy>,
    pub(in super::super) provider: Arc<dyn Provider>,
    pub(in super::super) registry: Arc<ToolRegistry>,
    pub(in super::super) subagent_manager: Arc<SubagentOrchestrator>,
    pub(in super::super) rate_limiter: Arc<EntityRateLimiter>,
    pub(in super::super) permission_store: Arc<PermissionStore>,
    pub(in super::super) model: String,
    pub(in super::super) temperature: f64,
    pub(in super::super) channel_inference: HashMap<String, ChannelInferenceTarget>,
    pub(in super::super) mem: Arc<dyn Memory>,
    pub(in super::super) observer: Arc<dyn crate::contracts::observability::Observer>,
    pub(in super::super) self_amendment_candidate_review:
        crate::runtime::services::SelfAmendmentCandidateReviewStore,
    pub(in super::super) tenant_policy_context: crate::security::policy::TenantPolicyContext,
    pub(in super::super) media_store: Option<Arc<MediaStore>>,
    pub(in super::super) channel_capabilities_by_name: HashMap<String, ChannelCapabilities>,
    pub(in super::super) channel_surface_policies_by_name:
        HashMap<String, SurfaceRealizationPolicy>,
    pub(in super::super) channel_capabilities_section: Option<String>,
    pub(in super::super) channels: Vec<Arc<dyn Channel>>,
    pub(in super::super) channel_policies: HashMap<String, ChannelPolicy>,
    pub(in super::super) channel_action_brokers: HashMap<String, Arc<dyn ChannelActionBroker>>,
    pub(in super::super) thinking_states:
        Arc<tokio::sync::RwLock<HashMap<String, ChannelThinkingState>>>,
    pub(in super::super) session_manager: Option<Arc<SessionOrchestrator>>,
    pub(in super::super) runtime_sandbox_class: RuntimeSandboxClass,
}

#[derive(Clone)]
pub(in super::super) struct ChannelInferenceTarget {
    pub(in super::super) provider: Arc<dyn Provider>,
    pub(in super::super) model: String,
}

struct ChannelRuntimeBuilder {
    config: Arc<Config>,
    surface_plan: ChannelsSurfacePlan,
}

impl ChannelRuntimeBuilder {
    async fn bootstrap(config: &Arc<Config>) -> Result<Self> {
        let surface_plan = prepare_channels_surface_plan(config.as_ref()).await?;
        Ok(Self {
            config: Arc::clone(config),
            surface_plan,
        })
    }

    async fn init_channel_inference(
        &self,
        channels: &[Arc<dyn Channel>],
        default_provider_name: &str,
        default_api_key: Option<&str>,
        default_api_base: Option<&str>,
        default_provider: &Arc<dyn Provider>,
    ) -> Result<HashMap<String, ChannelInferenceTarget>> {
        init_channel_inference_overrides(
            &self.config,
            channels,
            self.surface_plan.auth_broker(),
            self.surface_plan.security(),
            default_provider_name,
            default_api_key,
            default_api_base,
            default_provider,
        )
        .await
    }

    async fn build(self) -> Result<ChannelRuntime> {
        let media_store = create_media_store(&self.config)?;
        let workspace = self.config.workspace_dir.clone();
        let skill_snapshot =
            load_channel_skill_snapshot(&self.config, self.surface_plan.security());
        let prompt_skill_entries = skill_snapshot.search_index().prompt_index_entries();
        let (channels, channel_policies) =
            build_channels_with_policies(&self.config, self.surface_plan.security());
        let channel_capabilities_by_name = channels
            .iter()
            .map(|channel| (channel.name().to_string(), channel.capabilities()))
            .collect::<HashMap<_, _>>();
        let channel_surface_policies_by_name = channels
            .iter()
            .map(|channel| {
                (
                    channel.name().to_string(),
                    channel.surface_realization_policy(),
                )
            })
            .collect::<HashMap<_, _>>();
        let channel_capabilities_section = build_channel_capabilities_section(&channels);

        let best_channel = most_capable_channel(&channels);
        let channel_capabilities = best_channel.as_ref().map(|(_name, caps)| caps);
        let channel_action_brokers = build_channel_action_brokers(&self.config, &channels);

        let system_prompt = build_channel_system_prompt(
            &self.config,
            &workspace,
            self.surface_plan.model_name(),
            &channels,
            &prompt_skill_entries,
            self.surface_plan.security(),
        );
        let surface = self
            .surface_plan
            .compose(&self.config, &system_prompt, channel_capabilities)
            .await?;

        let channel_inference = self
            .init_channel_inference(
                &channels,
                surface.provider_name(),
                surface.preferred_api_key.as_deref(),
                surface.preferred_api_base.as_deref(),
                &surface.provider,
            )
            .await?;

        announce_loaded_skills(skill_snapshot.metadata());

        Ok(ChannelRuntime {
            config: Arc::clone(&self.config),
            security: surface.security,
            provider: surface.provider,
            registry: surface.registry,
            subagent_manager: surface.subagents,
            rate_limiter: surface.rate_limiter,
            permission_store: surface.permission_store,
            model: surface.model_name,
            temperature: surface.temperature,
            channel_inference,
            mem: surface.memory,
            observer: surface.observer,
            self_amendment_candidate_review: surface.self_amendment_candidate_review,
            tenant_policy_context:
                crate::transport::channels::ingress_policy::channel_runtime_policy_context(),
            media_store,
            channel_capabilities_by_name,
            channel_surface_policies_by_name,
            channel_capabilities_section,
            channels,
            channel_policies,
            channel_action_brokers,
            thinking_states: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            session_manager: surface.sessions,
            runtime_sandbox_class: self.surface_plan.runtime_sandbox_class(),
        })
    }
}

fn capability_score(caps: &ChannelCapabilities) -> usize {
    [
        caps.can_edit_message,
        caps.can_delete_message,
        caps.can_send_media,
        caps.can_send_embed,
        caps.can_send_typing,
        caps.can_create_thread,
        caps.can_manage_thread_members,
        caps.can_add_reaction,
        caps.can_read_reactions,
        caps.can_send_buttons,
        caps.can_send_select_menu,
        caps.can_send_modal,
        caps.can_fetch_history,
        caps.can_receive_reactions,
        caps.can_receive_edits,
        caps.can_receive_deletes,
        caps.can_receive_typing,
    ]
    .into_iter()
    .filter(|flag| *flag)
    .count()
}

fn most_capable_channel(channels: &[Arc<dyn Channel>]) -> Option<(String, ChannelCapabilities)> {
    channels
        .iter()
        .map(|channel| (channel.name().to_string(), channel.capabilities()))
        .filter(|(_name, capabilities)| capability_score(capabilities) > 0)
        .max_by_key(|(_name, capabilities)| capability_score(capabilities))
}

fn make_channel_action_broker(
    config: &Config,
    channel_name: &str,
) -> Option<Arc<dyn ChannelActionBroker>> {
    #[cfg(feature = "discord")]
    if channel_name == "discord"
        && let Some(discord) = config.channels_config.discord.as_ref()
    {
        return Some(Arc::new(
            crate::transport::channels::discord::DiscordActionBroker::new(&discord.bot_token),
        ));
    }

    let _ = config;
    let _ = channel_name;
    None
}

fn build_channel_action_brokers(
    config: &Config,
    channels: &[Arc<dyn Channel>],
) -> HashMap<String, Arc<dyn ChannelActionBroker>> {
    channels
        .iter()
        .filter_map(|channel| {
            make_channel_action_broker(config, channel.name())
                .map(|broker| (channel.name().to_string(), broker))
        })
        .collect()
}

fn split_provider_model_ref(model_ref: &str) -> Option<(&str, &str)> {
    let trimmed = model_ref.trim();
    let (provider, model) = trimmed.split_once('/')?;
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider, model))
}

fn lookup_channel_override(config: &Config, key: &str) -> Option<String> {
    config.channels_config.model_by_channel.get(key).cloned()
}

fn default_account_for_channel(config: &Config, channel_name: &str) -> Option<String> {
    match channel_name {
        "telegram" => config
            .channels_config
            .telegram
            .as_ref()
            .and_then(|cfg| cfg.default_account.clone()),
        "discord" => config
            .channels_config
            .discord
            .as_ref()
            .and_then(|cfg| cfg.default_account.clone()),
        "slack" => config
            .channels_config
            .slack
            .as_ref()
            .and_then(|cfg| cfg.default_account.clone()),
        _ => None,
    }
}

fn resolve_channel_model_override(config: &Config, channel_name: &str) -> Option<String> {
    if let Some(account) = default_account_for_channel(config, channel_name) {
        let colon_key = format!("{channel_name}:{account}");
        if let Some(model) = lookup_channel_override(config, &colon_key) {
            return Some(model);
        }
    }
    lookup_channel_override(config, channel_name)
}

async fn init_channel_inference_overrides(
    config: &Arc<Config>,
    channels: &[Arc<dyn Channel>],
    auth_broker: &AuthBroker,
    security: &Arc<SecurityPolicy>,
    default_provider_name: &str,
    default_api_key: Option<&str>,
    default_api_base: Option<&str>,
    default_provider: &Arc<dyn Provider>,
) -> Result<HashMap<String, ChannelInferenceTarget>> {
    let mut overrides = HashMap::new();
    let mut pending = tokio::task::JoinSet::new();

    for channel in channels {
        let channel_name = channel.name().to_string();
        let Some(raw_model_override) = resolve_channel_model_override(config, &channel_name) else {
            continue;
        };

        let config = Arc::clone(config);
        let auth_broker = auth_broker.clone();
        let security = Arc::clone(security);
        let default_provider = Arc::clone(default_provider);
        let default_provider_name = default_provider_name.to_string();
        let default_api_key = default_api_key.map(ToOwned::to_owned);
        let default_api_base = default_api_base.map(ToOwned::to_owned);

        pending.spawn(async move {
            let selection = if let Some((provider_override, model_override)) =
                split_provider_model_ref(&raw_model_override)
            {
                config.resolve_model(Some(provider_override), Some(model_override))
            } else {
                config.resolve_model(None, Some(&raw_model_override))
            };
            let provider_selector =
                provider_selector_with_api_base(&selection.provider, selection.api_base.as_deref());
            let default_provider_selector = provider_selector_with_api_base(
                &default_provider_name,
                default_api_base.as_deref(),
            );

            let provider_matches_default = provider_selector == default_provider_selector
                && selection.api_key.as_deref() == default_api_key.as_deref();

            let provider = if provider_matches_default {
                default_provider
            } else {
                create_resilient_provider_with_credential_provider(
                    config.as_ref(),
                    &auth_broker,
                    security.as_ref(),
                    &provider_selector,
                    &selection.provider,
                    selection.api_key.as_deref(),
                )?
            };

            if !provider_matches_default && let Err(error) = provider.warmup().await {
                tracing::warn!(
                    channel = %channel_name,
                    provider = %provider_selector,
                    error = %error,
                    "channel override provider warmup failed (non-fatal)"
                );
            }

            Ok::<_, anyhow::Error>((
                channel_name,
                provider_selector,
                selection.model.clone(),
                ChannelInferenceTarget {
                    provider,
                    model: selection.model,
                },
            ))
        });
    }

    while let Some(result) = pending.join_next().await {
        let (channel_name, provider_name, model_name, target) =
            result.map_err(|error| anyhow::anyhow!("channel override task failed: {error}"))??;

        tracing::info!(
            channel = channel_name,
            provider = %provider_name,
            model = %model_name,
            "configured channel model override"
        );

        overrides.insert(channel_name, target);
    }

    Ok(overrides)
}

fn create_media_store(config: &Config) -> Result<Option<Arc<MediaStore>>> {
    if config.media.enabled {
        let workspace_dir = config.workspace_dir.to_string_lossy().into_owned();
        let store = Arc::new(MediaStore::new(&config.media, &workspace_dir)?);
        tracing::info!(
            path = %store.storage_dir().display(),
            "media storage initialized"
        );
        Ok(Some(store))
    } else {
        Ok(None)
    }
}

fn load_channel_skill_snapshot(
    config: &Config,
    security: &Arc<SecurityPolicy>,
) -> Arc<SkillMetadataSnapshot> {
    crate::plugins::skills::load_skill_metadata_snapshot_with_policy_and_config(
        &config.workspace_dir,
        security,
        &config.skills,
    )
}

fn build_channels_with_policies(
    config: &Config,
    security: &Arc<SecurityPolicy>,
) -> (Vec<Arc<dyn Channel>>, HashMap<String, ChannelPolicy>) {
    let mut channels = Vec::new();
    let mut channel_policies = HashMap::new();

    for entry in factory::build_channels(config.channels_config.clone(), security) {
        channel_policies.insert(entry.channel.name().to_string(), entry.policy);
        channels.push(entry.channel);
    }

    (channels, channel_policies)
}

fn announce_loaded_skills(skills: &[SkillMetadata]) {
    if skills.is_empty() {
        return;
    }

    let mut skill_names = String::new();
    for skill in skills {
        if !skill_names.is_empty() {
            skill_names.push_str(", ");
        }
        skill_names.push_str(&skill.name);
    }
    println!("  › {} {}", t!("channels.skills"), skill_names);
}

pub(super) async fn init_channel_runtime(config: &Arc<Config>) -> Result<ChannelRuntime> {
    ChannelRuntimeBuilder::bootstrap(config)
        .await?
        .build()
        .await
}

#[cfg(test)]
mod tests {
    use super::{
        provider_selector_with_api_base, resolve_channel_model_override, split_provider_model_ref,
    };

    #[test]
    fn split_provider_model_ref_parses_provider_and_model() {
        let parsed = split_provider_model_ref("openai/gpt-5-mini").expect("parse provider/model");
        assert_eq!(parsed.0, "openai");
        assert_eq!(parsed.1, "gpt-5-mini");
    }

    #[test]
    fn resolve_channel_model_override_prefers_account_key() {
        let mut config = crate::config::Config::default();
        config.channels_config.discord = Some(crate::config::DiscordConfig {
            bot_token: "token".to_string(),
            application_id: None,
            guild_id: None,
            allowed_users: Vec::new(),
            intents: None,
            status: None,
            default_account: Some("ops".to_string()),
            default_to: None,
            activity_type: None,
            activity_name: None,
            thinking_embed: false,
            thinking_embed_include_preview: false,
            pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
            security: crate::config::ChannelSecurityPolicy::default(),
        });
        config.channels_config.model_by_channel.insert(
            "discord".to_string(),
            "anthropic/claude-sonnet-4.6".to_string(),
        );
        config
            .channels_config
            .model_by_channel
            .insert("discord:ops".to_string(), "openai/gpt-5-mini".to_string());

        assert_eq!(
            resolve_channel_model_override(&config, "discord"),
            Some("openai/gpt-5-mini".to_string())
        );
    }

    #[test]
    fn provider_selector_with_api_base_promotes_openai_to_custom_route() {
        assert_eq!(
            provider_selector_with_api_base("openai", Some("https://example.test/v1")),
            "custom:https://example.test/v1"
        );
    }
}
