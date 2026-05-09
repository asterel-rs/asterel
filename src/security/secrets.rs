//! Encrypted secret store -- defense-in-depth for API keys and
//! tokens.
//!
//! Uses ChaCha20-Poly1305 AEAD with a random key stored in
//! `~/.asterel/.secret_key` (mode 0600). Each encryption
//! generates a fresh 12-byte nonce prepended to the ciphertext.
//! Prevents plaintext exposure in config files, `grep`/`git log`
//! leaks, and ciphertext tampering. Disable with
//! `secrets.encrypt = false` for sovereign plaintext mode.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, ChaCha20Poly1305, Key, Nonce};
use zeroize::Zeroizing;

pub use crate::contracts::security::SecretStore;
use crate::security::kdf;

/// ChaCha20-Poly1305 nonce length in bytes.
const NONCE_LEN: usize = 12;

impl SecretStore {
    /// Create a new secret store rooted at the given directory.
    #[must_use]
    pub fn new(asterel_dir: &Path, enabled: bool) -> Self {
        Self {
            key_path: asterel_dir.join(".secret_key"),
            enabled,
        }
    }

    /// Encrypt a plaintext secret. Returns hex-encoded ciphertext prefixed with `enc2:`.
    /// Format: `enc2:<hex(nonce ‖ ciphertext ‖ tag)>` (12 + N + 16 bytes).
    /// If encryption is disabled, returns the plaintext as-is.
    ///
    /// Accepts `&[u8]` so callers holding `Zeroizing<Vec<u8>>` can encrypt
    /// without converting to `&str` first.
    ///
    /// # Errors
    /// Returns an error if key loading, encryption, or output encoding fails.
    pub fn encrypt(&self, plaintext: impl AsRef<[u8]>) -> Result<String> {
        let plaintext = plaintext.as_ref();
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        if !self.enabled {
            tracing::warn!("secret encryption disabled; storing value in plaintext");
            return Ok(String::from_utf8_lossy(plaintext).into_owned());
        }

        let key_bytes = self.load_or_create_key()?;
        let key = Key::from_slice(&key_bytes);
        let cipher = ChaCha20Poly1305::new(key);

        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|error| anyhow::anyhow!("encryption failed: {error}"))?;

