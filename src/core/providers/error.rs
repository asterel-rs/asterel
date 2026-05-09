//! Typed provider error enum for structured error classification.
//!
//! Enables retry/fallback/recovery logic without fragile string
//! parsing of `anyhow::Error` messages.

use super::sanitize_api_error;

/// Typed error for provider operations.
///
/// Enables structured error classification for retry/fallback/recovery logic
/// instead of fragile string-parsing of `anyhow::Error` messages.
///
/// Implements `std::error::Error` via `thiserror`, so it auto-converts to
/// `anyhow::Error` where boundary erasure is required.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Authentication failure (401/403) — may be recoverable via OAuth refresh.
    #[error("{provider} authentication failed ({status}): {message}")]
    Auth {
        provider: String,
        status: u16,
        message: String,
    },

    /// Rate limited (429) — retryable after backoff.
    #[error("{provider} rate limited ({status}): {message}")]
    RateLimited {
        provider: String,
        status: u16,
        message: String,
    },

    /// Non-retryable client error (400, 404, etc.) — skip retries, try fallback.
    #[error("{provider} client error ({status}): {message}")]
    ClientError {
        provider: String,
        status: u16,
        message: String,
    },

    /// Server error (5xx) or timeout — retryable.
    #[error("{provider} server error ({status}): {message}")]
    ServerError {
        provider: String,
        status: u16,
        message: String,
    },

    /// Quota exhausted (`billing`/`insufficient_quota`) — non-retryable.
    #[error("{provider} quota exhausted: {message}")]
    QuotaExhausted { provider: String, message: String },

    /// Missing credentials — non-retryable configuration error.
    #[error("{provider} credentials not configured: {message}")]
    MissingCredentials { provider: String, message: String },

    /// Network/transport error (connection reset, DNS, TLS).
    #[error("{provider} network error: {source}")]
    Network {
        provider: String,
        #[source]
        source: reqwest::Error,
    },

    /// Response parsing failure (invalid JSON, missing fields).
    #[error("{provider} response parse error: {message}")]
    ResponseParse { provider: String, message: String },

    /// No response content from provider (empty response body).
    #[error("{provider} returned no content")]
    EmptyResponse { provider: String },

    /// All providers in a fallback chain exhausted.
    #[error("All providers failed. Attempts:\n{summary}")]
    AllProvidersFailed { summary: String },
}

/// Error type returned by the provider trait boundary.
///
/// Preserves structured [`ProviderError`] variants while still allowing
/// untyped fallback errors where needed.
#[derive(Debug, thiserror::Error)]
pub enum ProviderCallError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ProviderCallError {
    #[must_use]
    pub fn is_auth_error(&self) -> bool {
        matches!(self, Self::Provider(err) if err.is_auth_error())
    }

    #[must_use]
    pub fn is_non_retryable(&self) -> bool {
        matches!(self, Self::Provider(err) if err.is_non_retryable())
    }
}

pub type ProviderResult<T> = std::result::Result<T, ProviderCallError>;

impl ProviderError {
    /// Whether this error is non-retryable (should skip remaining retries).
    #[must_use]
    pub fn is_non_retryable(&self) -> bool {
        matches!(
            self,
            Self::Auth { .. }
                | Self::ClientError { .. }
                | Self::QuotaExhausted { .. }
                | Self::MissingCredentials { .. }
                | Self::ResponseParse { .. }
                | Self::EmptyResponse { .. }
        )
    }

    /// Whether this error is an auth failure that might be recoverable via
    /// OAuth token refresh.
    #[must_use]
    pub fn is_auth_error(&self) -> bool {
        matches!(self, Self::Auth { .. })
    }

    /// Whether this error indicates quota/billing exhaustion.
    #[must_use]
    pub fn is_quota_exhausted(&self) -> bool {
        matches!(self, Self::QuotaExhausted { .. })
    }

    /// Construct from an HTTP response. Classifies by status code and body content.
    pub async fn from_http_response(provider: &str, response: reqwest::Response) -> Self {
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read provider error body>".to_string());
        let message = sanitize_api_error(&body);

        if is_quota_message(&body) {
            return Self::QuotaExhausted {
                provider: provider.to_string(),
                message,
            };
        }

        if status == 401 || status == 403 {
            return Self::Auth {
                provider: provider.to_string(),
                status,
                message,
            };
        }

        if status == 429 {
            return Self::RateLimited {
                provider: provider.to_string(),
                status,
                message,
            };
        }

        // 4xx (except 429, 408) are non-retryable client errors
        if (400..500).contains(&status) && status != 408 {
            return Self::ClientError {
                provider: provider.to_string(),
                status,
                message,
            };
        }

        Self::ServerError {
            provider: provider.to_string(),
            status,
            message,
        }
    }
}

fn is_quota_message(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("insufficient_quota")
        || lower.contains("exceeded your current quota")
        || lower.contains("billing")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_retryable_classification() {
        let auth = ProviderError::Auth {
            provider: "test".into(),
            status: 401,
            message: "unauthorized".into(),
        };
        assert!(auth.is_non_retryable());
        assert!(auth.is_auth_error());

        let rate = ProviderError::RateLimited {
            provider: "test".into(),
            status: 429,
            message: "slow down".into(),
        };
        assert!(!rate.is_non_retryable());
        assert!(!rate.is_auth_error());

        let server = ProviderError::ServerError {
            provider: "test".into(),
            status: 500,
            message: "internal".into(),
        };
        assert!(!server.is_non_retryable());

        let client = ProviderError::ClientError {
            provider: "test".into(),
            status: 400,
            message: "bad request".into(),
        };
        assert!(client.is_non_retryable());

        let quota = ProviderError::QuotaExhausted {
            provider: "test".into(),
            message: "exceeded".into(),
        };
        assert!(quota.is_non_retryable());
        assert!(quota.is_quota_exhausted());

        let creds = ProviderError::MissingCredentials {
            provider: "test".into(),
            message: "set API key".into(),
        };
        assert!(creds.is_non_retryable());
    }

    #[test]
    fn display_format() {
        let err = ProviderError::Auth {
            provider: "Anthropic".into(),
            status: 401,
            message: "invalid key".into(),
        };
        assert_eq!(
            err.to_string(),
            "Anthropic authentication failed (401): invalid key"
        );
    }

    #[test]
    fn provider_call_error_preserves_typed_variant() {
        let err = ProviderError::ServerError {
            provider: "test".into(),
            status: 502,
            message: "bad gateway".into(),
        };
        let call_err: ProviderCallError = err.into();
        assert!(matches!(
            call_err,
            ProviderCallError::Provider(ProviderError::ServerError { .. })
        ));
    }

    #[test]
    fn provider_call_error_wraps_untyped_anyhow() {
        let call_err: ProviderCallError = anyhow::anyhow!("boom").into();
        assert!(matches!(call_err, ProviderCallError::Other(_)));
    }

    #[test]
    fn quota_detection() {
        assert!(is_quota_message("insufficient_quota"));
        assert!(is_quota_message("You exceeded your current quota"));
        assert!(is_quota_message("billing issue"));
        assert!(!is_quota_message("normal error"));
    }
}
