//! Chat completions call path for the OpenAI-compatible provider.

use anyhow::Context;

use super::types::{ChatRequest, ChatResponse, Message, extract_chat_text};
use super::{ChatCompletionsOutcome, OpenAiCompatProvider};
use crate::core::providers::{ProviderResponse, sanitize_api_error};

impl OpenAiCompatProvider {
    /// Send a chat completions request and return the parsed response.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure or JSON decode failure.
    pub(super) async fn call_chat_completions(
        &self,
        request: &ChatRequest,
    ) -> anyhow::Result<ChatCompletionsOutcome> {
        let url = self.chat_completions_url();

        let response = self
            .apply_auth_header(self.client.post(url).json(request))
            .send()
            .await
            .with_context(|| format!("{} chat completions request failed", self.name))?;

        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await?;
            let sanitized_error = sanitize_api_error(&error);

            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(ChatCompletionsOutcome::NotFound(sanitized_error));
            }

            anyhow::bail!("{} API error: {sanitized_error}", self.name);
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .with_context(|| format!("{} chat completions JSON decode failed", self.name))?;
        Ok(ChatCompletionsOutcome::Ok(chat_response))
    }

    /// Build and execute a system+user chat request, returning the
    /// provider response with text and usage metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the API key is unset, the HTTP request
    /// fails, or the response cannot be parsed.
    pub(super) async fn chat_with_system_internal(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderResponse> {
        tracing::debug!(
            provider = %self.name,
            base_url = %self.base_url,
            auth_style = self.auth_style_label(),
            prefer_responses_api = self.prefer_responses_api,
            "issuing compatible provider request"
        );

        if self.api_key.is_none() {
            anyhow::bail!(
                "{} API key not set. Run `asterel onboard` or set the appropriate env var.",
                self.name
            );
        }

        let capacity = if system_prompt.is_some() { 2 } else { 1 };
        let mut messages = Vec::with_capacity(capacity);

        if let Some(sys) = system_prompt {
            messages.push(Message {
                role: "system",
                content: sys.to_string(),
            });
        }

        messages.push(Message {
            role: "user",
            content: message.to_string(),
        });

        let request = ChatRequest {
            model: model.to_string(),
            messages,
            temperature,
        };

        if self.prefer_responses_api {
            return self.chat_via_responses(system_prompt, message, model).await;
        }

        match self.call_chat_completions(&request).await? {
            ChatCompletionsOutcome::Ok(chat_response) => {
                let text = extract_chat_text(&chat_response, &self.name)?;
                let mut provider_response = if let Some(usage) = chat_response.usage {
                    ProviderResponse::with_usage(text, usage.prompt_tokens, usage.completion_tokens)
                } else {
                    ProviderResponse::text_only(text)
                };

                if let Some(api_model) = chat_response.model {
                    provider_response = provider_response.with_model(api_model);
                }

                Ok(provider_response)
            }
            ChatCompletionsOutcome::NotFound(sanitized_error) => {
                self.chat_via_responses(system_prompt, message, model)
                    .await
                    .map_err(|responses_err| {
                        anyhow::anyhow!(
                            "{} API error: {sanitized_error} (chat completions unavailable; responses fallback failed: {responses_err})",
                            self.name
                        )
                    })
            }
        }
    }
}
