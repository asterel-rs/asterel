//! Applies `ASTEREL_*` environment variable overrides on top of
//! the TOML-loaded config for core, gateway, runtime, and media settings.

use std::path::PathBuf;

use super::Config;
use crate::config::{RuntimeKind, SandboxSelectorMode};

impl Config {
    /// Apply `ASTEREL_*` environment variable overrides to this config.
    pub fn apply_env_overrides(&mut self) {
        self.apply_core_env_overrides();
        self.apply_gateway_env_overrides();
        self.apply_runtime_env_overrides();
        self.apply_media_env_overrides();
        self.apply_channels_env_overrides();
    }

    fn apply_core_env_overrides(&mut self) {
        if let Ok(key) = std::env::var("ASTEREL_API_KEY")
            && !key.is_empty()
        {
            self.api_key = Some(key);
        }

        if let Ok(provider) = std::env::var("ASTEREL_PROVIDER")
            && !provider.is_empty()
        {
            self.default_provider = Some(provider);
        }

        if let Ok(model) = std::env::var("ASTEREL_MODEL")
            && !model.is_empty()
        {
            self.default_model = Some(model);
        }

        if let Ok(workspace) = std::env::var("ASTEREL_WORKSPACE")
            && !workspace.is_empty()
        {
            let path = PathBuf::from(&workspace);
            match path.canonicalize() {
                Ok(canonical) if canonical.is_dir() => {
                    self.workspace_dir = canonical;
                }
                Ok(canonical) => {
                    tracing::warn!(
                        path = %canonical.display(),
                        "ASTEREL_WORKSPACE exists but is not \
                         a directory; ignoring"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        raw_path = %workspace,
                        %error,
                        "ASTEREL_WORKSPACE path could not be \
                         resolved; ignoring"
                    );
                }
            }
        }

        if let Ok(temp_str) = std::env::var("ASTEREL_TEMPERATURE")
            && let Ok(temp) = temp_str.parse::<f64>()
            && (0.0..=2.0).contains(&temp)
        {
            self.default_temperature = temp;
        }
    }

