//! Google Gemini provider with support for:
//! - Direct API key (`GEMINI_API_KEY` env var or config)
//! - Gemini CLI OAuth tokens (reuse existing ~/.gemini/ authentication)
//! - `GOOGLE_API_KEY` compatibility env var

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use directories::UserDirs;
use num_traits::ToPrimitive;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::providers::sse::{SseBuffer, parse_data_lines};
use crate::core::providers::streaming::{ProviderStream, StreamEvent};
use crate::core::providers::tool_convert::{ToolFields, map_tools_optional};
use crate::core::providers::traits::Provider;
use crate::core::providers::{
    ContentBlock, ImageSource, InferenceOpts, MessageRole, ProviderMessage, ProviderResponse,
    ProviderResult, StopReason, build_provider_http_client, scrub_secrets,
};
use crate::security::scrub::sanitize_api_error;

mod types;
use types::{
    Candidate, Content, GeminiFileData, GeminiFunctionCall, GeminiFunctionDeclaration,
    GeminiFunctionResponse, GeminiInlineData, GeminiTool, GenerateContentRequest,
    GenerateContentResponse, GenerationConfig, Part, ResponsePart, ThinkingConfig,
};

/// Gemini provider supporting multiple authentication methods.
pub struct GeminiProvider {
    auth: Option<GeminiResolvedAuth>,
    client: Client,
    surface: GeminiApiSurface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GeminiResolvedAuth {
    ApiKey(String),
    OAuthBearer(String),
    ApplicationDefaultCredentials,
}

impl GeminiResolvedAuth {
    async fn apply(
        &self,
        request: reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::RequestBuilder> {
        match self {
            Self::ApiKey(value) => Ok(request.header("x-goog-api-key", value)),
            Self::OAuthBearer(value) => {
                Ok(request.header("Authorization", format!("Bearer {value}")))
            }
            Self::ApplicationDefaultCredentials => {
                let token = fetch_vertex_adc_access_token().await?;
                Ok(request.header("Authorization", format!("Bearer {token}")))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeminiApiSurface {
    DeveloperApi,
    VertexAi {
        project: String,
        location: String,
        base_url: String,
    },
}

// ══════════════════════════════════════════════════════════════════════════════
// GEMINI CLI TOKEN STRUCTURES
// ══════════════════════════════════════════════════════════════════════════════

/// OAuth token stored by the `Gemini` CLI at `~/.gemini/oauth_creds.json`.
/// Loaded by `try_load_gemini_cli_token` to reuse existing CLI authentication.
#[derive(Debug, Deserialize)]
struct GeminiCliOAuthCreds {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expiry: Option<String>,
}

async fn fetch_vertex_adc_access_token() -> anyhow::Result<String> {
    let provider = gcp_auth::provider()
        .await
        .map_err(|error| anyhow::anyhow!("failed to initialize Google ADC auth: {error}"))?;
    let token = provider
        .token(&["https://www.googleapis.com/auth/cloud-platform"])
        .await
        .map_err(|error| anyhow::anyhow!("failed to obtain Google ADC access token: {error}"))?;
    Ok(token.as_str().to_string())
}

impl GeminiProvider {
    /// Create a new Gemini provider.
    ///
    /// Authentication priority:
    /// 1. Explicit API key passed in
    /// 2. `GEMINI_API_KEY` environment variable
    /// 3. `GOOGLE_API_KEY` environment variable
    /// 4. Gemini CLI OAuth tokens (`~/.gemini/oauth_creds.json`)
    #[must_use]
    pub fn new(api_key: Option<&str>) -> Self {
        let resolved_auth = Self::resolve_auth(api_key);

        Self {
            auth: resolved_auth,
            client: build_provider_http_client(),
            surface: GeminiApiSurface::DeveloperApi,
        }
    }

    #[must_use]
    pub fn new_vertex(
        project: impl Into<String>,
        location: impl Into<String>,
        api_key: Option<&str>,
    ) -> Self {
        Self {
            auth: Self::resolve_vertex_auth(api_key),
            client: build_provider_http_client(),
            surface: GeminiApiSurface::VertexAi {
                project: project.into(),
                location: location.into(),
                base_url: "https://aiplatform.googleapis.com/v1".to_string(),
            },
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn new_vertex_with_base_url(
        project: impl Into<String>,
        location: impl Into<String>,
        api_key: Option<&str>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            auth: Self::resolve_vertex_auth(api_key),
            client: build_provider_http_client(),
            surface: GeminiApiSurface::VertexAi {
                project: project.into(),
                location: location.into(),
                base_url: base_url.into(),
            },
        }
    }

    pub(crate) fn resolve_auth(api_key: Option<&str>) -> Option<GeminiResolvedAuth> {
        api_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| GeminiResolvedAuth::ApiKey(value.to_string()))
            .or_else(|| {
                std::env::var("GEMINI_API_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(GeminiResolvedAuth::ApiKey)
            })
            .or_else(|| {
                std::env::var("GOOGLE_API_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(GeminiResolvedAuth::ApiKey)
            })
            .or_else(|| Self::try_load_gemini_cli_token().map(GeminiResolvedAuth::OAuthBearer))
    }

    pub(crate) fn resolve_vertex_auth(api_key: Option<&str>) -> Option<GeminiResolvedAuth> {
        api_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| GeminiResolvedAuth::ApiKey(value.to_string()))
            .or_else(|| {
                std::env::var("GOOGLE_API_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(GeminiResolvedAuth::ApiKey)
            })
            .or_else(|| {
                std::env::var("GEMINI_API_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(GeminiResolvedAuth::ApiKey)
            })
            .or_else(|| {
                Self::has_adc_credentials()
                    .then_some(GeminiResolvedAuth::ApplicationDefaultCredentials)
            })
    }

    /// Try to load OAuth access token from Gemini CLI's cached credentials.
    /// Location: `~/.gemini/oauth_creds.json`
    fn try_load_gemini_cli_token() -> Option<String> {
        let gemini_dir = Self::gemini_cli_dir()?;
        let creds_path = gemini_dir.join("oauth_creds.json");

        if !creds_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&creds_path).ok()?;
        let creds: GeminiCliOAuthCreds = serde_json::from_str(&content).ok()?;

        // Check if token is expired (basic check)
        if let Some(ref expiry) = creds.expiry
            && let Ok(expiry_time) = chrono::DateTime::parse_from_rfc3339(expiry)
            && expiry_time < chrono::Utc::now()
        {
            tracing::debug!("Gemini CLI OAuth token expired, skipping");
            return None;
        }

        if creds.access_token.is_none() && creds.refresh_token.is_some() {
            tracing::debug!("Gemini CLI creds present refresh_token but no access_token");
        }

        creds.access_token
    }

    /// Return the `Gemini` CLI config directory (`~/.gemini`).
    fn gemini_cli_dir() -> Option<PathBuf> {
        UserDirs::new().map(|u| u.home_dir().join(".gemini"))
    }

    fn gcloud_adc_path() -> Option<PathBuf> {
        UserDirs::new().map(|u| {
            u.home_dir()
                .join(".config")
                .join("gcloud")
                .join("application_default_credentials.json")
        })
    }

    /// Return `true` if the `Gemini` CLI has a valid (non-expired) OAuth token.
    #[must_use]
    pub fn has_cli_credentials() -> bool {
        Self::try_load_gemini_cli_token().is_some()
    }

    #[must_use]
    pub fn has_adc_credentials() -> bool {
        std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .is_some_and(|value| std::path::Path::new(&value).exists())
            || Self::gcloud_adc_path().is_some_and(|path| path.exists())
    }

    /// Return `true` if any `Gemini` authentication source is available
    /// (`GEMINI_API_KEY`, `GOOGLE_API_KEY`, or CLI OAuth credentials).
    #[must_use]
    pub fn has_any_auth() -> bool {
        std::env::var("GEMINI_API_KEY").is_ok()
            || std::env::var("GOOGLE_API_KEY").is_ok()
            || Self::has_cli_credentials()
    }

    /// Return a human-readable label for the active authentication source,
    /// used in diagnostic and onboarding messages.
    #[must_use]
    pub fn auth_source(&self) -> &'static str {
        if self.auth.is_none() {
            return "none";
        }
        if self
            .auth
            .as_ref()
            .is_some_and(|auth| matches!(auth, GeminiResolvedAuth::ApplicationDefaultCredentials))
        {
            return "Application Default Credentials";
        }
        if std::env::var("GEMINI_API_KEY").is_ok() {
            return "GEMINI_API_KEY env var";
        }
        if std::env::var("GOOGLE_API_KEY").is_ok() {
            return "GOOGLE_API_KEY env var";
        }
        if self
            .auth
            .as_ref()
            .is_some_and(|auth| matches!(auth, GeminiResolvedAuth::OAuthBearer(_)))
        {
            return "Gemini CLI OAuth";
        }
        "config"
    }

    fn build_generation_config(
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> GenerationConfig {
        const BASE_MAX_OUTPUT_TOKENS: u32 = 8192;
        let max_output_tokens = inference_options
            .and_then(|opts| opts.max_tokens_factor)
            .map_or(BASE_MAX_OUTPUT_TOKENS, |factor| {
                scaled_max_tokens(BASE_MAX_OUTPUT_TOKENS, factor)
            });
        GenerationConfig {
            temperature,
            max_output_tokens,
            top_p: inference_options.and_then(|opts| opts.top_p),
            thinking_config: Self::map_thinking_config(inference_options),
        }
    }

    fn build_request(
        system_prompt: Option<&str>,
        message: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> GenerateContentRequest {
        let system_instruction = system_prompt.map(|sys| Content {
            role: None,
            parts: vec![Part::text(scrub_secrets(sys).into_owned())],
        });

        GenerateContentRequest {
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part::text(scrub_secrets(message).into_owned())],
            }],
            system_instruction,
            tools: None,
            generation_config: Self::build_generation_config(temperature, inference_options),
        }
    }

    fn map_thinking_config(inference_options: Option<&InferenceOpts>) -> Option<ThinkingConfig> {
        let thinking_budget = inference_options
            .and_then(|options| super::inference::gemini_thinking_budget(options.thinking_level))?;
        Some(ThinkingConfig {
            thinking_budget,
            include_thoughts: false,
        })
    }

    fn model_name(model: &str) -> String {
        if model.starts_with("models/") {
            model.to_string()
        } else {
            format!("models/{model}")
        }
    }

    fn vertex_model_name(model: &str) -> &str {
        model.trim_start_matches("models/")
    }

    fn request_url(&self, model: &str, streaming: bool) -> String {
        match &self.surface {
            GeminiApiSurface::DeveloperApi => {
                let model_name = Self::model_name(model);
                if streaming {
                    format!(
                        "https://generativelanguage.googleapis.com/v1beta/{model_name}:streamGenerateContent?alt=sse"
                    )
                } else {
                    format!(
                        "https://generativelanguage.googleapis.com/v1beta/{model_name}:generateContent"
                    )
                }
            }
            GeminiApiSurface::VertexAi {
                project,
                location,
                base_url,
            } => {
                let base = base_url.trim_end_matches('/');
                let model_name = Self::vertex_model_name(model);
                let method = if streaming {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                };
                format!(
                    "{base}/projects/{project}/locations/{location}/publishers/google/models/{model_name}:{method}"
                )
            }
        }
    }

    fn auth(&self) -> anyhow::Result<&GeminiResolvedAuth> {
        self.auth.as_ref().ok_or_else(|| {
            let message = if matches!(self.surface, GeminiApiSurface::VertexAi { .. }) {
                "Vertex AI credentials not found. Options:\n\
                          1. Set GOOGLE_API_KEY or GEMINI_API_KEY env var\n\
                          2. Set GOOGLE_APPLICATION_CREDENTIALS or run `gcloud auth application-default login`\n\
                          3. Configure project/location via `gemini-vertex:<project>/<location>`\n\
                          4. Run `asterel onboard` to configure"
            } else {
                "API key not found. Options:\n\
                          1. Set GEMINI_API_KEY env var\n\
                          2. Run `gemini` CLI to authenticate (tokens will be reused)\n\
                          3. Get an API key from https://aistudio.google.com/app/apikey\n\
                          4. Run `asterel onboard` to configure"
            };
            super::ProviderError::MissingCredentials {
                provider: "Gemini".into(),
                message: message.into(),
            }
            .into()
        })
    }

    /// POST `request` to `url` with the resolved Gemini auth header and return the
    /// raw response after checking the HTTP status code.
    async fn send_api_request(
        &self,
        url: &str,
        auth: &GeminiResolvedAuth,
        request: &GenerateContentRequest,
    ) -> anyhow::Result<reqwest::Response> {
        let request_builder = auth.apply(self.client.post(url)).await?;
        let response = request_builder.json(request).send().await?;
        Self::ensure_success_status(response).await
    }

    /// Return the response unchanged on 2xx, or bail with the body text on
    /// non-2xx status codes.
    async fn ensure_success_status(
        response: reqwest::Response,
    ) -> anyhow::Result<reqwest::Response> {
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            let sanitized = sanitize_api_error(&error_text);
            anyhow::bail!("Gemini API error ({status}): {sanitized}");
        }

        Ok(response)
    }

    fn extract_text(result: &GenerateContentResponse) -> anyhow::Result<String> {
        let text = result
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .map(|candidate| {
                let mut out = String::new();
                for part in &candidate.content.parts {
                    if let Some(t) = &part.text {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(t);
                    }
                }
                out
            })
            .unwrap_or_default();

        if text.is_empty() {
            return Err(super::ProviderError::EmptyResponse {
                provider: "Gemini".into(),
            }
            .into());
        }

        Ok(text)
    }

    /// Classify a `Gemini` API error message string into a typed `ProviderError`.
    ///
    /// `Gemini` embeds HTTP status codes inside JSON error bodies even on
    /// 200-OK SSE streams, so classification is done by substring matching
    /// rather than HTTP status code inspection.
    fn classify_embedded_api_error(message: &str) -> super::ProviderError {
        let normalized = message.to_ascii_lowercase();
        let sanitized = sanitize_api_error(message);
        if normalized.contains("insufficient_quota")
            || normalized.contains("exceeded your current quota")
            || normalized.contains("billing")
        {
            return super::ProviderError::QuotaExhausted {
                provider: "Gemini".into(),
                message: sanitized,
            };
        }
        if normalized.contains("429")
            || normalized.contains("rate limit")
            || normalized.contains("too many requests")
        {
            return super::ProviderError::RateLimited {
                provider: "Gemini".into(),
                status: 429,
                message: sanitized,
            };
        }
        if normalized.contains("401")
            || normalized.contains("403")
            || normalized.contains("unauthorized")
            || normalized.contains("authentication")
            || normalized.contains("invalid api key")
            || normalized.contains("permission denied")
        {
            return super::ProviderError::Auth {
                provider: "Gemini".into(),
                status: 401,
                message: sanitized,
            };
        }
        if normalized.contains("400")
            || normalized.contains("invalid argument")
            || normalized.contains("bad request")
        {
            return super::ProviderError::ClientError {
                provider: "Gemini".into(),
                status: 400,
                message: sanitized,
            };
        }
        super::ProviderError::ServerError {
            provider: "Gemini".into(),
            status: 500,
            message: sanitized,
        }
    }

    /// Convert a slice of `ToolSpec`s to `Gemini` function declarations.
    /// Returns `None` when `tools` is empty (omits the field from the request).
    fn build_gemini_tools(
        tools: &[crate::core::tools::traits::ToolSpec],
    ) -> Option<Vec<GeminiTool>> {
        map_tools_optional(tools, |tool| {
            let fields = ToolFields::from_tool_with_description(
                tool,
                scrub_secrets(&tool.description).into_owned(),
            );

            GeminiFunctionDeclaration {
                name: fields.name,
                description: fields.description,
                parameters: fields.parameters,
            }
        })
        .map(|function_declarations| {
            vec![GeminiTool {
                function_declarations,
            }]
        })
    }

    /// Convert a canonical `ProviderMessage` to a `Gemini` `Content` object.
    ///
    /// `Gemini` uses `"model"` for assistant turns and `"user"` for everything
    /// else. Tool results require a name lookup via `tool_id_to_name` because
    /// `Gemini`'s `FunctionResponse` uses the function name, not its call ID.
    fn map_provider_message(
        provider_message: &ProviderMessage,
        tool_id_to_name: &HashMap<String, String>,
    ) -> Content {
        let role = match provider_message.role {
            MessageRole::Assistant => "model",
            MessageRole::User | MessageRole::System => "user",
        }
        .to_string();

        let parts = provider_message
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => Part::text(scrub_secrets(text).into_owned()),
                ContentBlock::ToolUse { id, name, input } => {
                    let args = if input.is_object() {
                        input.clone()
                    } else {
                        let mut wrapped = Map::new();
                        wrapped.insert("input".to_string(), input.clone());
                        Value::Object(wrapped)
                    };
                    Part::function_call(GeminiFunctionCall {
                        name: name.clone(),
                        args,
                        id: Some(id.clone()),
                    })
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let tool_name = tool_id_to_name
                        .get(tool_use_id)
                        .cloned()
                        .unwrap_or_else(|| "tool".to_string());
                    Part::function_response(GeminiFunctionResponse {
                        name: tool_name,
                        response: serde_json::json!({
                            "tool_use_id": tool_use_id,
                            "content": scrub_secrets(content).into_owned(),
                            "is_error": is_error,
                        }),
                    })
                }
                ContentBlock::Image { source } => match source {
                    ImageSource::Base64 { media_type, data } => {
                        Part::inline_data(GeminiInlineData {
                            mime_type: media_type.clone(),
                            data: data.clone(),
                        })
                    }
                    ImageSource::Url { url } => Part::file_data(GeminiFileData {
                        mime_type: String::new(),
                        file_uri: url.clone(),
                    }),
                },
            })
            .collect();

        Content {
            role: Some(role),
            parts,
        }
    }

    fn build_tools_request(
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[crate::core::tools::traits::ToolSpec],
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> GenerateContentRequest {
        let tool_id_to_name = messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, .. } => Some((id.clone(), name.clone())),
                ContentBlock::Text { .. }
                | ContentBlock::ToolResult { .. }
                | ContentBlock::Image { .. } => None,
            })
            .collect::<HashMap<_, _>>();

        GenerateContentRequest {
            contents: messages
                .iter()
                .map(|message| Self::map_provider_message(message, &tool_id_to_name))
                .collect(),
            system_instruction: system_prompt.map(|system| Content {
                role: None,
                parts: vec![Part::text(scrub_secrets(system).into_owned())],
            }),
            tools: Self::build_gemini_tools(tools),
            generation_config: Self::build_generation_config(temperature, inference_options),
        }
    }

