use asterel::config::Config;

#[test]
fn minimal_config_deserializes_with_defaults() {
    let toml = r#"
api_key = "abc"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7

[gateway]
port = 3000
host = "127.0.0.1"
"#;

    let parsed: Config = toml::from_str(toml).expect("minimal config should deserialize");

    assert_eq!(parsed.gateway.port, 3000);
    assert_eq!(parsed.gateway.host, "127.0.0.1");
    assert_eq!(parsed.gateway.max_body_size_bytes, 65_536);
    assert!(parsed.gateway.require_pairing);
    assert!(parsed.channels_config.cli);
    assert_eq!(
        parsed.memory.backend,
        asterel::config::MemoryBackend::Postgres
    );
    assert_eq!(
        parsed.observability.backend,
        asterel::config::ObservabilityBackend::None
    );
}

#[test]
fn channel_defaults_are_preserved() {
    let toml = r#"
default_temperature = 0.7

[channels_config.discord]
bot_token = "token"
"#;

    let parsed: Config = toml::from_str(toml).expect("channel config should deserialize");
    let discord = parsed.channels_config.discord.expect("discord config");

    assert!(discord.allowed_users.is_empty());
    assert!(parsed.channels_config.telegram.is_none());
    assert!(parsed.channels_config.whatsapp.is_none());
}
