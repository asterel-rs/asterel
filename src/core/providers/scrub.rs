//! Provider-specific error sanitization.
//!
//! The general-purpose scrubbing functions (`scrub_secrets`,
//! `sanitize_api_error`) live in `security::scrub`.  This module
//! provides only the provider-specific `api_error()` helper that
//! depends on [`super::ProviderError`].

/// Build a sanitized provider error from a failed HTTP response.
///
/// Returns a typed [`super::ProviderError`] wrapped as `anyhow::Error`.
pub async fn api_error(provider: &str, response: reqwest::Response) -> anyhow::Error {
    super::ProviderError::from_http_response(provider, response)
        .await
        .into()
}