    fn apply_gateway_env_overrides(&mut self) {
        if let Ok(port_str) = std::env::var("ASTEREL_GATEWAY_PORT")
            && let Ok(port) = port_str.parse::<u16>()
        {
            self.gateway.port = port;
        }

        if let Ok(host) = std::env::var("ASTEREL_GATEWAY_HOST")
            && !host.is_empty()
        {
            self.gateway.host = host;
        }

        if let Ok(max_body_size_str) = std::env::var("ASTEREL_GATEWAY_MAX_BODY_SIZE_BYTES")
            && let Ok(max_body_size_bytes) = max_body_size_str.parse::<usize>()
            && max_body_size_bytes > 0
        {
            self.gateway.max_body_size_bytes = max_body_size_bytes;
        }

        if let Ok(allow_public_bind) = std::env::var("ASTEREL_GATEWAY_ALLOW_PUBLIC_BIND") {
            self.gateway.allow_public_bind = matches!(
                allow_public_bind.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
    }

    fn apply_runtime_env_overrides(&mut self) {
        if let Ok(runtime_kind) = std::env::var("ASTEREL_RUNTIME_KIND") {
            self.runtime.kind = match runtime_kind.trim().to_ascii_lowercase().as_str() {
                "auto" => RuntimeKind::Auto,
                "native" => RuntimeKind::Native,
                "docker" => RuntimeKind::Docker,
                "wasm" => RuntimeKind::Wasm,
                _ => self.runtime.kind,
            };
        }

        if let Ok(selector) = std::env::var("ASTEREL_RUNTIME_SANDBOX_SELECTOR") {
            self.runtime.sandbox_selector = match selector.trim().to_ascii_lowercase().as_str() {
                "fixed" => SandboxSelectorMode::Fixed,
                "auto" => SandboxSelectorMode::Auto,
                _ => self.runtime.sandbox_selector,
            };
        }

        if let Ok(enable_docker_runtime) = std::env::var("ASTEREL_ENABLE_DOCKER_RUNTIME") {
            self.runtime.enable_docker_runtime = matches!(
                enable_docker_runtime.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
    }

    fn apply_media_env_overrides(&mut self) {
        if let Some(enabled) = env_bool("ASTEREL_MEDIA_STT_ENABLED") {
            self.media.stt.enabled = Some(enabled);
        }
        if let Some(provider) = env_non_empty("ASTEREL_MEDIA_STT_PROVIDER") {
            self.media.stt.provider = Some(provider);
        }
        if let Some(model) = env_non_empty("ASTEREL_MEDIA_STT_MODEL") {
            self.media.stt.model = Some(model);
        }
        if let Some(endpoint) = env_non_empty("ASTEREL_MEDIA_STT_ENDPOINT") {
            self.media.stt.endpoint = Some(endpoint);
        }
        if let Some(language) = env_non_empty("ASTEREL_MEDIA_STT_LANGUAGE") {
            self.media.stt.language = Some(language);
        }
        if let Some(prompt) = env_non_empty("ASTEREL_MEDIA_STT_PROMPT") {
            self.media.stt.prompt = Some(prompt);
        }
        if let Some(api_key) = env_non_empty("ASTEREL_MEDIA_STT_API_KEY") {
            self.media.stt.api_key = Some(api_key);
        }

        if let Some(enabled) = env_bool("ASTEREL_MEDIA_TTS_ENABLED") {
            self.media.tts.enabled = Some(enabled);
        }
        if let Some(provider) = env_non_empty("ASTEREL_MEDIA_TTS_PROVIDER") {
            self.media.tts.provider = Some(provider);
        }
        if let Some(model) = env_non_empty("ASTEREL_MEDIA_TTS_MODEL") {
            self.media.tts.model = Some(model);
        }
        if let Some(voice) = env_non_empty("ASTEREL_MEDIA_TTS_VOICE") {
            self.media.tts.voice = Some(voice);
        }
        if let Some(response_format) = env_non_empty("ASTEREL_MEDIA_TTS_RESPONSE_FORMAT") {
            self.media.tts.response_format = Some(response_format);
        }
        if let Some(endpoint) = env_non_empty("ASTEREL_MEDIA_TTS_ENDPOINT") {
            self.media.tts.endpoint = Some(endpoint);
        }
        if let Some(api_key) = env_non_empty("ASTEREL_MEDIA_TTS_API_KEY") {
            self.media.tts.api_key = Some(api_key);
        }
    }

    fn apply_channels_env_overrides(&mut self) {
        let Some(twitter) = self.channels_config.twitter.as_mut() else {
            return;
        };

        if let Some(client_id) = env_non_empty("ASTEREL_CHANNEL_TWITTER_CLIENT_ID") {
            twitter.client_id = client_id;
        }
        if let Some(client_secret) = env_non_empty("ASTEREL_CHANNEL_TWITTER_CLIENT_SECRET") {
            twitter.client_secret = client_secret;
        }
        if let Some(access_token) = env_non_empty("ASTEREL_CHANNEL_TWITTER_ACCESS_TOKEN") {
            twitter.access_token = access_token;
        }
        if let Some(refresh_token) = env_non_empty("ASTEREL_CHANNEL_TWITTER_REFRESH_TOKEN") {
            twitter.refresh_token = refresh_token;
        }
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TwitterConfig;
    use crate::config::schema::core::test_env::{ENV_LOCK, EnvVarGuard};
    use crate::contracts::ids::UserId;

    #[test]
    fn strict_env_var_resolves_primary_key() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _primary = EnvVarGuard::set("ASTEREL_PROVIDER", "anthropic");
        let _legacy = EnvVarGuard::set("PROVIDER", "openai");

        let mut config = Config {
            default_provider: Some("openrouter".to_string()),
            ..Config::default()
        };
        config.apply_env_overrides();

        assert_eq!(config.default_provider.as_deref(), Some("anthropic"));
    }

    #[test]
    fn strict_env_var_ignores_legacy_key_when_primary_missing() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _primary = EnvVarGuard::unset("ASTEREL_PROVIDER");
        let _legacy = EnvVarGuard::set("PROVIDER", "openrouter");

        let mut config = Config {
            default_provider: Some("anthropic".to_string()),
            ..Config::default()
        };
        config.apply_env_overrides();

        assert_eq!(config.default_provider.as_deref(), Some("anthropic"));
    }

    #[test]
    fn apply_env_overrides_does_not_use_legacy_api_key_env_key() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _primary = EnvVarGuard::unset("ASTEREL_API_KEY");
        let _legacy = EnvVarGuard::set("API_KEY", "sk-legacy");

        let mut config = Config {
            api_key: Some("sk-primary".to_string()),
            ..Config::default()
        };
        config.apply_env_overrides();

        assert_eq!(config.api_key.as_deref(), Some("sk-primary"));
    }

    #[test]
    fn apply_env_overrides_does_not_use_legacy_gateway_port_or_host_keys() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _primary_port = EnvVarGuard::unset("ASTEREL_GATEWAY_PORT");
        let _primary_host = EnvVarGuard::unset("ASTEREL_GATEWAY_HOST");
        let _legacy_port = EnvVarGuard::set("PORT", "9999");
        let _legacy_host = EnvVarGuard::set("HOST", "0.0.0.0");

        let mut config = Config::default();
        config.gateway.port = 3000;
        config.gateway.host = "127.0.0.1".to_string();

        config.apply_env_overrides();

        assert_eq!(config.gateway.port, 3000);
        assert_eq!(config.gateway.host, "127.0.0.1");
    }

    #[test]
    fn apply_env_overrides_parses_gateway_max_body_size_bytes() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _max_body = EnvVarGuard::set("ASTEREL_GATEWAY_MAX_BODY_SIZE_BYTES", "131072");

        let mut config = Config::default();
        config.gateway.max_body_size_bytes = 65_536;

        config.apply_env_overrides();

        assert_eq!(config.gateway.max_body_size_bytes, 131_072);
    }

