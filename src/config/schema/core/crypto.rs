//! Transparent encrypt/decrypt of secret fields in the config file.
//! Operates in-place on `Config` using the `SecretStore` backend.

use std::path::Path;

use anyhow::Result;

use super::Config;
use crate::contracts::security::SecretStore;

#[cfg(test)]
pub(super) const CONFIG_SECRET_FIELD_PATHS: &[&str] = &[
    "api_key",
    "composio.api_key",
    "model_list[].api_key",
    "channels.telegram.bot_token",
    "channels.discord.bot_token",
    "channels.slack.bot_token",
    "channels.slack.app_token",
    "channels.webhook.secret",
    "channels.matrix.access_token",
    "channels.whatsapp.access_token",
    "channels.whatsapp.verify_token",
    "channels.whatsapp.app_secret",
    "channels.irc.server_password",
    "channels.irc.nickserv_password",
    "channels.irc.sasl_password",
    "channels.twitter.client_secret",
    "channels.twitter.access_token",
    "channels.twitter.refresh_token",
    "channels.email.password",
    "media.stt.api_key",
    "media.tts.api_key",
    "gateway.paired_tokens[]",
    "tunnel.cloudflare.token",
    "tunnel.ngrok.auth_token",
];

fn decrypt_secret_string(
    value: &mut String,
    store: &SecretStore,
    encrypt_enabled: bool,
) -> Result<bool> {
    let current = value.trim();
    if current.is_empty() {
        return Ok(false);
    }

    let needs_encrypt_persist = encrypt_enabled && !SecretStore::is_encrypted(current);
    let decrypted = store.decrypt(current)?;
    *value = decrypted;

    Ok(needs_encrypt_persist)
}

fn decrypt_opt_secret(
    value: &mut Option<String>,
    store: &SecretStore,
    encrypt_enabled: bool,
) -> Result<bool> {
    let Some(current) = value.as_deref() else {
        return Ok(false);
    };

    let trimmed = current.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }

    let needs_encrypt_persist = encrypt_enabled && !SecretStore::is_encrypted(trimmed);
    let decrypted = store.decrypt(trimmed)?;
    *value = Some(decrypted);

    Ok(needs_encrypt_persist)
}

fn decrypt_secret_vec(
    values: &mut [String],
    store: &SecretStore,
    encrypt_enabled: bool,
) -> Result<bool> {
    let mut needs_persist = false;
    for value in values {
        needs_persist |= decrypt_secret_string(value, store, encrypt_enabled)?;
    }
    Ok(needs_persist)
}

fn encrypt_secret_string(value: &mut String, store: &SecretStore) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || SecretStore::is_encrypted(trimmed) {
        if trimmed != value {
            *value = trimmed.to_string();
        }
        return Ok(());
    }

    *value = store.encrypt(trimmed)?;
    Ok(())
}

fn encrypt_opt_secret(value: &mut Option<String>, store: &SecretStore) -> Result<()> {
    let Some(current) = value.as_deref() else {
        return Ok(());
    };

    let trimmed = current.trim();
    if trimmed.is_empty() || SecretStore::is_encrypted(trimmed) {
        if trimmed != current {
            *value = Some(trimmed.to_string());
        }
        return Ok(());
    }

    *value = Some(store.encrypt(trimmed)?);
    Ok(())
}

fn encrypt_secret_vec(values: &mut [String], store: &SecretStore) -> Result<()> {
    for value in values {
        encrypt_secret_string(value, store)?;
    }
    Ok(())
}

impl Config {
    fn secret_store_root(&self) -> &Path {
        self.config_path.parent().unwrap_or_else(|| Path::new("."))
    }

    fn secret_store(&self) -> SecretStore {
        SecretStore::new(self.secret_store_root(), self.secrets.encrypt)
    }

