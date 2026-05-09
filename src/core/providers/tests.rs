//! Tests for the provider factory and top-level provider module.

use super::*;

#[test]
fn factory_openrouter() {
    assert!(create_provider("openrouter", Some("sk-test")).is_ok());
    assert!(create_provider("openrouter", None).is_ok());
}

#[test]
fn factory_anthropic() {
    assert!(create_provider("anthropic", Some("sk-test")).is_ok());
}

#[test]
fn factory_openai() {
    assert!(create_provider("openai", Some("sk-test")).is_ok());
}

#[test]
fn factory_openai_codex_alias() {
    assert!(create_provider("openai-codex", Some("sk-test")).is_ok());
}

#[test]
fn factory_openai_codex_disables_native_tool_capabilities() {
    let provider = create_provider("openai-codex", Some("sk-test")).expect("provider");
    assert!(!provider.supports_tools());
    assert!(provider.supports_streaming());
}

#[test]
fn factory_codex_cli_remains_text_only_backend() {
    assert!(create_provider("codex-cli", Some("sk-test")).is_err());

    let policy = crate::security::SecurityPolicy::default();
    let provider =
        factory::create_provider_with_security("codex-cli", Some("sk-test"), Some(&policy))
            .expect("provider");
    assert!(!provider.supports_tools());
    assert!(!provider.supports_streaming());
}

#[test]
fn factory_supported_provider_aliases() {
    for alias in [
        "google-gemini",
        "grok",
        "kimi",
        "z.ai",
        "zhipu",
        "baidu",
        "vercel-ai",
        "cloudflare-ai",
        "together-ai",
        "fireworks-ai",
        "github-copilot",
        "aws-bedrock",
        "opencode-zen",
    ] {
        assert!(create_provider(alias, Some("test-key")).is_ok(), "{alias}");
    }
}

#[test]
fn factory_ollama() {
    assert!(create_provider("ollama", None).is_ok());
    // Ollama ignores the api_key parameter since it's a local service
    assert!(create_provider("ollama", Some("dummy")).is_ok());
    assert!(create_provider("ollama", Some("any-value-here")).is_ok());
}

#[test]
fn factory_gemini() {
    assert!(create_provider("gemini", Some("test-key")).is_ok());
    // Should also work without key (will try CLI auth)
    assert!(create_provider("gemini", None).is_ok());
}

#[test]
fn factory_gemini_vertex() {
    assert!(create_provider("gemini-vertex:test-project/global", Some("test-key")).is_ok());
}

// ── OpenAI-compatible providers ──────────────────────────

#[test]
fn factory_venice() {
    assert!(create_provider("venice", Some("vn-key")).is_ok());
}

#[test]
fn factory_vercel() {
    assert!(create_provider("vercel", Some("key")).is_ok());
}

#[test]
fn factory_cloudflare() {
    assert!(create_provider("cloudflare", Some("key")).is_ok());
}

#[test]
fn factory_moonshot() {
    assert!(create_provider("moonshot", Some("key")).is_ok());
}

#[test]
fn factory_synthetic() {
    assert!(create_provider("synthetic", Some("key")).is_ok());
}

#[test]
fn factory_opencode() {
    assert!(create_provider("opencode", Some("key")).is_ok());
}

#[test]
fn factory_zai() {
    assert!(create_provider("zai", Some("key")).is_ok());
}

#[test]
fn factory_glm() {
    assert!(create_provider("glm", Some("key")).is_ok());
}

#[test]
fn factory_minimax() {
    assert!(create_provider("minimax", Some("key")).is_ok());
}

#[test]
fn factory_minimax_disables_vision_capability() {
    let provider = create_provider("minimax", Some("key")).expect("provider");
    assert!(provider.supports_tools());
    assert!(provider.supports_streaming());
    assert!(!provider.supports_vision());
}

#[test]
fn factory_bedrock() {
    assert!(create_provider("bedrock", Some("key")).is_ok());
}

#[test]
fn factory_qianfan() {
    assert!(create_provider("qianfan", Some("key")).is_ok());
}

// ── Extended ecosystem ───────────────────────────────────

#[test]
fn factory_groq() {
    assert!(create_provider("groq", Some("key")).is_ok());
}

#[test]
fn factory_mistral() {
    assert!(create_provider("mistral", Some("key")).is_ok());
}

#[test]
fn factory_xai() {
    assert!(create_provider("xai", Some("key")).is_ok());
}

#[test]
fn factory_deepseek() {
    assert!(create_provider("deepseek", Some("key")).is_ok());
}

#[test]
fn factory_together() {
    assert!(create_provider("together", Some("key")).is_ok());
}

