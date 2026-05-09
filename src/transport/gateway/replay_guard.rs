//! Replay protection for webhook endpoints.
//!
//! Tracks SHA-256 hashes of recent request bodies within a TTL window.
//! When storage is configured, persistence is handled asynchronously by a
//! background flusher to keep request paths non-blocking.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEFAULT_TTL_SECS: u64 = 300;
const MAX_ENTRIES: usize = 10_000;
const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_DIRTY_OPS_THRESHOLD: u64 = 256;
const DEFAULT_PRUNE_INTERVAL_OPS: u64 = 64;
static TEMP_FILE_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedReplayState {
    entries: HashMap<String, u64>,
}

#[derive(Debug, Serialize)]
struct PersistedReplayStateRef<'a> {
    entries: &'a HashMap<String, u64>,
}

#[derive(Debug)]
struct ReplayStorageRuntime {
    path: PathBuf,
    seen: Arc<Mutex<HashMap<String, u64>>>,
    ttl_secs: u64,
    flush_interval: Duration,
    dirty_ops_threshold: u64,
    mutation_seq: AtomicU64,
    persisted_seq: AtomicU64,
    stop: AtomicBool,
    wake_tx: SyncSender<()>,
}

#[derive(Debug)]
struct ReplayStorage {
    runtime: Arc<ReplayStorageRuntime>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ReplayStorage {
    fn new(
        path: PathBuf,
        seen: Arc<Mutex<HashMap<String, u64>>>,
        ttl_secs: u64,
        flush_interval: Duration,
        dirty_ops_threshold: u64,
    ) -> Self {
        let (wake_tx, wake_rx) = sync_channel::<()>(1);
        let runtime = Arc::new(ReplayStorageRuntime {
            path,
            seen,
            ttl_secs,
            flush_interval,
            dirty_ops_threshold: dirty_ops_threshold.max(1),
            mutation_seq: AtomicU64::new(0),
            persisted_seq: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            wake_tx,
        });

        let worker_runtime = Arc::clone(&runtime);
        let worker = std::thread::Builder::new()
            .name("replay-guard-flush".to_string())
            .spawn(move || flush_worker_loop(Arc::as_ref(&worker_runtime), &wake_rx))
            .map_err(|error| {
                tracing::warn!(%error, "failed to spawn replay guard flush worker");
                error
            })
            .ok();

        Self {
            runtime,
            worker: Mutex::new(worker),
        }
    }

    fn record_mutation(&self) {
        let sequence = self.runtime.mutation_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let persisted = self.runtime.persisted_seq.load(Ordering::Acquire);

        if sequence.saturating_sub(persisted) < self.runtime.dirty_ops_threshold {
            return;
        }

        match self.runtime.wake_tx.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => {}
            Err(TrySendError::Disconnected(())) => {
                if let Err(error) = flush_replay_state(&self.runtime) {
                    tracing::warn!(
                        %error,
                        path = %self.runtime.path.display(),
                        "failed to flush replay guard state after worker disconnect"
                    );
                }
            }
        }
    }

    fn flush_now(&self) -> io::Result<()> {
        flush_replay_state(&self.runtime)
    }

    fn storage_path(&self) -> &Path {
        &self.runtime.path
    }
}

impl Drop for ReplayStorage {
    fn drop(&mut self) {
        self.runtime.stop.store(true, Ordering::Release);
        let _ = self.runtime.wake_tx.try_send(());

        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            && let Err(error) = worker.join()
        {
            tracing::warn!(
                ?error,
                path = %self.runtime.path.display(),
                "replay guard flush worker panicked"
            );
        }

        if let Err(error) = flush_replay_state(&self.runtime) {
            tracing::warn!(
                %error,
                path = %self.runtime.path.display(),
                "failed to flush replay guard state during drop"
            );
        }
    }
}