    /// Decrypt all secret fields in-place and return whether any
    /// plaintext secrets were found that need re-encryption on disk.
    ///
    /// # Errors
    ///
    /// Returns an error if decryption of any secret field fails.
    pub(super) fn decrypt_config_secrets_in_place(&mut self) -> Result<bool> {
        let store = self.secret_store();
        let mut needs_persist = false;

        needs_persist |= decrypt_opt_secret(&mut self.api_key, &store, self.secrets.encrypt)?;
        needs_persist |=
            decrypt_opt_secret(&mut self.composio.api_key, &store, self.secrets.encrypt)?;
        for model in &mut self.model_list {
            needs_persist |= decrypt_opt_secret(&mut model.api_key, &store, self.secrets.encrypt)?;
        }

        if let Some(telegram) = self.channels_config.telegram.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut telegram.bot_token, &store, self.secrets.encrypt)?;
        }

        if let Some(discord) = self.channels_config.discord.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut discord.bot_token, &store, self.secrets.encrypt)?;
        }

        if let Some(slack) = self.channels_config.slack.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut slack.bot_token, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_opt_secret(&mut slack.app_token, &store, self.secrets.encrypt)?;
        }

        if let Some(webhook) = self.channels_config.webhook.as_mut() {
            needs_persist |= decrypt_opt_secret(&mut webhook.secret, &store, self.secrets.encrypt)?;
        }

        if let Some(matrix) = self.channels_config.matrix.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut matrix.access_token, &store, self.secrets.encrypt)?;
        }

        if let Some(whatsapp) = self.channels_config.whatsapp.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut whatsapp.access_token, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_secret_string(&mut whatsapp.verify_token, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_opt_secret(&mut whatsapp.app_secret, &store, self.secrets.encrypt)?;
        }

        if let Some(irc) = self.channels_config.irc.as_mut() {
            needs_persist |=
                decrypt_opt_secret(&mut irc.server_password, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_opt_secret(&mut irc.nickserv_password, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_opt_secret(&mut irc.sasl_password, &store, self.secrets.encrypt)?;
        }

        if let Some(twitter) = self.channels_config.twitter.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut twitter.client_secret, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_secret_string(&mut twitter.access_token, &store, self.secrets.encrypt)?;
            needs_persist |=
                decrypt_secret_string(&mut twitter.refresh_token, &store, self.secrets.encrypt)?;
        }

        if let Some(email) = self.channels_config.email.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut email.password, &store, self.secrets.encrypt)?;
        }

        needs_persist |=
            decrypt_opt_secret(&mut self.media.stt.api_key, &store, self.secrets.encrypt)?;
        needs_persist |=
            decrypt_opt_secret(&mut self.media.tts.api_key, &store, self.secrets.encrypt)?;
        needs_persist |= decrypt_secret_vec(
            &mut self.gateway.paired_tokens,
            &store,
            self.secrets.encrypt,
        )?;

        if let Some(cloudflare) = self.tunnel.cloudflare.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut cloudflare.token, &store, self.secrets.encrypt)?;
        }

        if let Some(ngrok) = self.tunnel.ngrok.as_mut() {
            needs_persist |=
                decrypt_secret_string(&mut ngrok.auth_token, &store, self.secrets.encrypt)?;
        }

        Ok(needs_persist)
    }

    /// Encrypt all plaintext secret fields in-place. No-op if
    /// `secrets.encrypt` is false.
    ///
    /// # Errors
    ///
    /// Returns an error if encryption of any secret field fails.
    pub(super) fn encrypt_config_secrets_in_place(&mut self) -> Result<()> {
        if !self.secrets.encrypt {
            return Ok(());
        }

        let store = self.secret_store();

        encrypt_opt_secret(&mut self.api_key, &store)?;
        encrypt_opt_secret(&mut self.composio.api_key, &store)?;
        for model in &mut self.model_list {
            encrypt_opt_secret(&mut model.api_key, &store)?;
        }

        if let Some(telegram) = self.channels_config.telegram.as_mut() {
            encrypt_secret_string(&mut telegram.bot_token, &store)?;
        }

        if let Some(discord) = self.channels_config.discord.as_mut() {
            encrypt_secret_string(&mut discord.bot_token, &store)?;
        }

        if let Some(slack) = self.channels_config.slack.as_mut() {
            encrypt_secret_string(&mut slack.bot_token, &store)?;
            encrypt_opt_secret(&mut slack.app_token, &store)?;
        }

        if let Some(webhook) = self.channels_config.webhook.as_mut() {
            encrypt_opt_secret(&mut webhook.secret, &store)?;
        }

        if let Some(matrix) = self.channels_config.matrix.as_mut() {
            encrypt_secret_string(&mut matrix.access_token, &store)?;
        }

        if let Some(whatsapp) = self.channels_config.whatsapp.as_mut() {
            encrypt_secret_string(&mut whatsapp.access_token, &store)?;
            encrypt_secret_string(&mut whatsapp.verify_token, &store)?;
            encrypt_opt_secret(&mut whatsapp.app_secret, &store)?;
        }

        if let Some(irc) = self.channels_config.irc.as_mut() {
            encrypt_opt_secret(&mut irc.server_password, &store)?;
            encrypt_opt_secret(&mut irc.nickserv_password, &store)?;
            encrypt_opt_secret(&mut irc.sasl_password, &store)?;
        }

        if let Some(twitter) = self.channels_config.twitter.as_mut() {
            encrypt_secret_string(&mut twitter.client_secret, &store)?;
            encrypt_secret_string(&mut twitter.access_token, &store)?;
            encrypt_secret_string(&mut twitter.refresh_token, &store)?;
        }

        if let Some(email) = self.channels_config.email.as_mut() {
            encrypt_secret_string(&mut email.password, &store)?;
        }

        encrypt_opt_secret(&mut self.media.stt.api_key, &store)?;
        encrypt_opt_secret(&mut self.media.tts.api_key, &store)?;
        encrypt_secret_vec(&mut self.gateway.paired_tokens, &store)?;

        if let Some(cloudflare) = self.tunnel.cloudflare.as_mut() {
            encrypt_secret_string(&mut cloudflare.token, &store)?;
        }

        if let Some(ngrok) = self.tunnel.ngrok.as_mut() {
            encrypt_secret_string(&mut ngrok.auth_token, &store)?;
        }

        Ok(())
    }

    /// Clone this config with secrets encrypted for safe persistence.
    ///
    /// # Errors
    ///
    /// Returns an error if secret encryption fails.
    pub(super) fn config_for_persistence(&self) -> Result<Self> {
        let mut persisted = self.clone();
        persisted.encrypt_config_secrets_in_place()?;
        Ok(persisted)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::config::{ChannelSecurityPolicy, EmailConfig, ModelListEntry, TwitterConfig};
    use crate::contracts::ids::UserId;

    #[test]
    fn config_secret_field_checklist_enumerates_current_crypto_contract() {
        assert_eq!(
            CONFIG_SECRET_FIELD_PATHS,
            &[
                "api_key",
                "composio.api_key",
                "model_list[].api_key",
                "channels.telegram.bot_token",
                "channels.discord.bot_token",
                "channels.slack.bot_token",
                "channels.slack.app_token",
                "channels.webhook.secret",
                "channels.matrix.access_token",
                "channels.whatsapp.access_token",
                "channels.whatsapp.verify_token",
                "channels.whatsapp.app_secret",
                "channels.irc.server_password",
                "channels.irc.nickserv_password",
                "channels.irc.sasl_password",
                "channels.twitter.client_secret",
                "channels.twitter.access_token",
                "channels.twitter.refresh_token",
                "channels.email.password",
                "media.stt.api_key",
                "media.tts.api_key",
                "gateway.paired_tokens[]",
                "tunnel.cloudflare.token",
                "tunnel.ngrok.auth_token",
            ]
        );
    }

    fn config_with_secret_coverage_fixture() -> (TempDir, Config) {
        let temp = TempDir::new().expect("temp config root should be created");
        let mut config = Config {
            config_path: temp.path().join("config.toml"),
            ..Config::default()
        };

        config.model_list.push(ModelListEntry {
            model_name: "secure-alias".to_string(),
            model: "openai/gpt-4o".to_string(),
            api_key: Some("model-list-secret".to_string()),
            api_base: None,
        });
        config.channels_config.twitter = Some(TwitterConfig {
            client_id: "twitter-client-id".to_string(),
            client_secret: "twitter-client-secret".to_string(),
            access_token: "twitter-access-token".to_string(),
            refresh_token: "twitter-refresh-token".to_string(),
            user_id: UserId::new("12345"),
            username: "bot".to_string(),
            allowed_users: Vec::new(),
            mention_poll_interval_secs: 180,
            dm_poll_interval_secs: 300,
            security: ChannelSecurityPolicy::default(),
        });
        config.channels_config.email = Some(EmailConfig {
            password: "email-password".to_string(),
            ..EmailConfig::default()
        });
        config.media.stt.api_key = Some("stt-secret".to_string());
        config.media.tts.api_key = Some("tts-secret".to_string());
        config.gateway.paired_tokens = vec!["pair-token-a".to_string(), "pair-token-b".to_string()];

        (temp, config)
    }

    #[test]
    fn persistence_encrypts_all_config_secret_fields() {
        let (_temp, config) = config_with_secret_coverage_fixture();

        let persisted = config
            .config_for_persistence()
            .expect("config secrets should encrypt");

        assert!(SecretStore::is_encrypted(
            persisted.model_list[0].api_key.as_deref().unwrap()
        ));
        let twitter = persisted.channels_config.twitter.as_ref().unwrap();
        assert!(SecretStore::is_encrypted(&twitter.client_secret));
        assert!(SecretStore::is_encrypted(&twitter.access_token));
        assert!(SecretStore::is_encrypted(&twitter.refresh_token));
        assert!(SecretStore::is_encrypted(
            &persisted.channels_config.email.as_ref().unwrap().password
        ));
        assert!(SecretStore::is_encrypted(
            persisted.media.stt.api_key.as_deref().unwrap()
        ));
        assert!(SecretStore::is_encrypted(
            persisted.media.tts.api_key.as_deref().unwrap()
        ));
        assert!(
            persisted
                .gateway
                .paired_tokens
                .iter()
                .all(|token| SecretStore::is_encrypted(token))
        );
    }

    #[test]
    fn encrypted_config_secret_fields_decrypt_back_to_plaintext() {
        let (_temp, config) = config_with_secret_coverage_fixture();
        let mut loaded = config
            .config_for_persistence()
            .expect("config secrets should encrypt");

        let needs_persist = loaded
            .decrypt_config_secrets_in_place()
            .expect("config secrets should decrypt");

        assert!(!needs_persist);
        assert_eq!(
            loaded.model_list[0].api_key.as_deref(),
            Some("model-list-secret")
        );
        let twitter = loaded.channels_config.twitter.as_ref().unwrap();
        assert_eq!(twitter.client_secret, "twitter-client-secret");
        assert_eq!(twitter.access_token, "twitter-access-token");
        assert_eq!(twitter.refresh_token, "twitter-refresh-token");
        assert_eq!(
            loaded.channels_config.email.as_ref().unwrap().password,
            "email-password"
        );
        assert_eq!(loaded.media.stt.api_key.as_deref(), Some("stt-secret"));
        assert_eq!(loaded.media.tts.api_key.as_deref(), Some("tts-secret"));
        assert_eq!(
            loaded.gateway.paired_tokens,
            vec!["pair-token-a".to_string(), "pair-token-b".to_string()]
        );
    }

    #[test]
    fn plaintext_config_secret_fields_request_rewrite() {
        let (_temp, mut config) = config_with_secret_coverage_fixture();

        let needs_persist = config
            .decrypt_config_secrets_in_place()
            .expect("plaintext config secrets should pass through");

        assert!(needs_persist);
        assert_eq!(
            config.model_list[0].api_key.as_deref(),
            Some("model-list-secret")
        );
        assert_eq!(config.media.stt.api_key.as_deref(), Some("stt-secret"));
        assert_eq!(
            config.gateway.paired_tokens,
            vec!["pair-token-a".to_string(), "pair-token-b".to_string()]
        );
    }
}
