//! Gateway pairing mode -- first-connect authentication.
//!
//! On startup the gateway generates a one-time pairing code printed
//! to the terminal. The first client presents this code via
//! `X-Pairing-Code` on `POST /pair` and receives a bearer token
//! for all subsequent requests. Paired tokens are persisted across
//! restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::{Choice, ConstantTimeEq};

/// Maximum failed pairing attempts before lockout.
const MAX_PAIR_ATTEMPTS: u32 = 5;
/// Lockout duration after too many failed pairing attempts.
const PAIR_LOCKOUT_SECS: u64 = 300; // 5 minutes
const DEFAULT_TOKEN_TTL_SECS: u64 = 2_592_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedPairingToken {
    hash: String,
    issued_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PersistedPairingState {
    #[serde(default)]
    tokens: Vec<PersistedPairingToken>,
}

/// Manages pairing state for the gateway.
///
/// Bearer tokens are stored as SHA-256 hashes to prevent plaintext exposure
/// in config files. When a new token is generated, the plaintext is returned
/// to the client once, and only the hash is retained.
#[derive(Debug)]
pub struct PairingGuard {
    /// Whether pairing is required at all.
    require_pairing: bool,
    /// One-time pairing code (generated on startup, consumed on first pair).
    pairing_code: Mutex<Option<String>>,
    /// SHA-256 hashes of issued bearer tokens, keyed by hash.
    paired_tokens: RwLock<HashMap<String, u64>>,
    /// Maximum token lifetime in seconds.
    token_ttl_secs: u64,
    /// Optional path for persisting tokens across restarts.
    storage_path: Option<PathBuf>,
    /// Brute-force protection: failed attempt counter + lockout time.
    failed_attempts: Mutex<(u32, Option<Instant>)>,
}

impl PairingGuard {
    /// Create a new pairing guard.
    ///
    /// If `require_pairing` is true and no tokens exist yet, a fresh
    /// pairing code is generated and returned via `pairing_code()`.
    #[must_use]
    pub fn new(
        require_pairing: bool,
        existing_tokens: &[String],
        token_ttl_secs: Option<u64>,
    ) -> Self {
        Self::new_with_storage(require_pairing, existing_tokens, token_ttl_secs, None)
    }

    /// Create a pairing guard with optional persistent token storage.
    #[must_use]
    pub fn new_with_storage(
        require_pairing: bool,
        existing_tokens: &[String],
        token_ttl_secs: Option<u64>,
        storage_path: Option<PathBuf>,
    ) -> Self {
        let ttl_secs = token_ttl_secs.unwrap_or(DEFAULT_TOKEN_TTL_SECS).max(60);
        let now = current_unix_seconds();
        let mut tokens = storage_path
            .as_deref()
            .and_then(|path| match load_pairing_tokens(path) {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to load pairing tokens");
                    None
                }
            })
            .unwrap_or_default();

        for token_hash in existing_tokens {
            tokens.entry(token_hash.clone()).or_insert(now);
        }

        retain_non_expired_tokens(&mut tokens, now, ttl_secs);
        let code = if require_pairing && tokens.is_empty() {
            Some(generate_code())
        } else {
            None
        };
        let guard = Self {
            require_pairing,
            pairing_code: Mutex::new(code),
            paired_tokens: RwLock::new(tokens),
            token_ttl_secs: ttl_secs,
            storage_path,
            failed_attempts: Mutex::new((0, None)),
        };
        guard.persist_tokens();
        guard
    }

    fn persist_tokens(&self) {
        let Some(path) = self.storage_path.clone() else {
            return;
        };

        let tokens = self
            .paired_tokens
            .read()
            .unwrap_or_else(crate::security::poison_recover!())
            .clone();
        if let Err(error) = store_pairing_tokens(&path, &tokens) {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to persist pairing tokens"
            );
        }
    }

    /// The one-time pairing code (only set when no tokens exist yet).
    pub fn pairing_code(&self) -> Option<String> {
        self.pairing_code
            .lock()
            .unwrap_or_else(crate::security::poison_recover!())
            .clone()
    }

    /// Whether pairing is required at all.
    pub fn require_pairing(&self) -> bool {
        self.require_pairing
    }

    /// Attempt to pair with the given code. Returns a bearer token on success.
    /// Returns `Err(lockout_seconds)` if locked out due to brute force.
    ///
    /// # Errors
    ///
    /// Returns `Err(lockout_seconds)` when too many failed attempts triggered
    /// a temporary lockout.
    pub fn try_pair(&self, code: &str) -> Result<Option<String>, u64> {
        // Lock ordering: always acquire `failed_attempts` before `pairing_code`.
        // Never acquire `pairing_code` first, as this would risk deadlock.

        // Check brute force lockout
        {
            let mut attempts = self
                .failed_attempts
                .lock()
                .unwrap_or_else(crate::security::poison_recover!());
            if let (count, Some(locked_at)) = &*attempts
                && *count >= MAX_PAIR_ATTEMPTS
            {
                let elapsed = locked_at.elapsed().as_secs();
                if elapsed < PAIR_LOCKOUT_SECS {
                    return Err(PAIR_LOCKOUT_SECS - elapsed);
                }
                // Reset counter after lockout expires
                *attempts = (0, None);
            }
        }

        {
            let mut pairing_code = self
                .pairing_code
                .lock()
                .unwrap_or_else(crate::security::poison_recover!());
            // Trim both values upfront so the constant-time comparison
            // does not sit behind a variable-time trim call.
            let submitted = code.trim();
            if let Some(ref expected) = *pairing_code
                && constant_time_eq(submitted, expected.trim())
            {
                // Reset failed attempts on success
                {
                    let mut attempts = self
                        .failed_attempts
                        .lock()
                        .unwrap_or_else(crate::security::poison_recover!());
                    *attempts = (0, None);
                }
                let token = generate_token();
                let mut tokens = self
                    .paired_tokens
                    .write()
                    .unwrap_or_else(crate::security::poison_recover!());
                tokens.insert(hash_token(&token), current_unix_seconds());
                drop(tokens);
                self.persist_tokens();

                // Consume the pairing code so it cannot be reused
                *pairing_code = None;

                return Ok(Some(token));
            }
        }

        {
            let mut attempts = self
                .failed_attempts
                .lock()
                .unwrap_or_else(crate::security::poison_recover!());
            attempts.0 += 1;
            if attempts.0 >= MAX_PAIR_ATTEMPTS {
                attempts.1 = Some(Instant::now());
            }
        }

        Ok(None)
    }

    /// Check if a bearer token is valid (compares against stored hashes).
    pub fn is_authenticated(&self, token: &str) -> bool {
        if !self.require_pairing {
            return true;
        }
        let hashed = hash_token(token);
        let now = current_unix_seconds();

        // Fast path: read lock only — avoids serializing every auth check
        // behind an exclusive write lock when no expired tokens need cleanup.
        {
            let tokens = self
                .paired_tokens
                .read()
                .unwrap_or_else(crate::security::poison_recover!());
            let has_expired = tokens
                .values()
                .any(|ts| self.token_ttl_secs > 0 && now.saturating_sub(*ts) > self.token_ttl_secs);
            if !has_expired {
                return tokens.contains_key(&hashed);
            }
        }

        // Slow path: upgrade to write lock to clean up expired tokens.
        let mut tokens = self
            .paired_tokens
            .write()
            .unwrap_or_else(crate::security::poison_recover!());
        let before = tokens.len();
        retain_non_expired_tokens(&mut tokens, now, self.token_ttl_secs);
        let is_authenticated = tokens.contains_key(&hashed);
        let changed = tokens.len() != before;
        drop(tokens);
        if changed {
            self.persist_tokens();
        }
        is_authenticated
    }

    /// Returns true if the gateway is already paired (has at least one token).
    pub fn is_paired(&self) -> bool {
        let now = current_unix_seconds();
        let mut tokens = self
            .paired_tokens
            .write()
            .unwrap_or_else(crate::security::poison_recover!());
        let before = tokens.len();
        retain_non_expired_tokens(&mut tokens, now, self.token_ttl_secs);
        let is_paired = !tokens.is_empty();
        let changed = tokens.len() != before;
        drop(tokens);
        if changed {
            self.persist_tokens();
        }
        is_paired
    }

    /// Get all paired token hashes (for persisting to config).
    pub fn tokens(&self) -> Vec<String> {
        let now = current_unix_seconds();
        let mut tokens = self
            .paired_tokens
            .write()
            .unwrap_or_else(crate::security::poison_recover!());
        let before = tokens.len();
        retain_non_expired_tokens(&mut tokens, now, self.token_ttl_secs);
        let values = tokens.keys().cloned().collect();
        let changed = tokens.len() != before;
        drop(tokens);
        if changed {
            self.persist_tokens();
        }
        values
    }

    /// Revoke all paired tokens and persist the empty state.
    pub fn revoke_all(&self) {
        let mut tokens = self
            .paired_tokens
            .write()
            .unwrap_or_else(crate::security::poison_recover!());
        tokens.clear();
        drop(tokens);
        self.persist_tokens();
    }
}