fn flush_worker_loop(runtime: &ReplayStorageRuntime, wake_rx: &Receiver<()>) {
    while let Ok(()) | Err(RecvTimeoutError::Timeout) = wake_rx.recv_timeout(runtime.flush_interval)
    {
        if let Err(error) = flush_replay_state(runtime) {
            tracing::warn!(
                %error,
                path = %runtime.path.display(),
                "failed to flush replay guard state"
            );
        }
        if runtime.stop.load(Ordering::Acquire) {
            break;
        }
    }

    if let Err(error) = flush_replay_state(runtime) {
        tracing::warn!(
            %error,
            path = %runtime.path.display(),
            "failed to flush replay guard state on worker shutdown"
        );
    }
}

fn flush_replay_state(runtime: &ReplayStorageRuntime) -> io::Result<()> {
    let target_seq = runtime.mutation_seq.load(Ordering::Acquire);
    let persisted_seq = runtime.persisted_seq.load(Ordering::Acquire);
    if target_seq <= persisted_seq {
        return Ok(());
    }

    let snapshot = {
        let mut seen = runtime
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_entries(&mut seen, current_unix_seconds(), runtime.ttl_secs);
        seen.clone()
    };
    let bytes = serialize_entries(&snapshot)?;

    store_entries(&runtime.path, &bytes)?;
    runtime
        .persisted_seq
        .fetch_max(target_seq, Ordering::AcqRel);
    Ok(())
}

/// TTL-based deduplication guard that rejects replayed request bodies
/// within a sliding time window.
pub struct ReplayGuard {
    seen: Arc<Mutex<HashMap<String, u64>>>,
    ttl: Duration,
    storage: Option<ReplayStorage>,
    mutation_count: AtomicU64,
}

impl Default for ReplayGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayGuard {
    /// Creates an in-memory replay guard with the default TTL.
    pub fn new() -> Self {
        Self::new_with_options(
            None,
            Duration::from_secs(DEFAULT_TTL_SECS),
            DEFAULT_FLUSH_INTERVAL,
            DEFAULT_DIRTY_OPS_THRESHOLD,
        )
    }

    /// Creates a replay guard that persists nonces to disk at `path`.
    pub fn new_with_storage(path: PathBuf) -> Self {
        Self::new_with_options(
            Some(path),
            Duration::from_secs(DEFAULT_TTL_SECS),
            DEFAULT_FLUSH_INTERVAL,
            DEFAULT_DIRTY_OPS_THRESHOLD,
        )
    }

    #[cfg(test)]
    fn new_with_storage_and_flush_options(
        path: PathBuf,
        ttl: Duration,
        flush_interval: Duration,
        dirty_ops_threshold: u64,
    ) -> Self {
        Self::new_with_options(Some(path), ttl, flush_interval, dirty_ops_threshold)
    }

    fn new_with_options(
        storage_path: Option<PathBuf>,
        ttl: Duration,
        flush_interval: Duration,
        dirty_ops_threshold: u64,
    ) -> Self {
        let now = current_unix_seconds();
        let mut seen = storage_path
            .as_deref()
            .and_then(|path| load_entries(path).ok())
            .unwrap_or_default();
        prune_entries(&mut seen, now, ttl.as_secs());

        let seen = Arc::new(Mutex::new(seen));
        let storage = storage_path.map(|path| {
            ReplayStorage::new(
                path,
                Arc::clone(&seen),
                ttl.as_secs(),
                flush_interval,
                dirty_ops_threshold,
            )
        });

        Self {
            seen,
            ttl,
            storage,
            mutation_count: AtomicU64::new(0),
        }
    }

    fn build_nonce(scope: &str, body: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(scope.as_bytes());
        hasher.update(b":");
        hasher.update(body);
        hex::encode(hasher.finalize())
    }

