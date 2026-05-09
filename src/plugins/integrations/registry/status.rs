//! Per-integration status functions that check config to determine
//! whether each integration is active or available.

use super::super::IntegrationStatus;
use crate::config::Config;
use crate::core::providers::catalog::canonical_provider_name;

fn active_when(condition: bool) -> IntegrationStatus {
    if condition {
        IntegrationStatus::Active
    } else {
        IntegrationStatus::Available
    }
}

fn provider_status(config: &Config, provider: &str) -> IntegrationStatus {
    let resolved = config.resolve_model(None, None);
    active_when(canonical_provider_name(&resolved.provider) == provider)
}

fn model_prefix_status(config: &Config, prefix: &str) -> IntegrationStatus {
    let resolved = config.resolve_model(None, None);
    let resolved_model = resolved.model.as_str();
    if resolved_model.starts_with(prefix) {
        return IntegrationStatus::Active;
    }

    let resolved_qualified = format!("{}/{}", resolved.provider, resolved.model);
    active_when(resolved_qualified.starts_with(prefix))
}

macro_rules! channel_status {
    ($name:ident, $field:ident) => {
        pub(super) fn $name(config: &Config) -> IntegrationStatus {
            active_when(config.channels_config.$field.is_some())
        }
    };
}

macro_rules! provider_status_fn {
    ($name:ident, $provider:literal) => {
        pub(super) fn $name(config: &Config) -> IntegrationStatus {
            provider_status(config, $provider)
        }
    };
}

macro_rules! model_prefix_status_fn {
    ($name:ident, $prefix:literal) => {
        pub(super) fn $name(config: &Config) -> IntegrationStatus {
            model_prefix_status(config, $prefix)
        }
    };
}

channel_status!(telegram, telegram);
channel_status!(discord, discord);
channel_status!(slack, slack);
channel_status!(webhooks, webhook);
channel_status!(imessage, imessage);
channel_status!(matrix, matrix);
channel_status!(twitter_channel, twitter);

/// Check whether `OpenRouter` is the active provider.
pub(super) fn openrouter(config: &Config) -> IntegrationStatus {
    active_when(
        config.default_provider.as_deref() == Some("openrouter") && config.api_key.is_some(),
    )
}

provider_status_fn!(anthropic, "anthropic");
provider_status_fn!(openai, "openai");
/// Check whether Google/Gemini is the active provider.
pub(super) fn google(config: &Config) -> IntegrationStatus {
    provider_status(config, "gemini")
}

pub(super) fn google_vertex(config: &Config) -> IntegrationStatus {
    provider_status(config, "gemini-vertex")
}
model_prefix_status_fn!(deepseek, "deepseek/");
/// Check whether xAI/Grok is the active provider.
pub(super) fn xai(config: &Config) -> IntegrationStatus {
    provider_status(config, "xai")
}
/// Check whether Mistral is the active provider or model prefix.
pub(super) fn mistral_model(config: &Config) -> IntegrationStatus {
    let resolved = config.resolve_model(None, None);
    active_when(
        canonical_provider_name(&resolved.provider) == "mistral"
            || resolved.model.starts_with("mistral"),
    )
}
provider_status_fn!(ollama, "ollama");
provider_status_fn!(perplexity, "perplexity");
provider_status_fn!(venice, "venice");
provider_status_fn!(vercel, "vercel");
provider_status_fn!(cloudflare, "cloudflare");
provider_status_fn!(moonshot, "moonshot");
provider_status_fn!(synthetic, "synthetic");
provider_status_fn!(opencode, "opencode");
provider_status_fn!(zai, "zai");
provider_status_fn!(glm, "glm");
provider_status_fn!(minimax, "minimax");
provider_status_fn!(qianfan, "qianfan");
provider_status_fn!(groq, "groq");
provider_status_fn!(together, "together");
provider_status_fn!(fireworks, "fireworks");
provider_status_fn!(cohere, "cohere");

/// Active on macOS, available elsewhere.
pub(super) fn macos(_: &Config) -> IntegrationStatus {
    if cfg!(target_os = "macos") {
        IntegrationStatus::Active
    } else {
        IntegrationStatus::Available
    }
}

/// Active on Linux, available elsewhere.
pub(super) fn linux(_: &Config) -> IntegrationStatus {
    if cfg!(target_os = "linux") {
        IntegrationStatus::Active
    } else {
        IntegrationStatus::Available
    }
}

/// Always returns `Available`.
pub(super) fn available(_: &Config) -> IntegrationStatus {
    IntegrationStatus::Available
}

/// Always returns `Active`.
pub(super) fn active(_: &Config) -> IntegrationStatus {
    IntegrationStatus::Active
}