    /// Derive a canonical `StopReason` from a `Gemini` response candidate.
    ///
    /// Tool use is detected first by inspecting `function_call` parts (the
    /// `Gemini` API may not always set `finish_reason = "FUNCTION_CALL"`).
    fn map_stop_reason(candidate: &Candidate) -> StopReason {
        if candidate
            .content
            .parts
            .iter()
            .any(|part| part.function_call.is_some())
        {
            return StopReason::ToolUse;
        }

        match candidate.finish_reason.as_deref() {
            Some("STOP") => StopReason::EndTurn,
            Some("FUNCTION_CALL") => StopReason::ToolUse,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some(_) | None => StopReason::Error,
        }
    }

    /// Convert `Gemini` response parts to canonical `ContentBlock`s.
    /// Synthesizes tool call IDs when the API omits them (older model versions).
    fn parse_content_blocks(parts: &[ResponsePart]) -> Vec<ContentBlock> {
        let mut tool_call_index = 1usize;
        let mut blocks = Vec::new();

        for part in parts {
            if let Some(text) = &part.text {
                let scrubbed = scrub_secrets(text).into_owned();
                if !scrubbed.is_empty() {
                    blocks.push(ContentBlock::Text { text: scrubbed });
                }
            }

            if let Some(function_call) = &part.function_call {
                let input = if function_call.args.is_object() {
                    function_call.args.clone()
                } else {
                    let mut wrapped = Map::new();
                    wrapped.insert("input".to_string(), function_call.args.clone());
                    Value::Object(wrapped)
                };
                let id = function_call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("gemini_call_{tool_call_index}"));
                tool_call_index += 1;
                blocks.push(ContentBlock::ToolUse {
                    id,
                    name: function_call.name.clone(),
                    input,
                });
            }
        }