    #[test]
    fn apply_env_overrides_parses_gateway_allow_public_bind() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _allow_public_bind = EnvVarGuard::set("ASTEREL_GATEWAY_ALLOW_PUBLIC_BIND", "true");

        let mut config = Config::default();
        config.gateway.allow_public_bind = false;

        config.apply_env_overrides();

        assert!(config.gateway.allow_public_bind);
    }

    #[test]
    fn apply_env_overrides_updates_twitter_oauth_secrets_from_env() {
        let _lock = ENV_LOCK.lock().expect("lock env");
        let _client_id =
            EnvVarGuard::set("ASTEREL_CHANNEL_TWITTER_CLIENT_ID", "env-twitter-client-id");
        let _client_secret = EnvVarGuard::set(
            "ASTEREL_CHANNEL_TWITTER_CLIENT_SECRET",
            "env-twitter-client-secret",
        );
        let _access_token = EnvVarGuard::set(
            "ASTEREL_CHANNEL_TWITTER_ACCESS_TOKEN",
            "env-twitter-access-token",
        );
        let _refresh_token = EnvVarGuard::set(
            "ASTEREL_CHANNEL_TWITTER_REFRESH_TOKEN",
            "env-twitter-refresh-token",
        );

        let mut config = Config::default();
        config.channels_config.twitter = Some(TwitterConfig {
            client_id: "file-twitter-client-id".to_string(),
            client_secret: "file-twitter-client-secret".to_string(),
            access_token: "file-twitter-access-token".to_string(),
            refresh_token: "file-twitter-refresh-token".to_string(),
            user_id: UserId::new("1234567890"),
            username: "bot-user".to_string(),
            allowed_users: Vec::new(),
            mention_poll_interval_secs: 180,
            dm_poll_interval_secs: 300,
            security: Default::default(),
        });

        config.apply_env_overrides();

        let twitter = config
            .channels_config
            .twitter
            .as_ref()
            .expect("twitter config should remain present");
        assert_eq!(twitter.client_id, "env-twitter-client-id");
        assert_eq!(twitter.client_secret, "env-twitter-client-secret");
        assert_eq!(twitter.access_token, "env-twitter-access-token");
        assert_eq!(twitter.refresh_token, "env-twitter-refresh-token");
    }
}
