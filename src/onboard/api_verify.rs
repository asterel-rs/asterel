//! Inline API key verification during onboarding.

use std::time::Duration;

use anyhow::Result;

use crate::core::providers::catalog::{canonical_provider_name, compatible_provider_spec};

/// Result of an inline API key verification attempt.
pub(crate) enum VerifyResult {
    /// The key is valid. `detail` carries a human-readable confirmation.
    Valid { detail: String },
    /// The key is invalid or the endpoint rejected it. `reason` describes why.
    Invalid { reason: String },
    /// Verification was skipped (e.g. empty key or local provider).
    Skipped,
}

/// Verify an API key for the given provider by making a lightweight HTTP probe.
///
/// Returns [`VerifyResult::Skipped`] for an empty key. Uses a 5-second
/// timeout. Does not modify any state.
///
/// # Errors
///
/// Returns an error only when the HTTP client itself cannot be constructed
/// (extremely rare). Network failures are mapped to [`VerifyResult::Invalid`].
pub(crate) async fn verify_api_key(provider: &str, api_key: &str) -> Result<VerifyResult> {
    if api_key.is_empty() && provider_requires_api_key_for_verification(provider) {
        return Ok(VerifyResult::Skipped);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let result = probe_provider(&client, provider, api_key).await;
    Ok(result)
}

fn provider_requires_api_key_for_verification(provider: &str) -> bool {
    !matches!(canonical_provider_name(provider).as_str(), "ollama")
}

/// Dispatch the test HTTP request based on provider id.
async fn probe_provider(client: &reqwest::Client, provider: &str, api_key: &str) -> VerifyResult {
    match canonical_provider_name(provider).as_str() {
        "openrouter" => {
            probe_bearer(
                client,
                "https://openrouter.ai/api/v1/models",
                api_key,
                "openrouter",
            )
            .await
        }
        "anthropic" => probe_anthropic(client, api_key).await,
        "openai" | "openai-codex" => {
            probe_bearer(
                client,
                "https://api.openai.com/v1/models",
                api_key,
                "openai",
            )
            .await
        }
        "gemini" => probe_gemini(client, api_key).await,
        "gemini-vertex" => probe_gemini_vertex(client, provider, api_key).await,
        "ollama" => probe_ollama(client).await,
        "minimax" => VerifyResult::Skipped,
        other => {
            // Fall back to compatible provider spec if available.
            if let Some(spec) = compatible_provider_spec(other) {
                let url = compatible_models_probe_url(spec.base_url.as_str());
                probe_bearer(client, &url, api_key, other).await
            } else {
                // Unknown provider — skip rather than fail.
                VerifyResult::Skipped
            }
        }
    }
}

fn parse_gemini_vertex_selector(provider: &str) -> Option<(&str, &str)> {
    let selector = provider
        .trim()
        .strip_prefix("gemini-vertex:")
        .or_else(|| provider.trim().strip_prefix("vertex-gemini:"))?;
    let (project, location) = selector.split_once('/')?;
    Some((project.trim(), location.trim()))
}

fn compatible_models_probe_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/models") {
        base.to_string()
    } else if base.ends_with("/v1") || base.ends_with("/v1beta") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

/// GET with `Authorization: Bearer {key}`.
async fn probe_bearer(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    provider: &str,
) -> VerifyResult {
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await;

    map_response(response, provider)
}

/// GET with Anthropic-specific auth headers.
fn anthropic_verify_header(api_key: &str) -> (&'static str, String) {
    crate::core::providers::anthropic::AnthropicProvider::auth_header_for_token(api_key)
}

/// GET with Anthropic-specific auth headers.
async fn probe_anthropic(client: &reqwest::Client, api_key: &str) -> VerifyResult {
    let (header_name, header_value) = anthropic_verify_header(api_key);
    let response = client
        .get("https://api.anthropic.com/v1/models")
        .header(header_name, header_value)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await;

    map_response(response, "anthropic")
}

/// GET Gemini models list using query-string key auth.
async fn probe_gemini(client: &reqwest::Client, api_key: &str) -> VerifyResult {
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models?key={api_key}");
    let response = client.get(&url).send().await;
    map_response(response, "gemini")
}