        blocks
    }

    /// Send a non-streaming `generateContent` request and parse the response.
    /// Checks for embedded error objects in a 200-OK body and classifies them.
    async fn call_api_req(
        &self,
        model: &str,
        request: &GenerateContentRequest,
    ) -> anyhow::Result<GenerateContentResponse> {
        let auth = self.auth()?;
        let url = self.request_url(model, false);

        let response = self.send_api_request(&url, auth, request).await?;

        let result: GenerateContentResponse = response.json().await?;

        if let Some(err) = result.error.as_ref() {
            return Err(Self::classify_embedded_api_error(&err.message).into());
        }

        Ok(result)
    }

    /// Send a `streamGenerateContent?alt=sse` request and return the raw HTTP
    /// response for SSE processing. Adds `?alt=sse` to request SSE framing.
    async fn call_api_streaming(
        &self,
        model: &str,
        request: &GenerateContentRequest,
    ) -> anyhow::Result<reqwest::Response> {
        let auth = self.auth()?;
        let url = self.request_url(model, true);

        self.send_api_request(&url, auth, request).await
    }

    fn events_from_gemini_sse_block(
        event_block: &str,
        sent_start: &mut bool,
        tool_call_index: &mut usize,
    ) -> anyhow::Result<Vec<StreamEvent>> {
        let mut events = Vec::new();

        for data in parse_data_lines(event_block) {
            let gen_response =
                serde_json::from_str::<GenerateContentResponse>(data).map_err(|error| {
                    anyhow::anyhow!("Gemini stream returned malformed SSE JSON chunk: {error}")
                })?;

            if let Some(err) = gen_response.error.as_ref() {
                Err(Self::classify_embedded_api_error(&err.message))?;
            }

            if !*sent_start {
                events.push(StreamEvent::ResponseStart { model: None });
                *sent_start = true;
            }

            if let Some(candidates) = &gen_response.candidates {
                for candidate in candidates {
                    for part in &candidate.content.parts {
                        if let Some(delta_text) = &part.text
                            && !delta_text.is_empty()
                        {
                            events.push(StreamEvent::TextDelta {
                                text: scrub_secrets(delta_text).into_owned(),
                            });
                        }

                        if let Some(fc) = &part.function_call {
                            let id = fc.id.clone().unwrap_or_else(|| {
                                let generated = format!("gemini_call_{tool_call_index}");
                                *tool_call_index += 1;
                                generated
                            });
                            let input = if fc.args.is_object() {
                                fc.args.clone()
                            } else {
                                serde_json::json!({"input": fc.args})
                            };

                            events.push(StreamEvent::ToolCallComplete {
                                id,
                                name: fc.name.clone(),
                                input,
                            });
                        }
                    }

                    if candidate.finish_reason.is_some() {
                        let stop_reason = Self::map_stop_reason(candidate);
                        let (input_tokens, output_tokens) = gen_response
                            .usage_metadata
                            .as_ref()
                            .map_or((None, None), |usage| {
                                (
                                    Some(usage.prompt_token_count),
                                    Some(usage.candidates_token_count),
                                )
                            });

                        events.push(StreamEvent::Done {
                            stop_reason: Some(stop_reason),
                            input_tokens,
                            output_tokens,
                        });
                    }
                }
            }
        }

        Ok(events)
    }

