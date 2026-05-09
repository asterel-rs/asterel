//! OpenAI provider implementation.
//!
//! Supports Chat Completions with streaming, tool use, reasoning
//! effort, and vision (image) inputs.

pub(super) mod compat;
pub(super) mod types;
use std::future::Future;
use std::pin::Pin;

use compat as openai_compat;
use reqwest::Client;
#[cfg(test)]
use types::Message;
#[cfg(test)]
use types::OpenAiToolCall;
use types::{ChatRequest, ChatResponse};

use crate::core::providers::streaming::ProviderStream;
use crate::core::providers::traits::Provider;
#[cfg(test)]
use crate::core::providers::{ContentBlock, StopReason};
#[cfg(test)]
use crate::core::providers::{ImageSource, MessageRole, sse::parse_data_lines_no_done};
use crate::core::providers::{
    InferenceOpts, ProviderMessage, ProviderResponse, ProviderResult, build_provider_http_client,
};
use crate::core::tools::traits::ToolSpec;

/// `OpenAI` Chat Completions provider with streaming, tool use,
/// reasoning effort, and vision support.
pub struct OpenAiProvider {
    /// Pre-computed `"Bearer <key>"` header value (avoids `format!` per request).
    cached_auth_header: Option<String>,
    client: Client,
}

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const OPENAI_MISSING_API_KEY_MESSAGE: &str =
    "OpenAI API key not set. Set OPENAI_API_KEY or edit config.toml.";

impl OpenAiProvider {
    /// Create a new `OpenAI` provider with an optional API key.
    #[must_use]
    pub fn new(api_key: Option<&str>) -> Self {
        Self {
            cached_auth_header: api_key.and_then(|key| {
                let trimmed = key.trim();
                (!trimmed.is_empty()).then(|| format!("Bearer {trimmed}"))
            }),
            client: build_provider_http_client(),
        }
    }

    fn build_request(
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> ChatRequest {
        openai_compat::build_request(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        )
    }

    #[cfg(test)]
    fn map_provider_message(provider_message: &ProviderMessage) -> Vec<Message> {
        openai_compat::map_provider_message(provider_message)
    }

    fn build_tools_request(
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> ChatRequest {
        openai_compat::build_tools_request(
            system_prompt,
            messages,
            tools,
            model,
            temperature,
            inference_options,
        )
    }

    fn extract_text(chat_response: &ChatResponse) -> anyhow::Result<String> {
        openai_compat::extract_text(chat_response, "OpenAI")
    }

    #[cfg(test)]
    fn map_finish_reason(finish_reason: Option<&str>) -> StopReason {
        openai_compat::map_finish_reason(finish_reason)
    }

    #[cfg(test)]
    fn parse_tool_calls(
        tool_calls: Option<Vec<OpenAiToolCall>>,
    ) -> anyhow::Result<Vec<ContentBlock>> {
        openai_compat::parse_tool_calls(tool_calls, "OpenAI")
    }

    async fn call_api(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ChatResponse> {
        let request = Self::build_request(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        );

        self.call_api_req(&request).await
    }

    async fn call_api_req(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        openai_compat::send_chat_completions_json(
            &self.client,
            self.cached_auth_header.as_ref(),
            request,
            openai_compat::ChatCompletionsEndpoint {
                provider_name: "OpenAI",
                url: OPENAI_CHAT_COMPLETIONS_URL,
                missing_api_key_message: OPENAI_MISSING_API_KEY_MESSAGE,
                extra_headers: &[],
            },
        )
        .await
    }

    async fn call_api_streaming(&self, request: &ChatRequest) -> anyhow::Result<reqwest::Response> {
        openai_compat::send_chat_completions_raw(
            &self.client,
            self.cached_auth_header.as_ref(),
            request,
            openai_compat::ChatCompletionsEndpoint {
                provider_name: "OpenAI",
                url: OPENAI_CHAT_COMPLETIONS_URL,
                missing_api_key_message: OPENAI_MISSING_API_KEY_MESSAGE,
                extra_headers: &[],
            },
        )
        .await
    }

    async fn chat_with_tools_stream_inner(
        &self,
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ProviderStream> {
        let request = openai_compat::build_stream_request(
            system_prompt,
            messages,
            tools,
            model,
            temperature,
            inference_options,
        );
        let response = self.call_api_streaming(&request).await?;
        Ok(openai_compat::sse_response_to_provider_stream(response))
    }

    fn model_supports_native_tools(model: &str) -> bool {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return true;
        }
        normalized.starts_with("gpt-5")
            || normalized.starts_with("gpt-4")
            || normalized.starts_with("o1")
            || normalized.starts_with("o3")
            || normalized.starts_with("o4")
    }

    fn model_supports_vision(model: &str) -> bool {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return true;
        }
        normalized.starts_with("gpt-5")
            || normalized.starts_with("gpt-4o")
            || normalized.starts_with("gpt-4.1")
            || normalized.contains("vision")
            || normalized.contains("multimodal")
            || normalized.contains("omni")
    }
}

impl Provider for OpenAiProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: Self::model_supports_native_tools(model),
            streaming: true,
            vision: Self::model_supports_vision(model),
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
            let chat_response = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            Self::extract_text(&chat_response).map_err(Into::into)
        })
    }

    fn chat_with_system_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let chat_response = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            Self::extract_text(&chat_response).map_err(Into::into)
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
            let chat_response = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;

            openai_compat::build_text_provider_response(chat_response, "OpenAI").map_err(Into::into)
        })
    }

    fn chat_with_system_full_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let chat_response = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;

            openai_compat::build_text_provider_response(chat_response, "OpenAI").map_err(Into::into)
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
            let request =
                Self::build_tools_request(system_prompt, messages, tools, model, temperature, None);
            let chat_response = self.call_api_req(&request).await?;
            openai_compat::build_tool_provider_response(chat_response, "OpenAI").map_err(Into::into)
        })
    }

    fn chat_with_tools_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let request = Self::build_tools_request(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            );
            let chat_response = self.call_api_req(&request).await?;
            openai_compat::build_tool_provider_response(chat_response, "OpenAI").map_err(Into::into)
        })
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
            self.chat_with_tools_stream_inner(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                None,
            )
            .await
            .map_err(Into::into)
        })
    }

    fn chat_with_tools_stream_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_tools_stream_inner(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            )
            .await
            .map_err(Into::into)
        })
    }
}

#[cfg(test)]
mod tests;
