//! Provider alias resolution and built-in provider identification.
//!
//! Pure-data utilities shared by `config` (L0) and `core::providers` (L1).
//! No I/O, no catalog metadata — just canonical name mapping.

/// Known alias → canonical provider ID mapping.
const PROVIDER_ALIASES: &[(&str, &str)] = &[
    ("openai-codex", "openai"),
    ("google", "gemini"),
    ("google-gemini", "gemini"),
    ("vertex-gemini", "gemini-vertex"),
    ("kimi", "moonshot"),
    ("grok", "xai"),
    ("z.ai", "zai"),
    ("zhipu", "glm"),
    ("baidu", "qianfan"),
    ("vercel-ai", "vercel"),
    ("cloudflare-ai", "cloudflare"),
    ("together-ai", "together"),
    ("fireworks-ai", "fireworks"),
    ("github-copilot", "copilot"),
    ("aws-bedrock", "bedrock"),
    ("opencode-zen", "opencode"),
];

/// Canonical IDs of all built-in provider backends.
const BUILTIN_PROVIDER_IDS: &[&str] = &[
    "openrouter",
    "anthropic",
    "openai",
    "ollama",
    "gemini",
    "gemini-vertex",
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

/// Return the canonical provider identifier for an alias, or the
/// trimmed input when no alias matches.
#[must_use]
pub fn normalize_provider_alias(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.to_ascii_lowercase().starts_with("gemini-vertex:")
        || trimmed.to_ascii_lowercase().starts_with("vertex-gemini:")
    {
        return "gemini-vertex";
    }
    PROVIDER_ALIASES
        .iter()
        .find_map(|(alias, canonical)| trimmed.eq_ignore_ascii_case(alias).then_some(*canonical))
        .unwrap_or(trimmed)
}

/// Return whether the provider name resolves to a built-in backend.
#[must_use]
pub fn is_builtin_provider(name: &str) -> bool {
    let normalized = normalize_provider_alias(name);
    BUILTIN_PROVIDER_IDS
        .iter()
        .any(|id| id.eq_ignore_ascii_case(normalized))
}

#[cfg(test)]
pub(crate) fn provider_aliases() -> &'static [(&'static str, &'static str)] {
    PROVIDER_ALIASES
}

#[cfg(test)]
pub(crate) fn builtin_provider_ids() -> &'static [&'static str] {
    BUILTIN_PROVIDER_IDS
}
