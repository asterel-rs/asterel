//! Shared HTTP API response handling for channel adapters.
//!
//! Channel APIs commonly return provider-owned JSON/text error bodies and
//! `429 Too Many Requests` responses. Keep redaction and retry-delay parsing in
//! one small transport helper so individual adapters do not drift.

use std::time::Duration;

use reqwest::header::HeaderMap;

use crate::security::scrub::sanitize_api_error;

pub(crate) const CHANNEL_API_MAX_RATE_LIMIT_RETRIES: u8 = 3;
const CHANNEL_API_MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

pub(crate) fn retry_after_duration(headers: &HeaderMap) -> Duration {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(Duration::from_secs_f64)
        .map_or_else(
            || Duration::from_secs(1),
            |duration| duration.min(CHANNEL_API_MAX_RETRY_AFTER),
        )
}

pub(crate) async fn wait_for_rate_limit(headers: &HeaderMap) {
    tokio::time::sleep(retry_after_duration(headers)).await;
}

pub(crate) async fn channel_api_error_message(
    service: &str,
    operation: &str,
    response: reqwest::Response,
) -> String {
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("<failed to read response body: {error}>"));
    let sanitized_body = sanitize_api_error(&body);

    if status.as_u16() == 429 {
        format!(
            "{service} {operation} exceeded rate limit after {CHANNEL_API_MAX_RATE_LIMIT_RETRIES} retries ({status}): {sanitized_body}"
        )
    } else {
        format!("{service} {operation} failed ({status}): {sanitized_body}")
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    use super::retry_after_duration;

    #[test]
    fn retry_after_duration_parses_fractional_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("0.25"));

        let duration = retry_after_duration(&headers);

        assert_eq!(duration.as_millis(), 250);
    }

    #[test]
    fn retry_after_duration_defaults_on_invalid_header() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("not-a-duration"));

        let duration = retry_after_duration(&headers);

        assert_eq!(duration.as_secs(), 1);
    }

    #[test]
    fn retry_after_duration_clamps_huge_header() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("86400"));

        let duration = retry_after_duration(&headers);

        assert_eq!(duration.as_secs(), 30);
    }
}
