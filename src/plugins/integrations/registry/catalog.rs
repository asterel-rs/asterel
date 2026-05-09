//! Static catalog of all supported integrations (chat, AI model,
//! productivity, music, smart home, tools, media, social, platform).

use super::status;
use crate::config::Config;
use crate::core::providers::catalog::ai_integration_provider_entries;
use crate::plugins::integrations::{IntegrationCategory, IntegrationEntry, IntegrationStatus};

fn integration(
    name: &'static str,
    description: &'static str,
    category: IntegrationCategory,
    status_fn: fn(&Config) -> IntegrationStatus,
) -> IntegrationEntry {
    IntegrationEntry {
        name,
        description,
        category,
        status_fn,
    }
}

fn chat_integrations() -> Vec<IntegrationEntry> {
    vec![
        integration(
            "Telegram",
            "Bot API — long-polling",
            IntegrationCategory::Chat,
            status::telegram,
        ),
        integration(
            "Discord",
            "Servers, channels & DMs",
            IntegrationCategory::Chat,
            status::discord,
        ),
        integration(
            "Slack",
            "Workspace apps via Web API",
            IntegrationCategory::Chat,
            status::slack,
        ),
        integration(
            "Webhooks",
            "HTTP endpoint for triggers",
            IntegrationCategory::Chat,
            status::webhooks,
        ),
        integration(
            "iMessage",
            "macOS AppleScript bridge",
            IntegrationCategory::Chat,
            status::imessage,
        ),
        integration(
            "Matrix",
            "Matrix protocol (Element)",
            IntegrationCategory::Chat,
            status::matrix,
        ),
    ]
}

fn ai_status_fn(provider_id: &str) -> fn(&Config) -> IntegrationStatus {
    match provider_id {
        "openrouter" => status::openrouter,
        "anthropic" => status::anthropic,
        "openai" => status::openai,
        "gemini" => status::google,
        "gemini-vertex" => status::google_vertex,
        "deepseek" => status::deepseek,
        "xai" => status::xai,
        "mistral" => status::mistral_model,
        "ollama" => status::ollama,
        "perplexity" => status::perplexity,
        "venice" => status::venice,
        "vercel" => status::vercel,
        "cloudflare" => status::cloudflare,
        "moonshot" => status::moonshot,
        "synthetic" => status::synthetic,
        "opencode" => status::opencode,
        "zai" => status::zai,
        "glm" => status::glm,
        "minimax" => status::minimax,
        "qianfan" => status::qianfan,
        "groq" => status::groq,
        "together" => status::together,
        "fireworks" => status::fireworks,
        "cohere" => status::cohere,
        other => panic!("missing integration status mapping for provider '{other}'"),
    }
}

fn ai_model_integrations() -> Vec<IntegrationEntry> {
    ai_integration_provider_entries()
        .into_iter()
        .map(|provider| {
            let integration_spec = provider.integration.as_ref().unwrap_or_else(|| {
                panic!("provider '{}' missing integration metadata", provider.id)
            });
            integration(
                integration_spec.name.as_str(),
                integration_spec.description.as_str(),
                IntegrationCategory::AiModel,
                ai_status_fn(provider.id.as_str()),
            )
        })
        .collect()
}

fn productivity_integrations() -> Vec<IntegrationEntry> {
    vec![]
}

fn smart_home_integrations() -> Vec<IntegrationEntry> {
    vec![]
}

fn tools_automation_integrations() -> Vec<IntegrationEntry> {
    vec![
        integration(
            "Browser",
            "Chrome/Chromium control",
            IntegrationCategory::ToolsAutomation,
            status::available,
        ),
        integration(
            "Shell",
            "Terminal command execution",
            IntegrationCategory::ToolsAutomation,
            status::active,
        ),
        integration(
            "File System",
            "Read/write files",
            IntegrationCategory::ToolsAutomation,
            status::active,
        ),
        integration(
            "Cron",
            "Scheduled tasks",
            IntegrationCategory::ToolsAutomation,
            status::available,
        ),
    ]
}

fn media_creative_integrations() -> Vec<IntegrationEntry> {
    vec![]
}

fn social_integrations() -> Vec<IntegrationEntry> {
    vec![integration(
        "X (Twitter)",
        "Posts, mentions & DMs via API v2",
        IntegrationCategory::Social,
        status::twitter_channel,
    )]
}

fn platform_integrations() -> Vec<IntegrationEntry> {
    vec![
        integration(
            "macOS",
            "Native support + AppleScript",
            IntegrationCategory::Platform,
            status::macos,
        ),
        integration(
            "Linux",
            "Native support",
            IntegrationCategory::Platform,
            status::linux,
        ),
        integration(
            "Windows",
            "WSL2 recommended",
            IntegrationCategory::Platform,
            status::available,
        ),
        integration(
            "iOS",
            "Chat via Telegram/Discord",
            IntegrationCategory::Platform,
            status::available,
        ),
        integration(
            "Android",
            "Chat via Telegram/Discord",
            IntegrationCategory::Platform,
            status::available,
        ),
    ]
}

/// Returns the full catalog of integrations.
pub(super) fn all_integrations() -> Vec<IntegrationEntry> {
    let mut integrations = Vec::new();
    integrations.extend(chat_integrations());
    integrations.extend(ai_model_integrations());
    integrations.extend(productivity_integrations());
    integrations.extend(smart_home_integrations());
    integrations.extend(tools_automation_integrations());
    integrations.extend(media_creative_integrations());
    integrations.extend(social_integrations());
    integrations.extend(platform_integrations());
    integrations
}
