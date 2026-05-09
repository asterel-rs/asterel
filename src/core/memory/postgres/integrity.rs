//! Integrity verification for Postgres memory.
//!
//! Maintains SHA-256 hash chains on the `memory_events` and
//! `deletion_ledger` tables and verifies them on demand using
//! `PostgreSQL` advisory locks for serialization.

use sha2::{Digest, Sha256};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::PgRow;

use super::error::{PostgresMemoryResult, PostgresMemoryResultExt};
use crate::core::memory::memory_types::{MemoryIntegrityIssue, MemoryIntegrityReport};

const MEMORY_EVENTS_GENESIS_SEED: &str = "asterel-integrity:memory_events:genesis:v1";
const DELETION_LEDGER_GENESIS_SEED: &str = "asterel-integrity:deletion_ledger:genesis:v1";

/// Advisory lock keys for serializing hash chain writes.
/// These are arbitrary constants chosen to avoid collision with other advisory locks.
pub(super) const MEMORY_EVENTS_CHAIN_LOCK: i64 = 0x4173_7465_726F_6E01; // "Asteron" + 01
pub(super) const DELETION_LEDGER_CHAIN_LOCK: i64 = 0x4173_7465_726F_6E02; // "Asteron" + 02

fn genesis_hash(seed: &str) -> String {
    let hash = Sha256::digest(seed.as_bytes());
    hex::encode(hash)
}

fn canonical_f64(v: f64) -> String {
    if v.is_nan() {
        "0".to_string()
    } else if v == f64::INFINITY {
        format!("{:.17}", f64::MAX)
    } else if v == f64::NEG_INFINITY {
        format!("{:.17}", f64::MIN)
    } else if v == 0.0 {
        "0".to_string()
    } else {
        format!("{v:.17}")
    }
}

fn optional_or_empty(v: Option<&str>) -> &str {
    v.unwrap_or("")
}

/// Fields required to compute a `memory_events` integrity hash.
pub(crate) struct MemoryEventHashFields<'a> {
    pub(crate) event_id: &'a str,
    pub(crate) entity_id: &'a str,
    pub(crate) slot_key: &'a str,
    pub(crate) layer: &'a str,
    pub(crate) event_type: &'a str,
    pub(crate) value: &'a str,
    pub(crate) source: &'a str,
    pub(crate) confidence: f64,
    pub(crate) importance: f64,
    pub(crate) provenance_source_class: Option<&'a str>,
    pub(crate) provenance_reference: Option<&'a str>,
    pub(crate) provenance_evidence_uri: Option<&'a str>,
    pub(crate) retention_tier: &'a str,
    pub(crate) retention_expires_at: Option<&'a str>,
    pub(crate) signal_tier: &'a str,
    pub(crate) source_kind: Option<&'a str>,
    pub(crate) privacy_level: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) ingested_at: &'a str,
    pub(crate) supersedes_event_id: Option<&'a str>,
}

/// Fields required to compute a `deletion_ledger` integrity hash.
pub(super) struct DeletionLedgerHashFields<'a> {
    pub(super) ledger_id: &'a str,
    pub(crate) entity_id: &'a str,
    pub(super) target_slot_key: &'a str,
    pub(super) phase: &'a str,
    pub(super) reason: &'a str,
    pub(super) requested_by: &'a str,
    pub(super) executed_at: &'a str,
}