    /// Build a streaming tool-calling request and drive the SSE response into
    /// a `ProviderStream`. Each SSE data line contains a full partial
    /// `GenerateContentResponse`; function calls are emitted as
    /// `ToolCallComplete` events when the API delivers them in a single part.
    async fn chat_with_tools_stream_inner(
        &self,
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[crate::core::tools::traits::ToolSpec],
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ProviderStream> {
        use futures_util::StreamExt;

        let request = Self::build_tools_request(
            system_prompt,
            messages,
            tools,
            temperature,
            inference_options,
        );

        let response = self.call_api_streaming(model, &request).await?;
        let mut byte_stream = response.bytes_stream();

        let stream = async_stream::try_stream! {
            let mut sse_buffer = SseBuffer::new();
            let mut sent_start = false;
            let mut tool_call_index = 1usize;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = chunk_result?;
                sse_buffer.push_chunk(&chunk);

                while let Some(event_block) = sse_buffer.next_event_block() {
                    for event in Self::events_from_gemini_sse_block(
                        &event_block,
                        &mut sent_start,
                        &mut tool_call_index,
                    )? {
                        yield event;
                    }
                }
            }

            if let Some(event_block) = sse_buffer.finish_event_block() {
                for event in Self::events_from_gemini_sse_block(
                    &event_block,
                    &mut sent_start,
                    &mut tool_call_index,
                )? {
                    yield event;
                }
            }
        };

        Ok(Box::pin(stream))
    }