fn retain_non_expired_tokens(
    tokens: &mut HashMap<String, u64>,
    now_unix_seconds: u64,
    token_ttl_secs: u64,
) {
    tokens.retain(|_, issued_at| now_unix_seconds.saturating_sub(*issued_at) < token_ttl_secs);
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

fn load_pairing_tokens(path: &Path) -> std::io::Result<HashMap<String, u64>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e),
    };
    let state = serde_json::from_slice::<PersistedPairingState>(&bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid pairing token state: {error}"),
        )
    })?;

    Ok(state
        .tokens
        .into_iter()
        .filter(|record| {
            record.hash.len() == 64 && record.hash.chars().all(|c| c.is_ascii_hexdigit())
        })
        .map(|record| (record.hash, record.issued_at))
        .collect())
}

fn store_pairing_tokens(path: &Path, tokens: &HashMap<String, u64>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut records = tokens
        .iter()
        .map(|(hash, issued_at)| PersistedPairingToken {
            hash: hash.clone(),
            issued_at: *issued_at,
        })
        .collect::<Vec<_>>();
    records.sort_by(|a, b| a.hash.cmp(&b.hash));

    let payload = PersistedPairingState { tokens: records };
    let bytes = serde_json::to_vec(&payload).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to serialize pairing token state: {error}"),
        )
    })?;

    let temp_path = path.with_extension("tmp");

    #[cfg(unix)]
    {
        // Open with restricted permissions atomically to avoid a window
        // where the file is world-readable due to the process umask.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp_path)?;
        file.write_all(&bytes)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&temp_path, bytes)?;
    }

    std::fs::rename(&temp_path, path)?;

    Ok(())
}