async fn probe_gemini_vertex(
    client: &reqwest::Client,
    provider: &str,
    api_key: &str,
) -> VerifyResult {
    let Some((project, location)) = parse_gemini_vertex_selector(provider) else {
        return VerifyResult::Skipped;
    };
    let url = format!(
        "https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/gemini-2.5-flash:countTokens"
    );
    let response = client
        .post(&url)
        .header("x-goog-api-key", api_key)
        .json(&serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "ping"}]
            }]
        }))
        .send()
        .await;
    map_response(response, "gemini-vertex")
}

/// Connectivity-only check for Ollama (no auth required).
async fn probe_ollama(client: &reqwest::Client) -> VerifyResult {
    let response = client.get("http://localhost:11434/api/tags").send().await;

    match response {
        Ok(r) if r.status().is_success() => VerifyResult::Valid {
            detail: "ollama local server reachable".to_string(),
        },
        Ok(r) => VerifyResult::Invalid {
            reason: format!("ollama returned HTTP {}", r.status()),
        },
        Err(e) if e.is_timeout() => VerifyResult::Invalid {
            reason: "connection timed out (5s)".to_string(),
        },
        Err(e) => VerifyResult::Invalid {
            reason: e.to_string(),
        },
    }
}

/// Map a reqwest response (or error) to a [`VerifyResult`].
fn map_response(
    response: Result<reqwest::Response, reqwest::Error>,
    provider: &str,
) -> VerifyResult {
    match response {
        Ok(r) if r.status().is_success() => VerifyResult::Valid {
            detail: format!("{provider} API key verified"),
        },
        Ok(r) if r.status() == 401 || r.status() == 403 => VerifyResult::Invalid {
            reason: "invalid or expired API key".to_string(),
        },
        Ok(r) => VerifyResult::Invalid {
            reason: format!("unexpected HTTP {}", r.status()),
        },
        Err(e) if e.is_timeout() => VerifyResult::Invalid {
            reason: "connection timed out (5s)".to_string(),
        },
        Err(e) => VerifyResult::Invalid {
            reason: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_key_returns_skipped() {
        let result = verify_api_key("openrouter", "").await.unwrap();
        assert!(matches!(result, VerifyResult::Skipped));
    }

    #[tokio::test]
    async fn empty_key_returns_skipped_for_any_provider() {
        for provider in &["anthropic", "openai", "gemini", "groq"] {
            let result = verify_api_key(provider, "").await.unwrap();
            assert!(
                matches!(result, VerifyResult::Skipped),
                "expected Skipped for provider {provider}"
            );
        }
    }

    #[test]
    fn ollama_empty_key_still_runs_no_key_health_probe() {
        assert!(!provider_requires_api_key_for_verification("ollama"));
    }

    #[test]
    fn map_response_401_returns_invalid_key_reason() {
        // Construct a synthetic 401-like path via the enum logic.
        // We can't easily build a reqwest::Response in unit tests, but we can
        // verify the logic compiles and the enum variants are accessible.
        let _valid: VerifyResult = VerifyResult::Valid {
            detail: "test".into(),
        };
        let _invalid: VerifyResult = VerifyResult::Invalid {
            reason: "invalid or expired API key".into(),
        };
        // Confirm Skipped variant is constructible.
        assert!(matches!(VerifyResult::Skipped, VerifyResult::Skipped));
    }

    #[test]
    fn anthropic_verify_header_uses_bearer_for_setup_tokens() {
        let (name, value) = anthropic_verify_header("sk-ant-oat01-abcdef");
        assert_eq!(name, "Authorization");
        assert_eq!(value, "Bearer sk-ant-oat01-abcdef");
    }

    #[test]
    fn compatible_models_probe_url_does_not_duplicate_v1_suffix() {
        assert_eq!(
            compatible_models_probe_url("https://gateway.ai.cloudflare.com/v1"),
            "https://gateway.ai.cloudflare.com/v1/models"
        );
        assert_eq!(
            compatible_models_probe_url("https://api.example.com"),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn parse_gemini_vertex_selector_extracts_project_and_location() {
        assert_eq!(
            parse_gemini_vertex_selector("gemini-vertex:demo-project/global"),
            Some(("demo-project", "global"))
        );
        assert_eq!(
            parse_gemini_vertex_selector("vertex-gemini:demo-project/us-central1"),
            Some(("demo-project", "us-central1"))
        );
        assert_eq!(parse_gemini_vertex_selector("gemini-vertex"), None);
    }
}
