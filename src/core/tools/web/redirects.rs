//! SSRF-safe redirect follower for web fetch and scrape tools.
//!
//! # What it does
//!
//! `get_with_validated_redirects` follows HTTP 3xx chains up to `max_redirects`
//! hops by issuing each request manually rather than letting `reqwest` handle
//! redirects automatically. At each hop the new URL is passed through
//! `security::validate_fetch_url`, which re-applies the SSRF checks
//! (private-IP rejection, scheme enforcement). This prevents redirect-based
//! SSRF attacks where a public URL redirects to an internal service.
//!
//! Relative `Location` headers are resolved against the base URL of the
//! previous response via `resolve_redirect_url`.

use anyhow::Context;
use std::time::Duration;

pub(super) async fn get_with_validated_redirects(
    start_url: &url::Url,
    max_redirects: usize,
    require_https: bool,
) -> anyhow::Result<reqwest::Response> {
    let mut current_url = start_url.clone();

    for redirect_count in 0..=max_redirects {
        let client = web_fetch_client_for_url(&current_url).await?;
        let response = client
            .get(current_url.as_str())
            .send()
            .await
            .with_context(|| format!("request failed for {current_url}"))?;

        if response.status().is_redirection() {
            if redirect_count == max_redirects {
                anyhow::bail!(
                    "URL fetch aborted after {max_redirects} redirects (possible redirect loop): last URL was {current_url}"
                );
            }

            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .context("redirect response missing Location header")?
                .to_str()
                .context("invalid redirect Location header")?;

            let next_url = resolve_redirect_url(response.url(), location)?;
            current_url = crate::security::validate_fetch_url(next_url.as_str(), require_https)
                .await
                .context("redirect target rejected")?;
            continue;
        }

        return response.error_for_status().map_err(Into::into);
    }

    unreachable!("redirect loop always returns before exhausting iterations")
}

async fn web_fetch_client_for_url(url: &url::Url) -> anyhow::Result<reqwest::Client> {
    crate::utils::http::try_build_pinned_public_fetch_client_with(
        url,
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Asterel/0.1")
            .redirect(reqwest::redirect::Policy::none()),
    )
    .await
}

fn resolve_redirect_url(base_url: &url::Url, location: &str) -> anyhow::Result<url::Url> {
    match url::Url::parse(location) {
        Ok(url) => Ok(url),
        Err(_) => Ok(base_url.join(location)?),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_redirect_url;

    #[test]
    fn resolves_absolute_redirect_target() {
        let base = url::Url::parse("https://example.com/a").expect("valid base URL");
        let resolved =
            resolve_redirect_url(&base, "https://example.org/path").expect("absolute URL");
        assert_eq!(resolved.as_str(), "https://example.org/path");
    }

    #[test]
    fn resolves_relative_redirect_target() {
        let base = url::Url::parse("https://example.com/dir/page").expect("valid base URL");
        let resolved = resolve_redirect_url(&base, "../next").expect("relative URL");
        assert_eq!(resolved.as_str(), "https://example.com/next");
    }
}
