//! Argon2id key derivation for password-based secret encryption.
//!
//! Derives a 256-bit symmetric key from a user passphrase using
//! Argon2id (RFC 9106) with secure default parameters. The derived
//! key is suitable for use with ChaCha20-Poly1305 AEAD.

use anyhow::{Context, Result};
use argon2::Argon2;
use chacha20poly1305::Key;
use chacha20poly1305::aead::Generate;
use zeroize::Zeroizing;

/// Derived key length in bytes (256-bit key for ChaCha20-Poly1305).
const KEY_LEN: usize = 32;

/// Salt length in bytes.
pub const SALT_LEN: usize = 16;

/// Configurable Argon2id cost parameters.
#[derive(Debug, Clone, Copy)]
pub struct KdfParams {
    /// Memory cost in KiB (default: 65536 = 64 MiB).
    pub m_cost: u32,
    /// Time cost / iterations (default: 3).
    pub t_cost: u32,
    /// Parallelism degree (default: 4).
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost: 65_536,
            t_cost: 3,
            p_cost: 4,
        }
    }
}

/// Generate a random 16-byte salt using the OS CSPRNG.
#[must_use]
pub fn generate_salt() -> [u8; SALT_LEN] {
    let random = Key::generate();
    let mut salt = [0_u8; SALT_LEN];
    salt.copy_from_slice(&random[..SALT_LEN]);
    salt
}

/// Derive a 32-byte key from a password and salt using Argon2id.
///
/// Uses the provided [`KdfParams`] (or secure defaults) to configure
/// memory, time, and parallelism costs. The returned key is wrapped
/// in [`Zeroizing`] to ensure it is zeroed on drop.
///
/// # Errors
///
/// Returns an error if the Argon2id algorithm fails (e.g. invalid params).
pub fn derive_key_from_password(
    password: &[u8],
    salt: &[u8],
    params: &KdfParams,
) -> Result<Zeroizing<Vec<u8>>> {
    let argon2_params =
        argon2::Params::new(params.m_cost, params.t_cost, params.p_cost, Some(KEY_LEN))
            .context("invalid Argon2id parameters")?;
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2_params,
    );

    let mut key = Zeroizing::new(vec![0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, &mut key)
        .map_err(|e| anyhow::anyhow!("Argon2id key derivation failed: {e}"))?;

    Ok(key)
}

/// Derive a 32-byte key using secure default parameters.
///
/// Convenience wrapper around [`derive_key_from_password`] with
/// [`KdfParams::default()`].
///
/// # Errors
///
/// Returns an error if key derivation fails.
pub fn derive_key_default(password: &[u8], salt: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    derive_key_from_password(password, salt, &KdfParams::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_derivation() {
        let password = b"correct-horse-battery-staple";
        let salt = [0x42u8; SALT_LEN];

        let key1 = derive_key_default(password, &salt).unwrap();
        let key2 = derive_key_default(password, &salt).unwrap();

        assert_eq!(
            *key1, *key2,
            "Same password + salt must produce identical keys"
        );
    }

    #[test]
    fn different_passwords_produce_different_keys() {
        let salt = [0x42u8; SALT_LEN];

        let key1 = derive_key_default(b"password-alpha", &salt).unwrap();
        let key2 = derive_key_default(b"password-bravo", &salt).unwrap();

        assert_ne!(
            *key1, *key2,
            "Different passwords must produce different keys"
        );
    }

    #[test]
    fn different_salts_produce_different_keys() {
        let password = b"same-password";

        let key1 = derive_key_default(password, &[0x01u8; SALT_LEN]).unwrap();
        let key2 = derive_key_default(password, &[0x02u8; SALT_LEN]).unwrap();

        assert_ne!(*key1, *key2, "Different salts must produce different keys");
    }

    #[test]
    fn output_is_32_bytes() {
        let key = derive_key_default(b"test-password", &[0xAAu8; SALT_LEN]).unwrap();
        assert_eq!(key.len(), 32, "Derived key must be 32 bytes");
    }

    #[test]
    fn generate_salt_not_all_zeros() {
        let salt = generate_salt();
        assert!(salt.iter().any(|&b| b != 0), "Salt should not be all zeros");
    }

    #[test]
    fn two_salts_differ() {
        let s1 = generate_salt();
        let s2 = generate_salt();
        assert_ne!(s1, s2, "Two random salts should differ");
    }

    #[test]
    fn custom_params_work() {
        let params = KdfParams {
            m_cost: 8192,
            t_cost: 1,
            p_cost: 1,
        };
        let key = derive_key_from_password(b"test", &[0xBBu8; SALT_LEN], &params).unwrap();
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn key_uses_zeroizing_wrapper() {
        let key: Zeroizing<Vec<u8>> = derive_key_default(b"test", &[0xCCu8; SALT_LEN]).unwrap();
        assert!(!key.is_empty());
    }
}
