//! Memory hygiene subsystem: scheduled archival, pruning, and
//! sleep-phase consolidation.
//!
//! The public entry point is [`run_if_due`] (called from the backend
//! factory after every construction). It checks a persisted timestamp and
//! only runs when at least [`state::HYGIENE_INTERVAL_HOURS`] have elapsed
//! since the last pass.
//!
//! ## What a hygiene cycle does
//!
//! 1. **Filesystem archival** — moves old `YYYY-MM-DD.md` memory logs and
//!    session files into `archive/` subdirectories (`filesystem`).
//! 2. **Filesystem purge** — deletes archived files older than
//!    `purge_after_days` (`filesystem`).
//! 3. **Conversation pruning** — deletes stale inferred `belief_slots` /
//!    `retrieval_units` and their graph projections (`prune`).
//! 4. **Lifecycle pruning** — TTL expiry, low-confidence demotion,
//!    contradiction auto-demotion, stale trend demotion, recency refresh,
//!    and layer-specific cleanup (`prune`).
//! 5. **Working-memory promotion** — promotes expiring high-importance
//!    working slots into the episodic layer (`promotion`).
//! 6. **Sleep consolidation** — groups recent episodic memories by topic
//!    prefix and writes aggregated semantic snapshots (`sleep`).
//!    Runs on a separate 24-hour cadence.

mod filesystem;
pub(super) mod promotion;
mod prune;
mod sleep;
mod state;

pub use state::run_if_due;
