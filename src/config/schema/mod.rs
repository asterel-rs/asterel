//! Configuration schema re-export facade.
//!
//! This module groups typed config sections used by the runtime and
//! exposes a stable import surface for consumers.

mod autonomy;
mod channels;
mod commands;
mod core;
mod gateway;
mod inference;
mod mcp;
mod memory;
mod network;
mod observability;
mod security;
mod session;
mod taste;
mod tools;
mod tunnel;

/// Shared serde default helper: returns `true`.
pub(crate) fn default_true() -> bool {
    true
}

pub use core::codespace::{CodespaceConfig, PromoteGateLevel};
pub use core::identity::{
    BrowserConfig, ComposioConfig, HeartbeatConfig, IdentityConfig, ReliabilityConfig,
    RuntimeConfig, RuntimeKind, SandboxSelectorMode, SecretsConfig,
};
pub use core::models::{ModelListEntry, SkillSource, SkillsRuntimeConfig};
pub use core::persona::{
    AffectDecayConfig, AffectEdge, AffectTopologyConfig, CharacterConfig, CharacterIdentityConfig,
    CharacterStyleDefaultsConfig, CompanionBehaviorConfig, EmotionDecayRates, LatentBiasProfile,
    PersonaConfig, RelationshipTierConfig, TraitActivationConfig,
};
pub use core::types::{Config, DEFAULT_MODEL, DEFAULT_PROVIDER};

pub use autonomy::{
    AutonomyConfig, AutonomyRolloutStage, RolloutConfig, TemperatureBand, TemperatureBands,
};
pub use channels::{
    ChannelSecurityPolicy, ChannelsConfig, DiscordConfig, DiscordPickupMode,
    DiscordPickupPolicyConfig, EmailConfig, EventTriggerConfig, GroupIsolationLevel,
    GroupIsolationMode, GroupIsolationRuleConfig, IMessageConfig, IrcConfig, MatrixConfig,
    RoutingRuleConfig, SlackConfig, TelegramConfig, TwitterConfig, WebhookConfig, WhatsAppConfig,
};
pub use commands::CommandsConfig;
pub use gateway::{GatewayConfig, GatewayDefenseMode};
pub use inference::InferenceConfig;
pub use mcp::{McpConfig, McpServerConfig, McpTransport};
pub use memory::{EmbeddingProvider, MemoryBackend, MemoryConfig};
pub use network::NetworkConfig;
pub use observability::{ObservabilityBackend, ObservabilityConfig};
pub use security::{ExternalKnowledgeTrustConfig, IntentClassifierConfig, SecurityConfig};
pub use session::{DmScope, ResetPolicy, SessionRoutingConfig};
pub use taste::{TasteBackend, TasteConfig};
pub use tools::{LoopDetectionConfig, ToolEntry, ToolsConfig};
pub use tunnel::{
    CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TailscaleTunnelConfig,
    TunnelConfig, TunnelProvider,
};

pub use crate::contracts::media::MediaConfig;
