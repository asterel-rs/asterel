//! Ed25519 manifest signing and verification.
//!
//! Provides [`SigningKeyPair`] for generating keys, signing arbitrary data,
//! and verifying signatures. [`ManifestSignature`] is a portable container
//! suitable for embedding in JSON/TOML manifest files.

use ed25519_dalek::{Signer, Verifier};
use rand::RngExt;

/// Wrapper around an Ed25519 signing key pair.
pub struct SigningKeyPair {
    key: ed25519_dalek::SigningKey,
}

/// Hex-encoded Ed25519 signature for serialization.
#[derive(Debug, Clone)]
pub struct Signature {
    /// Raw 64-byte signature encoded as 128-character hex string.
    bytes: [u8; 64],
}

/// Portable manifest signature record suitable for serialization.
#[derive(Debug, Clone)]
pub struct ManifestSignature {
    /// Hex-encoded Ed25519 public key (32 bytes -> 64 hex chars).
    pub public_key: String,
    /// Hex-encoded Ed25519 signature (64 bytes -> 128 hex chars).
    pub signature: String,
    /// ISO-8601 timestamp of when the signature was created.
    pub signed_at: String,
}

/// Errors that can occur during signature verification.
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    /// The signature does not match the data and public key.
    #[error("signature verification failed")]
    VerificationFailed,
    /// The public key bytes are malformed.
    #[error("invalid public key: {reason}")]
    InvalidPublicKey {
        /// Description of why the key is invalid.
        reason: String,
    },
    /// The signature bytes are malformed.
    #[error("invalid signature encoding: {reason}")]
    InvalidSignature {
        /// Description of why the signature is invalid.
        reason: String,
    },
}

impl Signature {
    /// Return the hex-encoded representation of this signature.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }

    /// Decode a hex string into a [`Signature`].
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::InvalidSignature`] if the hex string is not
    /// exactly 128 characters or contains non-hex characters.
    pub fn from_hex(s: &str) -> Result<Self, SigningError> {
        let bytes = hex::decode(s).map_err(|e| SigningError::InvalidSignature {
            reason: e.to_string(),
        })?;
        let arr: [u8; 64] = bytes.try_into().map_err(|_| SigningError::InvalidSignature {
            reason: "expected 64 bytes".to_owned(),
        })?;
        Ok(Self { bytes: arr })
    }
}

impl SigningKeyPair {
    /// Generate a new random Ed25519 key pair.
    #[must_use]
    pub fn generate() -> Self {
        let secret: [u8; 32] = rand::rng().random();
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        Self { key }
    }

    /// Sign arbitrary data and return a [`Signature`].
    #[must_use]
    pub fn sign(&self, data: &[u8]) -> Signature {
        let sig = self.key.sign(data);
        Signature {
            bytes: sig.to_bytes(),
        }
    }

    /// Verify a signature against data and a public key.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError`] if the public key is invalid or the
    /// signature does not match.
    pub fn verify(
        data: &[u8],
        signature: &Signature,
        public_key: &[u8; 32],
    ) -> Result<(), SigningError> {
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(public_key).map_err(|e| {
            SigningError::InvalidPublicKey {
                reason: e.to_string(),
            }
        })?;
        let sig = ed25519_dalek::Signature::from_bytes(&signature.bytes);
        verifying_key
            .verify(data, &sig)
            .map_err(|_| SigningError::VerificationFailed)
    }

    /// Return the raw 32-byte public key.
    #[must_use]
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
}

/// Sign a manifest blob and produce a [`ManifestSignature`] record.
#[must_use]
pub fn sign_manifest(keypair: &SigningKeyPair, manifest_bytes: &[u8]) -> ManifestSignature {
    let signature = keypair.sign(manifest_bytes);
    ManifestSignature {
        public_key: hex::encode(keypair.public_key_bytes()),
        signature: signature.to_hex(),
        signed_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Verify a manifest blob against its [`ManifestSignature`].
///
/// # Errors
///
/// Returns [`SigningError`] if the public key or signature is malformed,
/// or if the signature does not match the data.
pub fn verify_manifest(
    manifest_bytes: &[u8],
    sig: &ManifestSignature,
) -> Result<(), SigningError> {
    let pk_bytes = hex::decode(&sig.public_key).map_err(|e| SigningError::InvalidPublicKey {
        reason: e.to_string(),
    })?;
    let pk: [u8; 32] = pk_bytes.try_into().map_err(|_| SigningError::InvalidPublicKey {
        reason: "expected 32 bytes".to_owned(),
    })?;
    let signature = Signature::from_hex(&sig.signature)?;
    SigningKeyPair::verify(manifest_bytes, &signature, &pk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let kp = SigningKeyPair::generate();
        let data = b"hello world";
        let sig = kp.sign(data);
        let pk = kp.public_key_bytes();

        SigningKeyPair::verify(data, &sig, &pk).expect("valid signature should verify");
    }

    #[test]
    fn tampered_data_fails_verification() {
        let kp = SigningKeyPair::generate();
        let sig = kp.sign(b"original");
        let pk = kp.public_key_bytes();

        let err = SigningKeyPair::verify(b"tampered", &sig, &pk).unwrap_err();
        assert!(
            matches!(err, SigningError::VerificationFailed),
            "expected VerificationFailed, got {err:?}"
        );
    }

    #[test]
    fn different_key_fails_verification() {
        let kp1 = SigningKeyPair::generate();
        let kp2 = SigningKeyPair::generate();
        let data = b"payload";
        let sig = kp1.sign(data);

        let err = SigningKeyPair::verify(data, &sig, &kp2.public_key_bytes()).unwrap_err();
        assert!(
            matches!(err, SigningError::VerificationFailed),
            "expected VerificationFailed, got {err:?}"
        );
    }

    #[test]
    fn manifest_sign_and_verify() {
        let kp = SigningKeyPair::generate();
        let manifest = br#"{"name":"my-tool","version":"1.0"}"#;
        let ms = sign_manifest(&kp, manifest);

        assert_eq!(ms.public_key.len(), 64);
        assert_eq!(ms.signature.len(), 128);
        assert!(!ms.signed_at.is_empty());

        verify_manifest(manifest, &ms).expect("manifest signature should verify");
    }

    #[test]
    fn manifest_tampered_fails() {
        let kp = SigningKeyPair::generate();
        let ms = sign_manifest(&kp, b"original manifest");

        let err = verify_manifest(b"tampered manifest", &ms).unwrap_err();
        assert!(
            matches!(err, SigningError::VerificationFailed),
            "expected VerificationFailed, got {err:?}"
        );
    }

    #[test]
    fn signature_hex_round_trip() {
        let kp = SigningKeyPair::generate();
        let sig = kp.sign(b"test");
        let hex_str = sig.to_hex();
        let recovered = Signature::from_hex(&hex_str).expect("valid hex should decode");
        assert_eq!(sig.bytes, recovered.bytes);
    }
}