#[test]
fn factory_fireworks() {
    assert!(create_provider("fireworks", Some("key")).is_ok());
}

#[test]
fn factory_perplexity() {
    assert!(create_provider("perplexity", Some("key")).is_ok());
}

#[test]
fn factory_cohere() {
    assert!(create_provider("cohere", Some("key")).is_ok());
}

#[test]
fn factory_copilot() {
    assert!(create_provider("copilot", Some("key")).is_ok());
}

// ── Custom / BYOP provider ─────────────────────────────

#[test]
fn factory_custom_url() {
    let p = create_provider("custom:https://my-llm.example.com", Some("key"));
    assert!(p.is_ok());
}

#[test]
fn factory_custom_localhost() {
    let p = create_provider("custom:http://localhost:1234", Some("key"));
    assert!(p.is_ok());
}

#[test]
fn factory_custom_no_key() {
    let p = create_provider("custom:https://my-llm.example.com", None);
    assert!(p.is_ok());
}

#[test]
fn factory_custom_empty_url_errors() {
    match create_provider("custom:", None) {
        Err(e) => assert!(
            e.to_string().contains("requires a URL"),
            "Expected 'requires a URL', got: {e}"
        ),
        Ok(_) => panic!("Expected error for empty custom URL"),
    }
}

// ── Anthropic-compatible custom endpoints ─────────────────

#[test]
fn factory_anthropic_custom_url() {
    let p = create_provider("anthropic-custom:https://api.example.com", Some("key"));
    assert!(p.is_ok());
}

#[test]
fn factory_anthropic_custom_trailing_slash() {
    let p = create_provider("anthropic-custom:https://api.example.com/", Some("key"));
    assert!(p.is_ok());
}

#[test]
fn factory_anthropic_custom_no_key() {
    let p = create_provider("anthropic-custom:https://api.example.com", None);
    assert!(p.is_ok());
}

#[test]
fn factory_anthropic_custom_empty_url_errors() {
    match create_provider("anthropic-custom:", None) {
        Err(e) => assert!(
            e.to_string().contains("requires a URL"),
            "Expected 'requires a URL', got: {e}"
        ),
        Ok(_) => panic!("Expected error for empty anthropic-custom URL"),
    }
}

// ── Error cases ──────────────────────────────────────────

#[test]
fn factory_unknown_provider_errors() {
    let p = create_provider("nonexistent", None);
    assert!(p.is_err());
    let msg = p.err().unwrap().to_string();
    assert!(msg.contains("Unknown provider"));
    assert!(msg.contains("nonexistent"));
}

#[test]
fn factory_empty_name_errors() {
    assert!(create_provider("", None).is_err());
}

#[test]
fn resilient_provider_ignores_duplicate_and_invalid_fallbacks() {
    let reliability = crate::config::ReliabilityConfig {
        provider_retries: 1,
        provider_backoff_ms: 100,
        fallback_providers: vec![
            "openrouter".into(),
            "nonexistent-provider".into(),
            "openai".into(),
            "openai".into(),
        ],
        channel_initial_backoff_secs: 2,
        channel_max_backoff_secs: 60,
        scheduler_poll_secs: 15,
        scheduler_retries: 2,
        scheduler_failure_budget: 0,
        scheduler_breaker_cooldown_secs: 0,
        scheduler_active_hours_start_utc: None,
        scheduler_active_hours_end_utc: None,
    };

    let provider = create_resilient_provider("openrouter", Some("sk-test"), &reliability);
    assert!(provider.is_ok());
}

#[test]
fn resilient_provider_errors_for_invalid_primary() {
    let reliability = crate::config::ReliabilityConfig::default();
    let provider = create_resilient_provider("totally-invalid", Some("sk-test"), &reliability);
    assert!(provider.is_err());
}

#[test]
fn resilient_provider_with_resolver_uses_per_provider_credentials() {
    let reliability = crate::config::ReliabilityConfig {
        fallback_providers: vec!["openai".into()],
        ..crate::config::ReliabilityConfig::default()
    };

    let provider =
        create_resilient_provider_with_resolver("openrouter", &reliability, |name| match name {
            "openrouter" => Some("sk-openrouter-key".to_string()),
            "openai" => Some("sk-openai-key".to_string()),
            _ => None,
        });
    assert!(provider.is_ok());
}

#[test]
fn factory_all_providers_create_successfully() {
    let providers = [
        "openrouter",
        "anthropic",
        "openai",
        "ollama",
        "gemini",
        "venice",
        "vercel",
        "cloudflare",
        "moonshot",
        "synthetic",
        "opencode",
        "zai",
        "glm",
        "minimax",
        "bedrock",
        "qianfan",
        "groq",
        "mistral",
        "xai",
        "deepseek",
        "together",
        "fireworks",
        "perplexity",
        "cohere",
        "copilot",
    ];
    for name in providers {
        assert!(
            create_provider(name, Some("test-key")).is_ok(),
            "Provider '{name}' should create successfully"
        );
    }
}