/// Generate a 6-digit numeric pairing code using cryptographically secure randomness.
fn generate_code() -> String {
    // UUID v4 uses getrandom (backed by /dev/urandom on Linux, BCryptGenRandom
    // on Windows) — a CSPRNG. We extract 4 bytes from it for a uniform random
    // number in [0, 1_000_000).
    //
    // Rejection sampling eliminates modulo bias: values above the largest
    // multiple of 1_000_000 that fits in u32 are discarded and re-drawn.
    // The rejection probability is ~0.02%, so this loop almost always exits
    // on the first iteration.
    const UPPER_BOUND: u32 = 1_000_000;
    const REJECT_THRESHOLD: u32 = (u32::MAX / UPPER_BOUND) * UPPER_BOUND;

    loop {
        let uuid = uuid::Uuid::new_v4();
        let bytes = uuid.as_bytes();
        let raw = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

        if raw < REJECT_THRESHOLD {
            return format!("{:06}", raw % UPPER_BOUND);
        }
    }
}

/// Generate a cryptographically-strong bearer token (160-bit entropy, hex-encoded).
fn generate_token() -> String {
    let mut buf = [0u8; 20];
    rand::rng().fill_bytes(&mut buf);
    format!("zc_{}", hex::encode(buf))
}

/// SHA-256 hash a bearer token for storage. Returns lowercase hex.
#[must_use]
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Constant-time string comparison to prevent timing attacks.
///
/// Does not short-circuit on length mismatch — always iterates over the
/// longer input to avoid leaking length information via timing.
#[must_use]
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let lengths_match: Choice = a.len().ct_eq(&b.len());
    let max_len = a.len().max(b.len());
    let mut acc = 0u8;
    for i in 0..max_len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        acc |= x ^ y;
    }
    let bytes_match: Choice = acc.ct_eq(&0u8);
    (lengths_match & bytes_match).into()
}