    /// Build and send a simple text-only `Gemini` request.
    async fn call_api(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<GenerateContentResponse> {
        let request = Self::build_request(system_prompt, message, temperature, inference_options);
        self.call_api_req(model, &request).await
    }
}

/// Scale a base output token limit by `factor`, clamped to [0.7, 1.0].
fn scaled_max_tokens(base: u32, factor: f64) -> u32 {
    (f64::from(base) * factor.clamp(0.7, 1.0))
        .round()
        .to_u32()
        .unwrap_or(base)
}

fn gemini_model_supports_generation_capabilities(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }
    normalized.starts_with("gemini-")
        && !normalized.contains("embedding")
        && !normalized.contains("embed")
}

impl Part {
    fn text(text: String) -> Self {
        Self {
            text: Some(text),
            function_call: None,
            function_response: None,
            inline_data: None,
            file_data: None,
        }
    }

    fn function_call(function_call: GeminiFunctionCall) -> Self {
        Self {
            text: None,
            function_call: Some(function_call),
            function_response: None,
            inline_data: None,
            file_data: None,
        }
    }

    fn function_response(function_response: GeminiFunctionResponse) -> Self {
        Self {
            text: None,
            function_call: None,
            function_response: Some(function_response),
            inline_data: None,
            file_data: None,
        }
    }