/// Compute the integrity hash for a `memory_events` row.
fn compute_memory_event_hash(prev_hash: &str, f: &MemoryEventHashFields<'_>) -> String {
    let canonical = format!(
        "v=1\n\
         chain=memory_events\n\
         prev_hash={prev_hash}\n\
         event_id={event_id}\n\
         entity_id={entity_id}\n\
         slot_key={slot_key}\n\
         layer={layer}\n\
         event_type={event_type}\n\
         value={value}\n\
         source={source}\n\
         confidence={confidence}\n\
         importance={importance}\n\
         provenance_source_class={psc}\n\
         provenance_reference={pr}\n\
         provenance_evidence_uri={peu}\n\
         retention_tier={retention_tier}\n\
         retention_expires_at={rea}\n\
         signal_tier={signal_tier}\n\
         source_kind={sk}\n\
         privacy_level={privacy_level}\n\
         occurred_at={occurred_at}\n\
         ingested_at={ingested_at}\n\
         supersedes_event_id={sei}",
        event_id = f.event_id,
        entity_id = f.entity_id,
        slot_key = f.slot_key,
        layer = f.layer,
        event_type = f.event_type,
        value = f.value,
        source = f.source,
        confidence = canonical_f64(f.confidence),
        importance = canonical_f64(f.importance),
        psc = optional_or_empty(f.provenance_source_class),
        pr = optional_or_empty(f.provenance_reference),
        peu = optional_or_empty(f.provenance_evidence_uri),
        retention_tier = f.retention_tier,
        rea = optional_or_empty(f.retention_expires_at),
        signal_tier = f.signal_tier,
        sk = optional_or_empty(f.source_kind),
        privacy_level = f.privacy_level,
        occurred_at = f.occurred_at,
        ingested_at = f.ingested_at,
        sei = optional_or_empty(f.supersedes_event_id),
    );

    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Compute the integrity hash for a `deletion_ledger` row.
fn compute_deletion_ledger_hash(prev_hash: &str, f: &DeletionLedgerHashFields<'_>) -> String {
    let canonical = format!(
        "v=1\n\
         chain=deletion_ledger\n\
         prev_hash={prev_hash}\n\
         ledger_id={ledger_id}\n\
         entity_id={entity_id}\n\
         target_slot_key={target_slot_key}\n\
         phase={phase}\n\
         reason={reason}\n\
         requested_by={requested_by}\n\
         executed_at={executed_at}",
        ledger_id = f.ledger_id,
        entity_id = f.entity_id,
        target_slot_key = f.target_slot_key,
        phase = f.phase,
        reason = f.reason,
        requested_by = f.requested_by,
        executed_at = f.executed_at,
    );

    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Compute the next hash in the `memory_events` chain.
///
/// Reads the last `integrity_hash` from the table, then computes the new hash.
pub(crate) async fn next_memory_event_chain(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    fields: &MemoryEventHashFields<'_>,
) -> PostgresMemoryResult<(String, String)> {
    // Acquire transaction-scoped advisory lock to serialize chain writes.
    // This prevents concurrent transactions from reading the same prev_hash.
    query("SELECT pg_advisory_xact_lock($1)")
        .bind(MEMORY_EVENTS_CHAIN_LOCK)
        .execute(&mut **tx)
        .await
        .pg_integrity("acquire memory_events chain advisory lock")?;

    let prev_hash: String =
        query("SELECT integrity_hash FROM memory_events ORDER BY seq_id DESC LIMIT 1")
            .fetch_optional(&mut **tx)
            .await
            .pg_integrity("fetch last memory_events integrity hash")?
            .map_or_else(
                || genesis_hash(MEMORY_EVENTS_GENESIS_SEED),
                |row| row.get::<String, _>("integrity_hash"),
            );

    let hash = compute_memory_event_hash(&prev_hash, fields);

    Ok((prev_hash, hash))
}

/// Compute the next hash in the `deletion_ledger` chain.
pub(super) async fn next_deletion_ledger_chain(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    fields: &DeletionLedgerHashFields<'_>,
) -> PostgresMemoryResult<(String, String)> {
    // Acquire transaction-scoped advisory lock to serialize chain writes.
    query("SELECT pg_advisory_xact_lock($1)")
        .bind(DELETION_LEDGER_CHAIN_LOCK)
        .execute(&mut **tx)
        .await
        .pg_integrity("acquire deletion_ledger chain advisory lock")?;

    let prev_hash: String =
        query("SELECT integrity_hash FROM deletion_ledger ORDER BY seq_id DESC LIMIT 1")
            .fetch_optional(&mut **tx)
            .await
            .pg_integrity("fetch last deletion_ledger integrity hash")?
            .map_or_else(
                || genesis_hash(DELETION_LEDGER_GENESIS_SEED),
                |row| row.get::<String, _>("integrity_hash"),
            );

    let hash = compute_deletion_ledger_hash(&prev_hash, fields);

    Ok((prev_hash, hash))
}

use super::PostgresMemory;

const INTEGRITY_VERIFICATION_ROW_LIMIT: usize = 100_000;

struct ChainVerification {
    checked_rows: usize,
    truncated: bool,
    issues: Vec<MemoryIntegrityIssue>,
}

impl PostgresMemory {
    /// Verify integrity of both hash chains.
    pub(super) async fn verify_integrity_impl(
        &self,
    ) -> PostgresMemoryResult<MemoryIntegrityReport> {
        let mut memory_events = self.verify_memory_events_chain().await?;
        let deletion_ledger = self.verify_deletion_ledger_chain().await?;
        let checked_memory_events = memory_events.checked_rows;
        let checked_deletion_ledger = deletion_ledger.checked_rows;

        if memory_events.truncated {
            memory_events.issues.push(MemoryIntegrityIssue {
                chain: "memory_events".to_string(),
                row_key: String::new(),
                reason: format!(
                    "Verification truncated at {INTEGRITY_VERIFICATION_ROW_LIMIT} rows; \
                     remaining rows were not checked"
                ),
            });
        }
        if deletion_ledger.truncated {
            memory_events.issues.push(MemoryIntegrityIssue {
                chain: "deletion_ledger".to_string(),
                row_key: String::new(),
                reason: format!(
                    "Verification truncated at {INTEGRITY_VERIFICATION_ROW_LIMIT} rows; \
                     remaining rows were not checked"
                ),
            });
        }

        memory_events.issues.extend(deletion_ledger.issues);
        let issues = memory_events.issues;

        Ok(MemoryIntegrityReport {
            backend: "postgres".to_string(),
            is_verified: issues.is_empty(),
            checked_memory_events,
            checked_deletion_ledger,
            issues,
        })
    }

    async fn verify_memory_events_chain(&self) -> PostgresMemoryResult<ChainVerification> {
        let rows = query(
            "SELECT seq_id, event_id, entity_id, slot_key, layer, event_type, value, source, \
                    confidence, importance, \
                    provenance_source_class, provenance_reference, provenance_evidence_uri, \
                    retention_tier, \
                    to_char(retention_expires_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS retention_expires_at_str, \
                    signal_tier, source_kind, privacy_level, \
                    to_char(occurred_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS occurred_at_str, \
                    to_char(ingested_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS ingested_at_str, \
                    supersedes_event_id, \
                    integrity_prev_hash, integrity_hash \
             FROM memory_events ORDER BY seq_id ASC \
             LIMIT 100000",
        )
        .fetch_all(&self.pool)
        .await
        .pg_integrity("fetch memory_events for integrity verification")?;

        let mut issues = Vec::new();
        let mut expected_prev = genesis_hash(MEMORY_EVENTS_GENESIS_SEED);
        for row in &rows {
            expected_prev = verify_memory_event_row(row, &expected_prev, &mut issues);
        }

        Ok(ChainVerification {
            checked_rows: rows.len(),
            truncated: rows.len() >= INTEGRITY_VERIFICATION_ROW_LIMIT,
            issues,
        })
    }

    async fn verify_deletion_ledger_chain(&self) -> PostgresMemoryResult<ChainVerification> {
        let rows = query(
            "SELECT seq_id, ledger_id, entity_id, target_slot_key, phase, reason, \
                    requested_by, \
                    to_char(executed_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS executed_at_str, \
                    integrity_prev_hash, integrity_hash \
             FROM deletion_ledger ORDER BY seq_id ASC \
             LIMIT 100000",
        )
        .fetch_all(&self.pool)
        .await
        .pg_integrity("fetch deletion_ledger for integrity verification")?;

        let mut issues = Vec::new();
        let mut expected_prev = genesis_hash(DELETION_LEDGER_GENESIS_SEED);
        for row in &rows {
            expected_prev = verify_deletion_ledger_row(row, &expected_prev, &mut issues);
        }

        Ok(ChainVerification {
            checked_rows: rows.len(),
            truncated: rows.len() >= INTEGRITY_VERIFICATION_ROW_LIMIT,
            issues,
        })
    }
}

fn verify_memory_event_row(
    row: &PgRow,
    expected_prev: &str,
    issues: &mut Vec<MemoryIntegrityIssue>,
) -> String {
    let stored_prev: String = row.get("integrity_prev_hash");
    let stored_hash: String = row.get("integrity_hash");
    let event_id: String = row.get("event_id");

    if stored_prev != expected_prev {
        issues.push(MemoryIntegrityIssue {
            chain: "memory_events".to_string(),
            row_key: event_id.clone(),
            reason: format!("prev_hash mismatch: expected={expected_prev}, stored={stored_prev}"),
        });
    }

    let entity_id = row.get::<String, _>("entity_id");
    let slot_key = row.get::<String, _>("slot_key");
    let layer = row.get::<String, _>("layer");
    let event_type = row.get::<String, _>("event_type");
    let value = row.get::<String, _>("value");
    let source = row.get::<String, _>("source");
    let retention_tier = row.get::<String, _>("retention_tier");
    let signal_tier = row.get::<String, _>("signal_tier");
    let privacy_level = row.get::<String, _>("privacy_level");
    let occurred_at = row.get::<String, _>("occurred_at_str");
    let ingested_at = row.get::<String, _>("ingested_at_str");
    let provenance_source_class = row.try_get::<String, _>("provenance_source_class").ok();
    let provenance_reference = row.try_get::<String, _>("provenance_reference").ok();
    let provenance_evidence_uri = row.try_get::<String, _>("provenance_evidence_uri").ok();
    let retention_expires_at = row.try_get::<String, _>("retention_expires_at_str").ok();
    let source_kind = row.try_get::<String, _>("source_kind").ok();
    let supersedes_event_id = row.try_get::<String, _>("supersedes_event_id").ok();

    let computed = compute_memory_event_hash(
        &stored_prev,
        &MemoryEventHashFields {
            event_id: &event_id,
            entity_id: &entity_id,
            slot_key: &slot_key,
            layer: &layer,
            event_type: &event_type,
            value: &value,
            source: &source,
            confidence: row.get::<f64, _>("confidence"),
            importance: row.get::<f64, _>("importance"),
            provenance_source_class: provenance_source_class.as_deref(),
            provenance_reference: provenance_reference.as_deref(),
            provenance_evidence_uri: provenance_evidence_uri.as_deref(),
            retention_tier: &retention_tier,
            retention_expires_at: retention_expires_at.as_deref(),
            signal_tier: &signal_tier,
            source_kind: source_kind.as_deref(),
            privacy_level: &privacy_level,
            occurred_at: &occurred_at,
            ingested_at: &ingested_at,
            supersedes_event_id: supersedes_event_id.as_deref(),
        },
    );

    if computed != stored_hash {
        issues.push(MemoryIntegrityIssue {
            chain: "memory_events".to_string(),
            row_key: event_id,
            reason: format!("hash mismatch: computed={computed}, stored={stored_hash}"),
        });
    }

    stored_hash
}

fn verify_deletion_ledger_row(
    row: &PgRow,
    expected_prev: &str,
    issues: &mut Vec<MemoryIntegrityIssue>,
) -> String {
    let stored_prev: String = row.get("integrity_prev_hash");
    let stored_hash: String = row.get("integrity_hash");
    let ledger_id: String = row.get("ledger_id");

    if stored_prev != expected_prev {
        issues.push(MemoryIntegrityIssue {
            chain: "deletion_ledger".to_string(),
            row_key: ledger_id.clone(),
            reason: format!("prev_hash mismatch: expected={expected_prev}, stored={stored_prev}"),
        });
    }

    let entity_id = row.get::<String, _>("entity_id");
    let target_slot_key = row.get::<String, _>("target_slot_key");
    let phase = row.get::<String, _>("phase");
    let reason = row.get::<String, _>("reason");
    let requested_by = row.get::<String, _>("requested_by");
    let executed_at = row.get::<String, _>("executed_at_str");

    let computed = compute_deletion_ledger_hash(
        &stored_prev,
        &DeletionLedgerHashFields {
            ledger_id: &ledger_id,
            entity_id: &entity_id,
            target_slot_key: &target_slot_key,
            phase: &phase,
            reason: &reason,
            requested_by: &requested_by,
            executed_at: &executed_at,
        },
    );

    if computed != stored_hash {
        issues.push(MemoryIntegrityIssue {
            chain: "deletion_ledger".to_string(),
            row_key: ledger_id,
            reason: format!("hash mismatch: computed={computed}, stored={stored_hash}"),
        });
    }

    stored_hash
}