        // Prepend nonce to ciphertext for storage
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);

        Ok(format!("enc2:{}", hex_encode(&blob)))
    }

    /// Decrypt a secret.
    /// - `enc2:` prefix → ChaCha20-Poly1305 (current format)
    /// - No prefix → returned as-is (plaintext config)
    ///
    /// # Caller obligations
    ///
    /// The returned `String` contains plaintext secret material. Callers that
    /// store this value long-term should wrap it in `Zeroizing<String>` (from
    /// the `zeroize` crate) to ensure the memory is zeroed on drop. Short-lived
    /// temporaries (e.g. passed directly to an API client) are acceptable
    /// without wrapping.
    ///
    /// # Errors
    /// Returns an error if encrypted payload decoding or decryption fails.
    pub fn decrypt(&self, value: &str) -> Result<String> {
        if let Some(hex_str) = value.strip_prefix("enc2:") {
            self.decrypt_chacha20(hex_str)
        } else if value.starts_with("enc3:") {
            anyhow::bail!(
                "enc3: password-encrypted value detected but no password provided; \
                 use SecretStore::decrypt_with_password() instead"
            )
        } else {
            Ok(value.to_string())
        }
    }

    /// Decrypt using ChaCha20-Poly1305 (current secure format).
    fn decrypt_chacha20(&self, hex_str: &str) -> Result<String> {
        let blob = Zeroizing::new(
            hex_decode(hex_str).context("Failed to decode encrypted secret (corrupt hex)")?,
        );
        anyhow::ensure!(
            blob.len() > NONCE_LEN,
            "Encrypted value too short (missing nonce)"
        );

        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        let key_bytes = self.load_or_create_key()?;
        let key = Key::from_slice(&key_bytes);
        let cipher = ChaCha20Poly1305::new(key);

        let plaintext_bytes = Zeroizing::new(
            cipher
                .decrypt(nonce, ciphertext)
                .map_err(|_| anyhow::anyhow!("decryption failed — wrong key or tampered data"))?,
        );

        String::from_utf8(plaintext_bytes.to_vec())
            .context("Decrypted secret is not valid UTF-8 — corrupt data")
    }

    /// Encrypt a plaintext secret using a password-derived key (Argon2id).
    ///
    /// Returns `enc3:<hex(salt || nonce || ciphertext || tag)>` where
    /// salt is 16 bytes, nonce is 12 bytes, and tag is 16 bytes.
    ///
    /// # Errors
    /// Returns an error if key derivation or encryption fails.
    pub fn encrypt_with_password(&self, plaintext: &[u8], password: &str) -> Result<String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }

        let salt = kdf::generate_salt();
        let key = kdf::derive_key_default(password.as_bytes(), &salt)?;
        let key = Key::from_slice(&key);
        let cipher = ChaCha20Poly1305::new(key);

        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|error| anyhow::anyhow!("password-based encryption failed: {error}"))?;

        let mut blob = Vec::with_capacity(kdf::SALT_LEN + NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);

        Ok(format!("enc3:{}", hex_encode(&blob)))
    }

    /// Decrypt a secret encrypted with a password-derived key (Argon2id).
    ///
    /// Expects `enc3:<hex(salt || nonce || ciphertext || tag)>` format.
    /// This is a static method -- no `SecretStore` instance or key file needed.
    ///
    /// # Errors
    /// Returns an error if the value is not `enc3:` formatted, decoding fails,
    /// the password is wrong, or the ciphertext is tampered.
    pub fn decrypt_with_password(value: &str, password: &str) -> Result<String> {
        let hex_str = value
            .strip_prefix("enc3:")
            .ok_or_else(|| anyhow::anyhow!("value is not enc3: formatted"))?;

        let blob = Zeroizing::new(
            hex_decode(hex_str).context("Failed to decode enc3 encrypted secret (corrupt hex)")?,
        );

        let min_len = kdf::SALT_LEN + NONCE_LEN + 1;
        anyhow::ensure!(
            blob.len() >= min_len,
            "enc3 encrypted value too short (expected at least {min_len} bytes, got {})",
            blob.len()
        );

        let (salt, rest) = blob.split_at(kdf::SALT_LEN);
        let (nonce_bytes, ciphertext) = rest.split_at(NONCE_LEN);

        let key = kdf::derive_key_default(password.as_bytes(), salt)?;
        let key = Key::from_slice(&key);
        let cipher = ChaCha20Poly1305::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext_bytes = Zeroizing::new(cipher.decrypt(nonce, ciphertext).map_err(|_| {
            anyhow::anyhow!("password decryption failed -- wrong password or tampered data")
        })?);

        String::from_utf8(plaintext_bytes.to_vec())
            .context("Decrypted secret is not valid UTF-8 -- corrupt data")
    }

    /// Returns `true` if the value uses an encrypted format (`enc2:` or `enc3:`).
    #[must_use]
    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with("enc2:") || value.starts_with("enc3:")
    }

    /// Load the encryption key from disk, or create one if it doesn't exist.
    ///
    /// On Unix, the key file is created atomically with mode 0o600 using
    /// `O_CREAT | O_EXCL` to prevent TOCTOU races that could expose the key.
    fn load_or_create_key(&self) -> Result<Zeroizing<Vec<u8>>> {
        match fs::read_to_string(&self.key_path) {
            Ok(hex_key) => {
                return Ok(Zeroizing::new(
                    hex_decode(hex_key.trim()).context("Secret key file is corrupt")?,
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Key file doesn't exist yet — fall through to creation below.
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context("Failed to read secret key file"));
            }
        }

        {
            let key = generate_random_key();
            if let Some(parent) = self.key_path.parent() {
                fs::create_dir_all(parent)?;
            }

            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                // Atomically create with mode 0o600 — never world-readable.
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true) // O_CREAT | O_EXCL
                    .mode(0o600)
                    .open(&self.key_path)
                    .context("Failed to create secret key file")?;
                file.write_all(hex_encode(&key).as_bytes())
                    .context("Failed to write secret key file")?;
            }

            #[cfg(not(unix))]
            {
                // Write to a temp file first so the key is never world-readable
                // at the final path (closes the ACL race window on Windows).
                let temp_path = self.key_path.with_extension("tmp");
                fs::write(&temp_path, hex_encode(&key))
                    .context("Failed to write temporary key file")?;

                #[cfg(windows)]
                {
                    // Apply ACL to the temp file BEFORE renaming to the final path.
                    let username = std::env::var("USERNAME").unwrap_or_default();
                    if let Some(grant_arg) = build_windows_icacls_grant_arg(&username) {
                        match std::process::Command::new("icacls")
                            .arg(&temp_path)
                            .args(["/inheritance:r", "/grant:r"])
                            .arg(grant_arg)
                            .output()
                        {
                            Ok(o) if !o.status.success() => {
                                tracing::warn!(
                                    "Failed to set key file permissions via icacls (exit code {:?})",
                                    o.status.code()
                                );
                            }
                            Err(e) => {
                                tracing::warn!("Could not set key file permissions: {e}");
                            }
                            _ => {
                                tracing::debug!("Key file permissions restricted via icacls");
                            }
                        }
                    } else {
                        tracing::warn!(
                            "USERNAME environment variable is empty; \
                             cannot restrict key file permissions via icacls"
                        );
                    }
                }

                fs::rename(&temp_path, &self.key_path)
                    .context("Failed to rename key file to final path")?;
            }

            Ok(key)
        }
    }
}

/// Generate a random 256-bit key using the OS CSPRNG.
///
/// Uses `OsRng` (via `getrandom`) directly, providing full 256-bit entropy
/// without the fixed version/variant bits that UUID v4 introduces.
fn generate_random_key() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(ChaCha20Poly1305::generate_key(&mut OsRng).to_vec())
}

/// Hex-encode bytes to a lowercase hex string.
fn hex_encode(data: &[u8]) -> String {
    hex::encode(data)
}

/// Build the `/grant` argument for `icacls` using a normalized username.
/// Returns `None` when the username is empty or whitespace-only.
#[cfg(windows)]
fn build_windows_icacls_grant_arg(username: &str) -> Option<String> {
    let normalized = username.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{normalized}:F"))
}

/// Hex-decode a hex string to bytes.
fn hex_decode(hex_str: &str) -> Result<Vec<u8>> {
    hex::decode(hex_str).map_err(|e| anyhow::anyhow!("invalid hex encoding in secret store: {e}"))
}

#[cfg(test)]
mod tests;