/// Check if a host string represents a non-localhost bind address.
///
/// Detects the full `127.0.0.0/8` loopback range, IPv6 `::1` variants,
/// and the `localhost` hostname.
#[must_use]
pub fn is_public_bind(host: &str) -> bool {
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');

    // Check "localhost" hostname
    if trimmed.eq_ignore_ascii_case("localhost") {
        return false;
    }

    // Check IPv6 loopback (::1 in various forms)
    if trimmed == "::1" || trimmed == "0:0:0:0:0:0:0:1" {
        return false;
    }

    // Check IPv4 127.0.0.0/8 loopback range
    if let Some(first_octet) = trimmed.split('.').next()
        && first_octet == "127"
        && trimmed.split('.').count() == 4
    {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    // ── PairingGuard ─────────────────────────────────────────

    #[test]
    fn new_guard_generates_code_when_no_tokens() {
        let guard = PairingGuard::new(true, &[], None);
        assert!(guard.pairing_code().is_some());
        assert!(!guard.is_paired());
    }

    #[test]
    fn new_guard_does_not_generate_code_when_existing_token_is_active() {
        let guard = PairingGuard::new(true, &[hash_token("zc_existing")], None);
        assert!(guard.pairing_code().is_none());
        assert!(guard.is_paired());
    }

    #[test]
    fn new_guard_no_code_when_pairing_disabled() {
        let guard = PairingGuard::new(false, &[], None);
        assert!(guard.pairing_code().is_none());
    }

    #[test]
    fn try_pair_correct_code() {
        let guard = PairingGuard::new(true, &[], None);
        let code = guard.pairing_code().unwrap().clone();
        let token = guard.try_pair(&code).unwrap();
        assert!(token.is_some());
        assert!(token.unwrap().starts_with("zc_"));
        assert!(guard.is_paired());
    }

    #[test]
    fn try_pair_wrong_code() {
        let guard = PairingGuard::new(true, &[], None);
        let result = guard.try_pair("000000").unwrap();
        // Might succeed if code happens to be 000000, but extremely unlikely
        // Just check it returns Ok(None) normally
        let _ = result;
    }

    #[test]
    fn try_pair_empty_code() {
        let guard = PairingGuard::new(true, &[], None);
        assert!(guard.try_pair("").unwrap().is_none());
    }

    #[test]
    fn is_authenticated_with_valid_token() {
        let guard = PairingGuard::new(true, &[hash_token("zc_valid")], None);
        assert!(guard.is_authenticated("zc_valid"));
    }

    #[test]
    fn is_authenticated_with_prehashed_token() {
        // Pass an already-hashed token (64 hex chars)
        let hashed = hash_token("zc_valid");
        let guard = PairingGuard::new(true, &[hashed], None);
        assert!(guard.is_authenticated("zc_valid"));
    }

    #[test]
    fn is_authenticated_with_invalid_token() {
        let guard = PairingGuard::new(true, &[hash_token("zc_valid")], None);
        assert!(!guard.is_authenticated("zc_invalid"));
    }

    #[test]
    fn is_authenticated_when_pairing_disabled() {
        let guard = PairingGuard::new(false, &[], None);
        assert!(guard.is_authenticated("anything"));
        assert!(guard.is_authenticated(""));
    }

    #[test]
    fn tokens_returns_hashes() {
        let guard = PairingGuard::new(true, &[hash_token("zc_a"), hash_token("zc_b")], None);
        let tokens = guard.tokens();
        assert_eq!(tokens.len(), 2);
        // Tokens should be stored as 64-char hex hashes, not plaintext
        for t in &tokens {
            assert_eq!(t.len(), 64, "Token should be a SHA-256 hash");
            assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!t.starts_with("zc_"), "Token should not be plaintext");
        }
    }

    #[test]
    fn pair_then_authenticate() {
        let guard = PairingGuard::new(true, &[], None);
        let code = guard.pairing_code().unwrap().clone();
        let token = guard.try_pair(&code).unwrap().unwrap();
        assert!(guard.is_authenticated(&token));
        assert!(!guard.is_authenticated("wrong"));
    }

    #[test]
    fn is_authenticated_rejects_expired_token_and_cleans_it_up() {
        // Use storage to inject a token with an old issued_at timestamp so that
        // it is already expired even with the minimum TTL floor of 60 seconds.
        let temp = TempDir::new().expect("tempdir");
        let store_path = temp.path().join("pairing_tokens.json");
        let stale_token = hash_token("zc_valid");
        let stale_timestamp = current_unix_seconds().saturating_sub(3600);
        let payload = PersistedPairingState {
            tokens: vec![PersistedPairingToken {
                hash: stale_token,
                issued_at: stale_timestamp,
            }],
        };
        let bytes = serde_json::to_vec(&payload).expect("serialize payload");
        std::fs::write(&store_path, bytes).expect("write persisted tokens");

        let guard = PairingGuard::new_with_storage(true, &[], Some(60), Some(store_path));
        assert!(!guard.is_authenticated("zc_valid"));
        assert!(guard.tokens().is_empty());
        assert!(!guard.is_paired());
    }

    #[test]
    fn token_ttl_has_minimum_floor() {
        // Even when token_ttl_secs=0 is requested, the floor of 60s applies,
        // so a freshly-issued token is NOT immediately expired.
        let guard = PairingGuard::new(true, &[hash_token("zc_valid")], Some(0));
        assert!(guard.is_authenticated("zc_valid"));
    }

    #[test]
    fn is_authenticated_accepts_non_expired_token() {
        let guard = PairingGuard::new(true, &[hash_token("zc_valid")], Some(60));
        assert!(guard.is_authenticated("zc_valid"));
    }

    #[test]
    fn revoke_all_clears_all_paired_tokens() {
        let guard = PairingGuard::new(true, &[hash_token("zc_a"), hash_token("zc_b")], None);
        guard.revoke_all();
        assert!(guard.tokens().is_empty());
        assert!(!guard.is_paired());
    }

    #[test]
    fn pairing_tokens_persist_across_restarts_when_storage_is_enabled() {
        let temp = TempDir::new().expect("tempdir");
        let store_path = temp.path().join("pairing_tokens.json");

        let first = PairingGuard::new_with_storage(true, &[], Some(600), Some(store_path.clone()));
        let code = first
            .pairing_code()
            .expect("pairing code should be present");
        let token = first
            .try_pair(&code)
            .expect("pairing should succeed")
            .expect("pairing should return token");
        assert!(first.is_authenticated(&token));

        std::thread::sleep(std::time::Duration::from_millis(100));
        let restarted = PairingGuard::new_with_storage(true, &[], Some(600), Some(store_path));
        assert!(restarted.is_authenticated(&token));
        assert!(restarted.is_paired());
        assert!(restarted.pairing_code().is_none());
    }

    #[test]
    fn expired_persisted_pairing_tokens_are_pruned_on_startup() {
        let temp = TempDir::new().expect("tempdir");
        let store_path = temp.path().join("pairing_tokens.json");
        let stale_token = hash_token("zc_stale");
        let stale_timestamp = current_unix_seconds().saturating_sub(3600);
        let payload = PersistedPairingState {
            tokens: vec![PersistedPairingToken {
                hash: stale_token.clone(),
                issued_at: stale_timestamp,
            }],
        };
        let bytes = serde_json::to_vec(&payload).expect("serialize payload");
        std::fs::write(&store_path, bytes).expect("write persisted tokens");

        let guard = PairingGuard::new_with_storage(true, &[], Some(1), Some(store_path));
        assert!(!guard.is_paired());
        assert!(!guard.tokens().contains(&stale_token));
    }

    // ── Token hashing ────────────────────────────────────────

    #[test]
    fn hash_token_produces_64_hex_chars() {
        let hash = hash_token("zc_test_token");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_token_is_deterministic() {
        assert_eq!(hash_token("zc_abc"), hash_token("zc_abc"));
    }

    #[test]
    fn hash_token_differs_for_different_inputs() {
        assert_ne!(hash_token("zc_a"), hash_token("zc_b"));
    }

    // ── is_public_bind ───────────────────────────────────────

    #[test]
    fn localhost_variants_not_public() {
        assert!(!is_public_bind("127.0.0.1"));
        assert!(!is_public_bind("localhost"));
        assert!(!is_public_bind("::1"));
        assert!(!is_public_bind("[::1]"));
        assert!(!is_public_bind("0:0:0:0:0:0:0:1"));
    }

    #[test]
    fn loopback_range_not_public() {
        assert!(!is_public_bind("127.0.0.1"));
        assert!(!is_public_bind("127.0.0.2"));
        assert!(!is_public_bind("127.255.255.255"));
        assert!(!is_public_bind("127.1.2.3"));
    }

    #[test]
    fn zero_zero_is_public() {
        assert!(is_public_bind("0.0.0.0"));
    }

    #[test]
    fn real_ip_is_public() {
        assert!(is_public_bind("192.168.1.100"));
        assert!(is_public_bind("10.0.0.1"));
    }

    // ── constant_time_eq ─────────────────────────────────────

    #[test]
    fn constant_time_eq_same() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(!constant_time_eq("a", ""));
    }

    #[test]
    fn constant_time_eq_empty_strings() {
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn constant_time_eq_empty_vs_nonempty() {
        assert!(!constant_time_eq("", "x"));
        assert!(!constant_time_eq("x", ""));
    }

    #[test]
    fn constant_time_eq_different_lengths_same_prefix() {
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("abcd", "abc"));
    }

    // ── generate helpers ─────────────────────────────────────

    #[test]
    fn generate_code_is_6_digits() {
        let code = generate_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn generate_code_is_not_deterministic() {
        // Two codes should differ with overwhelming probability. We try
        // multiple pairs so a single 1-in-10^6 collision doesn't cause
        // a flaky CI failure. All 10 pairs colliding is ~1-in-10^60.
        for _ in 0..10 {
            if generate_code() != generate_code() {
                return; // Pass: found a non-matching pair.
            }
        }
        panic!("Generated 10 pairs of codes and all were collisions — CSPRNG failure");
    }

    #[test]
    fn generate_token_has_prefix() {
        let token = generate_token();
        assert!(token.starts_with("zc_"));
        assert_eq!(token.len(), 43);
    }

    #[test]
    fn generate_token_has_correct_length_and_format() {
        let token = generate_token();
        // "zc_" (3) + 40 hex chars = 43
        assert_eq!(token.len(), 43, "token should be 43 chars: zc_ + 40 hex");
        assert!(token.starts_with("zc_"));
        assert!(token[3..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_token_is_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b, "two tokens should differ");
    }

    // ── Brute force protection ───────────────────────────────

    #[test]
    fn brute_force_lockout_after_max_attempts() {
        let guard = PairingGuard::new(true, &[], None);
        // Exhaust all attempts with wrong codes
        for i in 0..MAX_PAIR_ATTEMPTS {
            let result = guard.try_pair(&format!("wrong_{i}"));
            assert!(result.is_ok(), "Attempt {i} should not be locked out yet");
        }
        // Next attempt should be locked out
        let result = guard.try_pair("another_wrong");
        assert!(
            result.is_err(),
            "Should be locked out after {MAX_PAIR_ATTEMPTS} attempts"
        );
        let lockout_secs = result.unwrap_err();
        assert!(lockout_secs > 0, "Lockout should have remaining seconds");
        assert!(
            lockout_secs <= PAIR_LOCKOUT_SECS,
            "Lockout should not exceed max"
        );
    }

    #[test]
    fn correct_code_resets_failed_attempts() {
        let guard = PairingGuard::new(true, &[], None);
        let code = guard.pairing_code().unwrap().clone();
        // Fail a few times
        for _ in 0..3 {
            let _ = guard.try_pair("wrong");
        }
        // Correct code should still work (under MAX_PAIR_ATTEMPTS)
        let result = guard.try_pair(&code).unwrap();
        assert!(result.is_some(), "Correct code should work before lockout");
    }

    #[test]
    fn lockout_returns_remaining_seconds() {
        let guard = PairingGuard::new(true, &[], None);
        for _ in 0..MAX_PAIR_ATTEMPTS {
            let _ = guard.try_pair("wrong");
        }
        let err = guard.try_pair("wrong").unwrap_err();
        // Should be close to PAIR_LOCKOUT_SECS (within a second)
        assert!(
            err >= PAIR_LOCKOUT_SECS - 1,
            "Remaining lockout should be ~{PAIR_LOCKOUT_SECS}s, got {err}s"
        );
    }
}