    /// Returns `true` if new (process), `false` if replay (reject).
    pub fn check_and_record_hash(&self, scope: &str, body: &[u8]) -> bool {
        let nonce = Self::build_nonce(scope, body);
        let now = current_unix_seconds();
        let ttl_secs = self.ttl.as_secs();
        let mutation = self.mutation_count.fetch_add(1, Ordering::Relaxed) + 1;

        {
            let mut seen = self
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(ts) = seen.get(&nonce)
                && now.saturating_sub(*ts) < ttl_secs
            {
                return false;
            }
            seen.insert(nonce, now);
            if seen.len() > MAX_ENTRIES || mutation.is_multiple_of(DEFAULT_PRUNE_INTERVAL_OPS) {
                prune_entries(&mut seen, now, ttl_secs);
            }
        }

        if let Some(storage) = &self.storage {
            storage.record_mutation();
        }

        true
    }

    /// Returns `true` if new (process), `false` if replay (reject).
    pub fn check_and_record(&self, body: &[u8]) -> bool {
        self.check_and_record_hash("", body)
    }

    /// Remove a previously recorded nonce, allowing a subsequent retry.
    ///
    /// This is intended for cases where downstream durable processing fails
    /// after replay reservation has already been recorded.
    pub fn forget_scoped(&self, scope: &str, body: &[u8]) {
        let nonce = Self::build_nonce(scope, body);
        let removed = {
            let mut seen = self
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            seen.remove(&nonce).is_some()
        };

        if removed && let Some(storage) = &self.storage {
            storage.record_mutation();
        }
    }

    /// Remove a previously recorded nonce for the default scope.
    pub fn forget(&self, body: &[u8]) {
        self.forget_scoped("", body);
    }

    /// Force an immediate flush of persisted replay state.
    ///
    /// In-memory-only guards return `Ok(())`.
    pub fn flush_now(&self) -> io::Result<()> {
        if let Some(storage) = &self.storage {
            storage.flush_now()?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for ReplayGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayGuard")
            .field("ttl", &self.ttl)
            .field(
                "storage_path",
                &self.storage.as_ref().map(ReplayStorage::storage_path),
            )
            .finish_non_exhaustive()
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn prune_entries(seen: &mut HashMap<String, u64>, now: u64, ttl_secs: u64) {
    seen.retain(|_, ts| now.saturating_sub(*ts) < ttl_secs);
    if seen.len() <= MAX_ENTRIES {
        return;
    }

    let overflow = seen.len().saturating_sub(MAX_ENTRIES);
    if overflow == 0 {
        return;
    }

    // Keep only the newest `MAX_ENTRIES` timestamps without cloning keys.
    let mut timestamps: Vec<u64> = seen.values().copied().collect();
    timestamps.select_nth_unstable(overflow.saturating_sub(1));
    let threshold = timestamps[overflow.saturating_sub(1)];

    let mut removed = 0usize;
    seen.retain(|_, ts| {
        if removed >= overflow {
            return true;
        }
        if *ts < threshold {
            removed += 1;
            false
        } else {
            true
        }
    });

    if removed < overflow {
        seen.retain(|_, ts| {
            if removed >= overflow {
                return true;
            }
            if *ts == threshold {
                removed += 1;
                false
            } else {
                true
            }
        });
    }
}

fn load_entries(path: &Path) -> io::Result<HashMap<String, u64>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let bytes = std::fs::read(path)?;
    let state = serde_json::from_slice::<PersistedReplayState>(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid replay guard state: {error}"),
        )
    })?;
    Ok(state.entries)
}

fn serialize_entries(seen: &HashMap<String, u64>) -> io::Result<Vec<u8>> {
    let payload = PersistedReplayStateRef { entries: seen };
    serde_json::to_vec(&payload).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize replay guard state: {error}"),
        )
    })
}