// ── API error sanitization ───────────────────────────────

#[test]
fn sanitize_scrubs_sk_prefix() {
    let input = "request failed: sk-1234567890abcdef";
    let out = sanitize_api_error(input);
    assert!(!out.contains("sk-1234567890abcdef"));
    assert!(out.contains("[REDACTED]"));
}

#[test]
fn sanitize_scrubs_multiple_prefixes() {
    let input = "keys sk-abcdef xoxb-12345 xoxp-67890";
    let out = sanitize_api_error(input);
    assert!(!out.contains("sk-abcdef"));
    assert!(!out.contains("xoxb-12345"));
    assert!(!out.contains("xoxp-67890"));
}

#[test]
fn sanitize_scrubs_additional_token_prefixes() {
    let input =
        "tokens ghp_abc123 github_pat_foo glpat-xyz hf_secret xoxs-999 ya29.token AIzaSySecret";
    let out = sanitize_api_error(input);
    assert!(!out.contains("ghp_abc123"));
    assert!(!out.contains("github_pat_foo"));
    assert!(!out.contains("glpat-xyz"));
    assert!(!out.contains("hf_secret"));
    assert!(!out.contains("xoxs-999"));
    assert!(!out.contains("ya29.token"));
    assert!(!out.contains("AIzaSySecret"));
    assert!(out.contains("[REDACTED]"));
}

#[test]
fn sanitize_short_prefix_then_real_key() {
    let input = "error with sk- prefix and key sk-1234567890";
    let result = sanitize_api_error(input);
    assert!(!result.contains("sk-1234567890"));
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn sanitize_sk_proj_comment_then_real_key() {
    let input = "note: sk- then sk-proj-abc123def456";
    let result = sanitize_api_error(input);
    assert!(!result.contains("sk-proj-abc123def456"));
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn sanitize_keeps_bare_prefix() {
    let input = "only prefix sk- present";
    let result = sanitize_api_error(input);
    assert!(result.contains("sk-"));
}

#[test]
fn sanitize_handles_json_wrapped_key() {
    let input = r#"{"error":"invalid key sk-abc123xyz"}"#;
    let result = sanitize_api_error(input);
    assert!(!result.contains("sk-abc123xyz"));
}

#[test]
fn sanitize_handles_delimiter_boundaries() {
    let input = "bad token xoxb-abc123}; next";
    let result = sanitize_api_error(input);
    assert!(!result.contains("xoxb-abc123"));
    assert!(result.contains("};"));
}

#[test]
fn sanitize_scrubs_bearer_authorization_headers() {
    let input = "upstream said Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
    let result = sanitize_api_error(input);
    assert!(!result.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
    assert!(result.contains("[REDACTED]"));
}

#[test]
fn sanitize_scrubs_query_and_json_token_fields() {
    let input = "url=/callback?access_token=abc123XYZ&state=ok body={\"api_key\":\"my-secret-key\",\"refresh_token\":\"rrr123\"}";
    let result = sanitize_api_error(input);
    assert!(!result.contains("access_token=abc123XYZ"));
    assert!(!result.contains("\"api_key\":\"my-secret-key\""));
    assert!(!result.contains("\"refresh_token\":\"rrr123\""));
    assert!(result.contains("state=ok"));
}

#[test]
fn sanitize_keeps_non_secret_key_value_pairs() {
    let input = "error_code=429 retry_after=30 reason=rate_limit";
    let result = sanitize_api_error(input);
    assert_eq!(result, input);
}

#[test]
fn sanitize_truncates_long_error() {
    let long = "a".repeat(400);
    let result = sanitize_api_error(&long);
    assert!(result.len() <= 203);
    assert!(result.ends_with("..."));
}

#[test]
fn sanitize_truncates_after_scrub() {
    let input = format!("{} sk-abcdef123456 {}", "a".repeat(190), "b".repeat(190));
    let result = sanitize_api_error(&input);
    assert!(!result.contains("sk-abcdef123456"));
    assert!(result.len() <= 203);
}

#[test]
fn sanitize_preserves_unicode_boundaries() {
    let input = format!("{} sk-abcdef123", "こんにちは".repeat(80));
    let result = sanitize_api_error(&input);
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    assert!(!result.contains("sk-abcdef123"));
}

#[test]
fn sanitize_no_secret_no_change() {
    let input = "simple upstream timeout";
    let result = sanitize_api_error(input);
    assert_eq!(result, input);
}
