//! Secret scrubbing utilities for logs, errors, and user-facing output.
//!
//! Lives in the security layer (L0) so that approval types and other L0
//! modules can use it without depending on higher layers.

use std::borrow::Cow;

const MAX_API_ERROR_CHARS: usize = 200;

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '+' | '/' | '=')
}

fn token_end(input: &str, from: usize) -> usize {
    let mut end = from;
    for (i, c) in input[from..].char_indices() {
        if is_secret_char(c) {
            end = from + i + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn scrub_after_marker(scrubbed: &mut String, marker: &str) -> bool {
    let mut modified = false;
    let mut search_from = 0;
    while let Some(rel) = scrubbed[search_from..].find(marker) {
        let start = search_from + rel;
        let content_start = start + marker.len();
        let end = token_end(scrubbed, content_start);

        // Skip bare markers without a token value.
        if end == content_start {
            search_from = content_start;
            continue;
        }

        scrubbed.replace_range(start..end, "[REDACTED]");
        modified = true;
        search_from = start + "[REDACTED]".len();
    }

    modified
}

fn scrub_pem_blocks(scrubbed: &mut String) -> bool {
    const PEM_BEGIN_MARKER: &str = "-----BEGIN ";
    const PEM_LINE_SUFFIX: &str = "-----";
    const REDACTED_PEM: &str = "[REDACTED-PEM]";

    let mut modified = false;
    let mut search_from = 0;

    while let Some(rel_begin) = scrubbed[search_from..].find(PEM_BEGIN_MARKER) {
        let begin = search_from + rel_begin;
        let kind_start = begin + PEM_BEGIN_MARKER.len();
        let Some(rel_kind_end) = scrubbed[kind_start..].find(PEM_LINE_SUFFIX) else {
            search_from = kind_start;
            continue;
        };

        let kind_end = kind_start + rel_kind_end;
        if kind_end == kind_start {
            search_from = kind_start;
            continue;
        }

        let kind = &scrubbed[kind_start..kind_end];
        let end_marker = format!("-----END {kind}-----");
        let end_search_from = kind_end + PEM_LINE_SUFFIX.len();
        let Some(rel_end) = scrubbed[end_search_from..].find(&end_marker) else {
            search_from = kind_start;
            continue;
        };

        let end_start = end_search_from + rel_end;
        let mut replace_end = end_start + end_marker.len();
        if scrubbed[replace_end..].starts_with("\r\n") {
            replace_end += 2;
        } else if scrubbed[replace_end..].starts_with('\n') {
            replace_end += 1;
        }

        scrubbed.replace_range(begin..replace_end, REDACTED_PEM);
        modified = true;
        search_from = begin + REDACTED_PEM.len();
    }

    modified
}

const PREFIX_PATTERNS: [&str; 23] = [
    "sk-",
    "xoxb-",
    "xoxp-",
    "xoxs-",
    "xoxa-",
    "xoxe-",
    "xapp-",
    "ghp_",
    "github_pat_",
    "hf_",
    "glpat-",
    "ya29.",
    "AIza",
    "AKIA",
    "ASIA",
    "eyJ",
    "-----BEGIN ",
    "GOCSPX-",
    "gho_",
    "ghu_",
    "ghs_",
    "sshpass-",
    "AGE-SECRET-KEY-",
];

const MARKER_PATTERNS: [&str; 23] = [
    "Authorization: Bearer ",
    "authorization: bearer ",
    "\"authorization\":\"Bearer ",
    "\"authorization\":\"bearer ",
    "api_key=",
    "access_token=",
    "refresh_token=",
    "id_token=",
    "\"api_key\":\"",
    "\"access_token\":\"",
    "\"refresh_token\":\"",
    "\"id_token\":\"",
    "\"token\":\"",
    "\"secret\":\"",
    "\"password\":\"",
    "\"private_key\":\"",
    "\"client_secret\":\"",
    "\"database_url\":\"",
    "password=",
    "secret=",
    "DATABASE_URL=",
    "PRIVATE_KEY=",
    "SECRET_KEY=",
];

fn needs_scrubbing(input: &str) -> bool {
    PREFIX_PATTERNS
        .iter()
        .chain(MARKER_PATTERNS.iter())
        .any(|pattern| input.contains(pattern))
}

/// Scrub known secret-like token patterns from arbitrary text.
///
/// Redacts provider keys and tokens in common forms:
/// - Prefix tokens: `sk-`, `xoxb-`, `ghp_`, etc.
/// - Header/query/json markers: `Authorization: Bearer ...`, `api_key=...`, `"access_token":"..."`
#[must_use]
pub fn scrub_secrets(input: &str) -> Cow<'_, str> {
    if !needs_scrubbing(input) {
        return Cow::Borrowed(input);
    }

    let mut scrubbed = input.to_string();

    for pattern in PREFIX_PATTERNS {
        if pattern == "-----BEGIN " {
            continue;
        }
        scrub_after_marker(&mut scrubbed, pattern);
    }

    for marker in MARKER_PATTERNS {
        scrub_after_marker(&mut scrubbed, marker);
    }

    scrub_pem_blocks(&mut scrubbed);

    Cow::Owned(scrubbed)
}

/// Sanitize API error text by scrubbing secrets and truncating length.
#[must_use]
pub fn sanitize_api_error(input: &str) -> String {
    let scrubbed = scrub_secrets(input);

    // Byte length is always >= char count; if bytes fit, chars fit too.
    if scrubbed.len() <= MAX_API_ERROR_CHARS || scrubbed.chars().count() <= MAX_API_ERROR_CHARS {
        return scrubbed.into_owned();
    }

    let scrubbed = scrubbed.as_ref();
    let mut end = MAX_API_ERROR_CHARS;
    while end > 0 && !scrubbed.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}...", &scrubbed[..end])
}

#[cfg(test)]
mod tests {
    use super::scrub_secrets;

    #[test]
    fn scrubs_aws_access_key_prefixes() {
        let input = "aws keys AKIA1234567890ABCDEF and ASIA1234567890ABCDEF";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("AKIA1234567890ABCDEF"));
        assert!(!scrubbed.contains("ASIA1234567890ABCDEF"));
        assert_eq!(scrubbed.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn scrubs_jwt_prefix_tokens() {
        let input = "jwt eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.signature";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.signature"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn scrubs_multiline_pem_blocks() {
        let input = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAu\nline2\n-----END RSA PRIVATE KEY-----\nafter\n";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("BEGIN RSA PRIVATE KEY"));
        assert!(!scrubbed.contains("MIIEowIBAAKCAQEAu"));
        assert!(!scrubbed.contains("END RSA PRIVATE KEY"));
        assert!(scrubbed.contains("[REDACTED-PEM]"));
    }

    #[test]
    fn scrubs_additional_github_tokens() {
        let input = "gho_1234567890abcdef ghu_1234567890abcdef ghs_1234567890abcdef";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("gho_1234567890abcdef"));
        assert!(!scrubbed.contains("ghu_1234567890abcdef"));
        assert!(!scrubbed.contains("ghs_1234567890abcdef"));
        assert_eq!(scrubbed.matches("[REDACTED]").count(), 3);
    }

    #[test]
    fn scrubs_new_json_secret_fields() {
        let input = r#"{"secret":"abc123","password":"hunter2","private_key":"key123","client_secret":"sec123","database_url":"postgres://user:passhost/db"}"#;
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("abc123"));
        assert!(!scrubbed.contains("hunter2"));
        assert!(!scrubbed.contains("key123"));
        assert!(!scrubbed.contains("sec123"));
        assert!(!scrubbed.contains("postgres://user:passhost/db"));
        assert_eq!(scrubbed.matches("[REDACTED]").count(), 5);
    }

    #[test]
    fn scrubs_env_and_query_secret_markers() {
        let input = "DATABASE_URL=postgres://user:passhost/db PRIVATE_KEY=abc123 SECRET_KEY=def456 password=hunter2 secret=s3cr3t";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("postgres://user:passhost/db"));
        assert!(!scrubbed.contains("abc123"));
        assert!(!scrubbed.contains("def456"));
        assert!(!scrubbed.contains("hunter2"));
        assert!(!scrubbed.contains("s3cr3t"));
        assert_eq!(scrubbed.matches("[REDACTED]").count(), 5);
    }

    // ── Coverage gap tests ─────────────────────────────────

    #[test]
    fn empty_input_no_panic() {
        let result = scrub_secrets("");
        assert_eq!(&*result, "");
    }

    #[test]
    fn token_boundary_emoji() {
        // Token with emoji boundary should still be redacted.
        let input = "key: sk-abc123def456 \u{1F600}";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("sk-abc123def456"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn pem_with_end_marker_scrubbed() {
        // PEM block with both BEGIN and END markers must be fully redacted.
        let input = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg\n-----END PRIVATE KEY-----";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("MIIEvQIBADANBg"));
        assert!(scrubbed.contains("[REDACTED-PEM]"));
    }

    #[test]
    fn mixed_markers_all_redacted() {
        let input = r#"Authorization: Bearer eyJtest api_key=secret123 "token":"hidden""#;
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("eyJtest"));
        assert!(!scrubbed.contains("secret123"));
        assert!(!scrubbed.contains("hidden"));
    }

    #[test]
    fn slack_workflow_tokens_are_redacted() {
        let input = "token=xoxe-secret-workflow-token";
        let scrubbed = scrub_secrets(input);
        assert!(!scrubbed.contains("xoxe-secret-workflow-token"));
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn zero_copy_no_match() {
        // Input with no secrets should be returned as borrowed (zero-copy).
        let input = "Hello, this is a normal message with no secrets.";
        let scrubbed = scrub_secrets(input);
        assert!(
            matches!(scrubbed, std::borrow::Cow::Borrowed(_)),
            "no-match input should be zero-copy Cow::Borrowed"
        );
    }

    #[test]
    fn utf8_multibyte_safe() {
        // Ensure scrubbing doesn't corrupt multi-byte UTF-8 boundaries.
        let input = "token: sk-\u{1F4A9}abc123\u{1F600} end";
        let scrubbed = scrub_secrets(input);
        assert!(std::str::from_utf8(scrubbed.as_bytes()).is_ok());
    }

    mod proptest_cases {
        use proptest::prelude::*;
        use proptest::sample;

        use super::*;

        proptest! {
            #[test]
            fn scrub_is_idempotent(input in "\\PC{0,500}") {
                let once = scrub_secrets(&input);
                let twice = scrub_secrets(&once);
                prop_assert_eq!(
                    &*once, &*twice,
                    "scrub must be idempotent"
                );
            }

            #[test]
            fn no_known_prefix_survives(
                prefix in sample::select(vec![
                    "sk-", "ghp_", "AKIA", "ASIA", "xoxb-", "glpat-",
                ]),
                suffix in "[a-zA-Z0-9]{10,30}"
            ) {
                let input = format!("{prefix}{suffix}");
                let scrubbed = scrub_secrets(&input);
                prop_assert!(
                    !scrubbed.contains(&suffix),
                    "Known prefix token must be redacted: {input:?}"
                );
            }

            #[test]
            fn sanitize_api_error_max_200(input in "\\PC{0,1000}") {
                let sanitized = super::super::sanitize_api_error(&input);
                // MAX_API_ERROR_CHARS is 200, output may have "..." suffix.
                prop_assert!(
                    sanitized.chars().count() <= 203,
                    "sanitize_api_error output too long: {} chars",
                    sanitized.chars().count()
                );
            }
        }
    }
}
