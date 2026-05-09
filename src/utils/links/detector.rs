//! URL detection in free-form text.
//!
//! Extracts deduplicated HTTP/HTTPS URLs from prose, handling
//! markdown link syntax and trailing punctuation stripping.

use std::collections::HashSet;

use url::Url;

/// Detect HTTP/HTTPS URLs in text. Returns deduplicated URLs in order of appearance.
#[must_use]
pub(crate) fn detect_urls(text: &str) -> Vec<Url> {
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for token in text.split_whitespace() {
        let candidate = extract_candidate(token);
        if let Some(url) = try_parse_url(&candidate) {
            let key = url.to_string();
            if seen.insert(key) {
                urls.push(url);
            }
        }
    }

    urls
}

fn extract_candidate(token: &str) -> String {
    if let Some(start) = token.find("](")
        && let Some(end) = token[start..].find(')')
    {
        let url_part = &token[start + 2..start + end];
        return url_part.to_string();
    }

    let stripped = token
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(token);

    let stripped = stripped
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(stripped);

    strip_trailing_punctuation(stripped).to_string()
}

fn strip_trailing_punctuation(s: &str) -> &str {
    // Only strip single-byte ASCII punctuation; this is safe at UTF-8
    // boundaries because every character checked is a 7-bit ASCII byte.
    s.trim_end_matches(['.', ',', ';', '!', '?', ')'])
}

fn try_parse_url(candidate: &str) -> Option<Url> {
    let url = Url::parse(candidate).ok()?;
    match url.scheme() {
        "http" | "https" => Some(url),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_url() {
        let urls = detect_urls("check https://example.com for info");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].as_str(), "https://example.com/");
    }

    #[test]
    fn multiple_urls() {
        let urls = detect_urls("visit https://a.com and http://b.org today");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].host_str(), Some("a.com"));
        assert_eq!(urls[1].host_str(), Some("b.org"));
    }

    #[test]
    fn deduplication() {
        let urls = detect_urls("https://example.com and https://example.com again");
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn angle_brackets() {
        let urls = detect_urls("see <https://example.com/path> for details");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].path(), "/path");
    }

    #[test]
    fn trailing_punctuation() {
        let urls = detect_urls("Go to https://example.com/page.");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].path(), "/page");

        let urls = detect_urls("Is it https://example.com?");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].as_str(), "https://example.com/");
    }

    #[test]
    fn parentheses() {
        let urls = detect_urls("(https://example.com/path)");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].path(), "/path");
    }

    #[test]
    fn markdown_link() {
        let urls = detect_urls("click [here](https://example.com/doc) now");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].path(), "/doc");
    }

    #[test]
    fn no_urls() {
        let urls = detect_urls("just some regular text with no links");
        assert!(urls.is_empty());
    }

    #[test]
    fn non_http_schemes_ignored() {
        let urls = detect_urls("ftp://files.example.com mailto:user@example.com");
        assert!(urls.is_empty());
    }

    #[test]
    fn url_with_query_and_fragment() {
        let urls = detect_urls("https://example.com/search?q=test#results");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].query(), Some("q=test"));
        assert_eq!(urls[0].fragment(), Some("results"));
    }

    #[test]
    fn preserves_order() {
        let urls = detect_urls("https://c.com https://a.com https://b.com");
        assert_eq!(urls[0].host_str(), Some("c.com"));
        assert_eq!(urls[1].host_str(), Some("a.com"));
        assert_eq!(urls[2].host_str(), Some("b.com"));
    }

    #[test]
    fn malformed_urls_are_ignored() {
        let urls =
            detect_urls("https:// http:///bad https://:443 ftp://example.com https://ok.dev");
        assert!(urls.iter().any(|url| url.host_str() == Some("ok.dev")));
        assert!(
            urls.iter()
                .all(|url| matches!(url.scheme(), "http" | "https"))
        );
        assert!(!urls.iter().any(|url| url.scheme() == "ftp"));
    }

    #[test]
    fn unicode_url_path_is_detected() {
        let urls = detect_urls("see https://example.com/na%C3%AFve/%E6%97%A5%E6%9C%AC");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].path(), "/na%C3%AFve/%E6%97%A5%E6%9C%AC");
    }

    #[test]
    fn internationalized_domain_name_is_normalized() {
        let urls = detect_urls("visit https://bücher.example/path");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].host_str(), Some("xn--bcher-kva.example"));
    }
}
