//! Generic `OpenAI`-compatible provider.
//!
//! Most hosted LLM APIs expose the same `/v1/chat/completions` JSON format.
//! This module provides a single `OpenAiCompatProvider` implementation that
//! handles many of them — Venice, Vercel AI Gateway, Cloudflare AI Gateway,
//! Moonshot, Groq, Mistral, `xAI`, DeepSeek, Together AI, Fireworks AI,
//! Perplexity, Cohere, GitHub Copilot, registry-projected Bedrock-compatible
//! endpoints, and others.
//!
//! Providers that target the newer `OpenAI` Responses API endpoint are
//! detected by the `prefer_responses_api` flag (set when the base URL ends
//! with `/responses`) and routed through `compatible/responses.rs` instead.

mod chat;
mod responses;
#[cfg(test)]
mod tests;
mod types;
use std::future::Future;
use std::pin::Pin;

use reqwest::Client;
use types::ChatResponse;

use crate::core::providers::fallback_tools::{augment_prompt_with_tools, build_fallback_response};
use crate::core::providers::streaming::{ProviderStream, resp_to_events};
use crate::core::providers::traits::{Provider, messages_to_text};
use crate::core::providers::{
    ProviderMessage, ProviderResponse, ProviderResult, build_provider_http_client,
};
use crate::core::tools::traits::ToolSpec;

/// A provider that speaks the OpenAI-compatible chat completions API.
/// Used by registry entries such as Venice, Vercel AI Gateway, Cloudflare AI
/// Gateway, Moonshot, Synthetic, `OpenCode` Zen, `Z.AI`, `GLM`, Bedrock-like
/// compatible endpoints, Qianfan, Groq, Mistral, `xAI`, etc. Canonical
/// `minimax` is routed through its dedicated Anthropic-compatible provider.
pub struct OpenAiCompatProvider {
    /// Display name shown in logs and error messages.
    pub(crate) name: String,
    /// Base URL for the provider API (trailing slash stripped).
    pub(crate) base_url: String,
    /// Resolved API key, if any.
    pub(crate) api_key: Option<String>,
    /// Authentication header style for this provider.
    pub(crate) auth_header: AuthStyle,
    /// Pre-computed `(header_name, header_value)` for auth (avoids `format!` per request).
    cached_auth: Option<(String, String)>,
    client: Client,
    /// Pre-computed full URL for `/chat/completions`.
    cached_chat_url: String,
    /// Pre-computed full URL for `/responses`.
    cached_responses_url: String,
    /// If true, native Responses API tool calling and streaming
    /// are used instead of Chat Completions.
    pub(crate) prefer_responses_api: bool,
    pub(crate) native_tools: bool,
}

/// How the API key is passed in HTTP requests.
#[derive(Debug, Clone)]
pub enum AuthStyle {
    /// Standard `Authorization: Bearer <key>` header.
    Bearer,
    /// `X-Api-Key: <key>` header (Anthropic-style).
    XApiKey,
    /// Custom header name with the key as the value.
    Custom(String),
}

impl OpenAiCompatProvider {
    /// Create a new compatible provider with the given display name,
    /// base URL, optional API key, and authentication style.
    pub fn new(
        name: &str,
        base_url: &str,
        api_key: Option<&str>,
        auth_style: AuthStyle,
        native_tools: bool,
    ) -> Self {
        let base = base_url.trim_end_matches('/');
        let cached_auth = api_key.map(|key| match &auth_style {
            AuthStyle::Bearer => ("Authorization".to_string(), format!("Bearer {key}")),
            AuthStyle::XApiKey => ("X-Api-Key".to_string(), key.to_string()),
            AuthStyle::Custom(header) => (header.clone(), key.to_string()),
        });

        let is_responses_endpoint = base.ends_with("/responses");

        let cached_chat_url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        };

        let cached_responses_url = if is_responses_endpoint {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{base}/responses")
        } else {
            format!("{base}/v1/responses")
        };

        Self {
            name: name.to_string(),
            base_url: base.to_string(),
            api_key: api_key.map(str::to_string),
            auth_header: auth_style,
            cached_auth,
            client: build_provider_http_client(),
            cached_chat_url,
            cached_responses_url,
            prefer_responses_api: is_responses_endpoint,
            native_tools,
        }
    }

    fn chat_completions_url(&self) -> &str {
        &self.cached_chat_url
    }

    fn responses_url(&self) -> &str {
        &self.cached_responses_url
    }

    pub(crate) fn auth_style_label(&self) -> &'static str {
        match self.auth_header {
            AuthStyle::Bearer => "bearer",
            AuthStyle::XApiKey => "x-api-key",
            AuthStyle::Custom(_) => "custom",
        }
    }
}

/// Result of a chat completions attempt, distinguishing a successful response
/// from a 404 (endpoint not found), which triggers a Responses API fallback.
enum ChatCompletionsOutcome {
    Ok(ChatResponse),
    NotFound(String),
}

impl OpenAiCompatProvider {
    /// Attach the pre-computed auth header to `req`, or return `req` unchanged
    /// when no API key was configured.
    fn apply_auth_header(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some((name, value)) = &self.cached_auth {
            req.header(name, value)
        } else {
            req
        }
    }

    /// Build the system prompt and message text for XML fallback tool calling.
    /// Returns `(augmented_system_prompt, concatenated_message_text)`.
    fn prepare_fallback_input(
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
    ) -> (String, String) {
        let augmented_prompt = augment_prompt_with_tools(system_prompt.unwrap_or(""), tools);
        let text = messages_to_text(messages);
        (augmented_prompt, text)
    }
}

impl Provider for OpenAiCompatProvider {
    fn capabilities(&self, _model: &str) -> crate::contracts::provider::ProviderCapabilities {
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: self.native_tools,
            streaming: true,
            vision: !self.prefer_responses_api,
        }
    }

    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_system_internal(system_prompt, message, model, temperature)
                .await
                .map(|response| response.text)
                .map_err(Into::into)
        })
    }

    fn chat_with_system_full<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_system_internal(system_prompt, message, model, temperature)
                .await
                .map_err(Into::into)
        })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            if self.prefer_responses_api {
                return self
                    .chat_via_responses_with_tools(system_prompt, messages, tools, model)
                    .await
                    .map_err(Into::into);
            }
            let (augmented_prompt, text) =
                Self::prepare_fallback_input(system_prompt, messages, tools);
            let response = self
                .chat_with_system_full(Some(&augmented_prompt), &text, model, temperature)
                .await?;
            Ok(build_fallback_response(response, tools))
        })
    }

    fn supports_tools(&self) -> bool {
        self.native_tools
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            if self.prefer_responses_api {
                return self
                    .chat_via_responses_stream(system_prompt, messages, tools, model)
                    .await
                    .map_err(Into::into);
            }
            // Non-responses-api: fall back to non-streaming
            let resp = self
                .chat_with_tools(system_prompt, messages, tools, model, temperature)
                .await?;
            Ok(Box::pin(futures_util::stream::iter(resp_to_events(resp))) as ProviderStream)
        })
    }
}