    fn inline_data(data: GeminiInlineData) -> Self {
        Self {
            text: None,
            function_call: None,
            function_response: None,
            inline_data: Some(data),
            file_data: None,
        }
    }

    fn file_data(data: GeminiFileData) -> Self {
        Self {
            text: None,
            function_call: None,
            function_response: None,
            inline_data: None,
            file_data: Some(data),
        }
    }
}

impl Provider for GeminiProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        let supports_generation = gemini_model_supports_generation_capabilities(model);
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: supports_generation,
            streaming: true,
            vision: supports_generation,
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
            let result = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            Self::extract_text(&result).map_err(Into::into)
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
            let result = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            Self::extract_text(&result).map_err(Into::into)
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
            let result = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            let text = Self::extract_text(&result)?;
            let mut provider_response = if let Some(usage) = result.usage_metadata {
                ProviderResponse::with_usage(
                    text,
                    usage.prompt_token_count,
                    usage.candidates_token_count,
                )
            } else {
                ProviderResponse::text_only(text)
            };
            if let Some(model_version) = result.model_version {
                provider_response = provider_response.with_model(model_version);
            }
            Ok(provider_response)
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
            let result = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            let text = Self::extract_text(&result)?;
            let mut provider_response = if let Some(usage) = result.usage_metadata {
                ProviderResponse::with_usage(
                    text,
                    usage.prompt_token_count,
                    usage.candidates_token_count,
                )
            } else {
                ProviderResponse::text_only(text)
            };
            if let Some(model_version) = result.model_version {
                provider_response = provider_response.with_model(model_version);
            }
            Ok(provider_response)
        })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [crate::core::tools::traits::ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let request =
                Self::build_tools_request(system_prompt, messages, tools, temperature, None);
            let result = self.call_api_req(model, &request).await?;

            let candidate = result
                .candidates
                .as_ref()
                .and_then(|candidates| candidates.first())
                .ok_or_else(|| super::ProviderError::EmptyResponse {
                    provider: "Gemini".into(),
                })?;

            let content_blocks = Self::parse_content_blocks(&candidate.content.parts);
            let text = {
                let mut out = String::new();
                for block in &content_blocks {
                    if let ContentBlock::Text { text: t } = block {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(t);
                    }
                }
                out
            };

            let mut provider_response = if let Some(usage) = result.usage_metadata {
                ProviderResponse::with_usage(
                    text,
                    usage.prompt_token_count,
                    usage.candidates_token_count,
                )
            } else {
                ProviderResponse::text_only(text)
            };

            provider_response.content_blocks = content_blocks;
            provider_response.stop_reason = Some(Self::map_stop_reason(candidate));

            if let Some(model_version) = result.model_version {
                provider_response = provider_response.with_model(model_version);
            }

            Ok(provider_response)
        })
    }

    fn chat_with_tools_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [crate::core::tools::traits::ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let request = Self::build_tools_request(
                system_prompt,
                messages,
                tools,
                temperature,
                inference_options,
            );
            let result = self.call_api_req(model, &request).await?;

            let candidate = result
                .candidates
                .as_ref()
                .and_then(|candidates| candidates.first())
                .ok_or_else(|| super::ProviderError::EmptyResponse {
                    provider: "Gemini".into(),
                })?;

            let content_blocks = Self::parse_content_blocks(&candidate.content.parts);
            let text = {
                let mut out = String::new();
                for block in &content_blocks {
                    if let ContentBlock::Text { text: t } = block {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(t);
                    }
                }
                out
            };

            let mut provider_response = if let Some(usage) = result.usage_metadata {
                ProviderResponse::with_usage(
                    text,
                    usage.prompt_token_count,
                    usage.candidates_token_count,
                )
            } else {
                ProviderResponse::text_only(text)
            };

            provider_response.content_blocks = content_blocks;
            provider_response.stop_reason = Some(Self::map_stop_reason(candidate));

            if let Some(model_version) = result.model_version {
                provider_response = provider_response.with_model(model_version);
            }

            Ok(provider_response)
        })
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [crate::core::tools::traits::ToolSpec],
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
        tools: &'a [crate::core::tools::traits::ToolSpec],
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