fn store_entries(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let temp_path = unique_temp_path(path);

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp_path)?;
        file.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&temp_path, bytes)?;
    }

    std::fs::rename(temp_path, path)?;
    Ok(())
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let suffix = TEMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("{suffix}.tmp"))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn new_body_accepted() {
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record(b"first"));
    }

    #[test]
    fn duplicate_body_rejected() {
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record(b"same"));
        assert!(!guard.check_and_record(b"same"));
    }

    #[test]
    fn different_bodies_both_accepted() {
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record(b"one"));
        assert!(guard.check_and_record(b"two"));
    }

    #[test]
    fn forget_allows_retry_after_record() {
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record_hash("webhook", b"same"));
        assert!(!guard.check_and_record_hash("webhook", b"same"));
        guard.forget_scoped("webhook", b"same");
        assert!(guard.check_and_record_hash("webhook", b"same"));
    }

    #[test]
    fn same_body_different_scopes_are_independent() {
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record_hash("whatsapp", b"same"));
        assert!(guard.check_and_record_hash("webhook", b"same"));
        assert!(!guard.check_and_record_hash("whatsapp", b"same"));
    }

    #[test]
    fn persisted_nonce_blocks_replay_after_restart() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("replay_guard.json");

        let guard = ReplayGuard::new_with_storage(state_path.clone());
        assert!(guard.check_and_record_hash("webhook", b"same"));
        guard.flush_now().expect("flush replay guard state");
        let flush_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < flush_deadline
            && std::fs::metadata(&state_path)
                .map(|metadata| metadata.len() == 0)
                .unwrap_or(true)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            state_path.exists(),
            "flush_now should persist the replay guard state file"
        );

        let restarted_guard = ReplayGuard::new_with_storage(state_path);
        assert!(!restarted_guard.check_and_record_hash("webhook", b"same"));
    }

    #[test]
    fn background_flush_persists_without_manual_flush() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("replay_guard_async.json");

        let guard = ReplayGuard::new_with_storage_and_flush_options(
            state_path.clone(),
            Duration::from_secs(DEFAULT_TTL_SECS),
            Duration::from_millis(25),
            1,
        );
        assert!(guard.check_and_record_hash("webhook", b"same"));
        let flush_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < flush_deadline && !state_path.exists() {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            state_path.exists(),
            "background flush should persist the replay state file"
        );
        let visible_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < visible_deadline {
            let restarted_guard = ReplayGuard::new_with_storage(state_path.clone());
            if !restarted_guard.check_and_record_hash("webhook", b"same") {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        panic!("persisted replay state should block the same nonce after restart");
    }

    #[test]
    fn expired_entries_do_not_require_eager_prune_to_allow_retry() {
        let dir = TempDir::new().expect("tempdir");
        let state_path = dir.path().join("replay_guard_expiry.json");

        let guard = ReplayGuard::new_with_storage_and_flush_options(
            state_path,
            Duration::from_millis(20),
            Duration::from_secs(1),
            u64::MAX,
        );
        assert!(guard.check_and_record_hash("webhook", b"same"));
        std::thread::sleep(Duration::from_millis(40));

        assert!(guard.check_and_record_hash("webhook", b"same"));
    }

    #[test]
    fn prune_over_capacity_keeps_newest_entries() {
        let mut seen = HashMap::new();
        for index in 0..(MAX_ENTRIES + 3) {
            seen.insert(format!("nonce-{index}"), index as u64);
        }

        prune_entries(&mut seen, u64::MAX, u64::MAX);

        assert_eq!(seen.len(), MAX_ENTRIES);
        assert!(!seen.contains_key("nonce-0"));
        assert!(!seen.contains_key("nonce-1"));
        assert!(!seen.contains_key("nonce-2"));
        assert!(seen.contains_key("nonce-3"));
    }

    #[test]
    fn unique_temp_path_produces_distinct_paths_for_same_target() {
        let path = PathBuf::from("replay_guard.json");

        let first = unique_temp_path(&path);
        let second = unique_temp_path(&path);

        assert_ne!(first, second);
        assert_ne!(first, path);
        assert_ne!(second, path);
    }
}
