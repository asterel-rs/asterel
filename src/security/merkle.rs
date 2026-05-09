//! Merkle hash chain for tamper-proof audit logs.
//!
//! Each [`AuditEntry`] includes the SHA-256 hash of the previous entry,
//! creating a verifiable chain of custody for every recorded event.

use sha2::{Digest, Sha256};

/// A single entry in the audit hash chain.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Monotonically increasing sequence number (0 = genesis).
    pub sequence: u64,
    /// ISO-8601 timestamp of the event.
    pub timestamp: String,
    /// Machine-readable event classifier.
    pub event_type: String,
    /// Arbitrary event payload (JSON, plain text, etc.).
    pub payload: String,
    /// Hex-encoded SHA-256 hash of the previous entry.
    pub prev_hash: String,
    /// Hex-encoded SHA-256 hash of *this* entry's fields.
    pub hash: String,
}

/// Errors detected when verifying an [`AuditChain`].
#[derive(Debug, thiserror::Error)]
pub enum AuditChainError {
    /// The chain contains no entries.
    #[error("audit chain is empty")]
    EmptyChain,
    /// The `prev_hash` field does not match the preceding entry's hash.
    #[error("broken chain link at sequence {at_sequence}")]
    BrokenChain {
        /// Sequence number of the entry whose `prev_hash` is wrong.
        at_sequence: u64,
    },
    /// The stored hash does not match the recomputed hash of the entry.
    #[error("invalid hash at sequence {at_sequence}")]
    InvalidHash {
        /// Sequence number of the entry whose hash is invalid.
        at_sequence: u64,
    },
}

/// Compute the SHA-256 hash for an audit entry from its constituent fields.
///
/// The hash covers sequence, timestamp, event type, payload, and the
/// previous entry's hash, concatenated with `|` delimiters.
pub fn compute_entry_hash(
    sequence: u64,
    timestamp: &str,
    event_type: &str,
    payload: &str,
    prev_hash: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{sequence}|{timestamp}|{event_type}|{payload}|{prev_hash}"
    ));
    hex::encode(hasher.finalize())
}

/// Maintains an append-only audit hash chain.
#[derive(Debug)]
pub struct AuditChain {
    last_hash: String,
    next_sequence: u64,
}

/// Sentinel "previous hash" for the genesis entry.
const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

impl AuditChain {
    /// Create a new, empty audit chain.
    ///
    /// The first entry appended will use a zeroed-out previous hash.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_hash: GENESIS_PREV_HASH.to_owned(),
            next_sequence: 0,
        }
    }

    /// Append a new event to the chain and return the created entry.
    pub fn append(&mut self, event_type: &str, payload: &str) -> AuditEntry {
        let sequence = self.next_sequence;
        let timestamp = chrono::Utc::now().to_rfc3339();
        let prev_hash = self.last_hash.clone();

        let hash = compute_entry_hash(sequence, &timestamp, event_type, payload, &prev_hash);

        self.last_hash = hash.clone();
        self.next_sequence += 1;

        AuditEntry {
            sequence,
            timestamp,
            event_type: event_type.to_owned(),
            payload: payload.to_owned(),
            prev_hash,
            hash,
        }
    }

    /// Verify the integrity of a sequence of audit entries.
    ///
    /// Checks that:
    /// 1. The chain is non-empty.
    /// 2. Each entry's hash matches a fresh recomputation.
    /// 3. Each entry's `prev_hash` matches the preceding entry's `hash`.
    ///
    /// # Errors
    ///
    /// Returns an [`AuditChainError`] describing the first inconsistency found.
    pub fn verify(entries: &[AuditEntry]) -> Result<(), AuditChainError> {
        if entries.is_empty() {
            return Err(AuditChainError::EmptyChain);
        }

        for (i, entry) in entries.iter().enumerate() {
            // Recompute and compare hash.
            let expected_hash = compute_entry_hash(
                entry.sequence,
                &entry.timestamp,
                &entry.event_type,
                &entry.payload,
                &entry.prev_hash,
            );
            if entry.hash != expected_hash {
                return Err(AuditChainError::InvalidHash {
                    at_sequence: entry.sequence,
                });
            }

            // Verify chain link (except for the first entry).
            if i > 0 {
                let prev = &entries[i - 1];
                if entry.prev_hash != prev.hash {
                    return Err(AuditChainError::BrokenChain {
                        at_sequence: entry.sequence,
                    });
                }
            }
        }

        Ok(())
    }
}

impl Default for AuditChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_verify_multiple_entries() {
        let mut chain = AuditChain::new();
        let mut entries = Vec::new();

        entries.push(chain.append("login", "user=alice"));
        entries.push(chain.append("action", "delete file.txt"));
        entries.push(chain.append("logout", "user=alice"));

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sequence, 0);
        assert_eq!(entries[2].sequence, 2);
        assert_eq!(entries[0].prev_hash, GENESIS_PREV_HASH);
        assert_eq!(entries[1].prev_hash, entries[0].hash);
        assert_eq!(entries[2].prev_hash, entries[1].hash);

        AuditChain::verify(&entries).expect("chain should be valid");
    }

    #[test]
    fn tampered_payload_detected() {
        let mut chain = AuditChain::new();
        let mut entries = vec![
            chain.append("login", "user=alice"),
            chain.append("action", "transfer $100"),
        ];

        // Tamper with the payload of the second entry.
        entries[1].payload = "transfer $999999".to_owned();

        let err = AuditChain::verify(&entries).unwrap_err();
        assert!(
            matches!(err, AuditChainError::InvalidHash { at_sequence: 1 }),
            "expected InvalidHash at sequence 1, got {err:?}"
        );
    }

    #[test]
    fn broken_chain_link_detected() {
        let mut chain = AuditChain::new();
        let mut entries = vec![
            chain.append("a", "1"),
            chain.append("b", "2"),
            chain.append("c", "3"),
        ];

        // Break the chain by swapping prev_hash and recomputing hash
        // so the entry is self-consistent but disconnected from entry[0].
        entries[1].prev_hash = "ff".repeat(32);
        entries[1].hash = compute_entry_hash(
            entries[1].sequence,
            &entries[1].timestamp,
            &entries[1].event_type,
            &entries[1].payload,
            &entries[1].prev_hash,
        );

        let err = AuditChain::verify(&entries).unwrap_err();
        assert!(
            matches!(err, AuditChainError::BrokenChain { at_sequence: 1 }),
            "expected BrokenChain at sequence 1, got {err:?}"
        );
    }

    #[test]
    fn verify_empty_chain_returns_error() {
        let err = AuditChain::verify(&[]).unwrap_err();
        assert!(
            matches!(err, AuditChainError::EmptyChain),
            "expected EmptyChain, got {err:?}"
        );
    }

    #[test]
    fn compute_entry_hash_is_deterministic() {
        let h1 = compute_entry_hash(0, "2025-01-01T00:00:00Z", "test", "data", GENESIS_PREV_HASH);
        let h2 = compute_entry_hash(0, "2025-01-01T00:00:00Z", "test", "data", GENESIS_PREV_HASH);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // hex-encoded SHA-256
    }
}
