//! Dynamic trust tracker: per-domain trust scores with time decay and
//! automatic autonomy demotion (WP-G4).
//!
//! Trust is earned, not granted. Each domain (e.g. "shell", "mcp:github",
//! "file:src/") maintains a floating-point score that decays over time.
//! Repeated successful use builds trust; violations trigger penalties and
//! can demote autonomy (Full → Supervised → ReadOnly).
//!
//! Design source: ecosystem survey 2026-04-03 (ZeroClaw TrustTracker,
//! IronClaw SafetyLayer). Per §6.4.H: principles adopted, redesigned
//! for Asterel's affect-informed governance.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::security::policy::AutonomyLevel;

/// Runtime trust state for a single domain.
///
/// ## Trust lifecycle
///
/// 1. **Initial score** — every new domain starts at `0.5` (neutral) with
///    `Supervised` autonomy. No domain is trusted or distrusted by default.
///
/// 2. **Decay** — the score decays exponentially toward neutral (`0.5`) over
///    time. After one half-life period (default: 7 days) the gap between the
///    current score and `0.5` is halved. This means trust built up quickly
///    fades if a domain is not used, reducing the blast radius of stale grants.
///
/// 3. **Boost / penalty** — each call to [`DomainTrustTracker::record_success`] adds
///    `success_boost` (default `+0.02`) and each call to
///    [`DomainTrustTracker::record_violation`] subtracts `violation_penalty`
///    (default `−0.15`). Violations have a larger magnitude than successes
///    deliberately: it takes many successes to build trust, but few violations
///    to erode it.
///
/// 4. **Promotion / demotion** — after every score change, autonomy level is
///    re-evaluated: if the score exceeds `promotion_threshold` *and* enough
///    interactions have occurred, autonomy steps up; if the score falls below
///    `demotion_threshold`, autonomy steps down. See [`maybe_promote`] and
///    [`maybe_demote`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainTrust {
    /// Domain identifier (e.g. `shell`, `mcp:github`, `file:src/`).
    pub domain: String,
    /// Trust score in [0.0, 1.0]. Higher = more trusted.
    pub score: f32,
    /// Current autonomy level for this domain.
    pub autonomy: AutonomyLevel,
    /// Last time this domain's trust was updated.
    pub last_updated: DateTime<Utc>,
    /// Cumulative successful interactions.
    pub success_count: u32,
    /// Cumulative violations.
    pub violation_count: u32,
}

impl DomainTrust {
    fn new(domain: String) -> Self {
        Self {
            domain,
            score: 0.5, // neutral starting point
            autonomy: AutonomyLevel::Supervised,
            last_updated: Utc::now(),
            success_count: 0,
            violation_count: 0,
        }
    }
}

/// Configuration for trust dynamics.
#[derive(Debug, Clone)]
pub struct TrustConfig {
    /// Half-life for trust decay in hours.
    pub decay_half_life_hours: f64,
    /// Score boost per successful interaction.
    pub success_boost: f32,
    /// Score penalty per violation.
    pub violation_penalty: f32,
    /// Score threshold below which autonomy is demoted.
    pub demotion_threshold: f32,
    /// Score threshold above which autonomy can be promoted.
    pub promotion_threshold: f32,
    /// Minimum interactions before promotion is possible.
    pub min_interactions_for_promotion: u32,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            decay_half_life_hours: 168.0, // 7 days
            success_boost: 0.02,
            violation_penalty: 0.15,
            demotion_threshold: 0.3,
            promotion_threshold: 0.8,
            min_interactions_for_promotion: 10,
        }
    }
}

/// The domain trust tracker. Thread-safe via internal mutex.
pub struct DomainTrustTracker {
    domains: Mutex<HashMap<String, DomainTrust>>,
    config: TrustConfig,
}

impl DomainTrustTracker {
    /// Create a new tracker with default config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            domains: Mutex::new(HashMap::new()),
            config: TrustConfig::default(),
        }
    }

    /// Create with custom config.
    #[must_use]
    pub fn with_config(config: TrustConfig) -> Self {
        Self {
            domains: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Get the current trust state for a domain, applying time decay.
    #[must_use]
    pub fn get_trust(&self, domain: &str) -> DomainTrust {
        let mut domains = self
            .domains
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = domains
            .entry(domain.to_string())
            .or_insert_with_key(|k| DomainTrust::new(k.clone()));
        apply_decay(entry, &self.config);
        entry.clone()
    }

    /// Record a successful interaction for a domain, potentially promoting
    /// its autonomy level.
    ///
    /// Callers should invoke this after a tool call in the domain completes
    /// without error, without operator intervention, and without triggering
    /// any safety policy. Do **not** call it for no-op or skipped tool calls —
    /// only completed, observable actions count toward trust building.
    pub fn record_success(&self, domain: &str) {
        let mut domains = self
            .domains
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = domains
            .entry(domain.to_string())
            .or_insert_with_key(|k| DomainTrust::new(k.clone()));
        apply_decay(entry, &self.config);
        entry.score = (entry.score + self.config.success_boost).min(1.0);
        entry.success_count += 1;
        entry.last_updated = Utc::now();
        maybe_promote(entry, &self.config);
    }

    /// Record a violation for a domain, potentially demoting its autonomy level.
    ///
    /// Callers should invoke this when a tool call in the domain triggered a
    /// safety policy block, an operator explicitly rejected an action, or an
    /// affect-governance bridge determined that the agent is in a high-distress
    /// state. "Hard" violations (e.g., a denied destructive command) warrant
    /// direct calls; "soft" violations (e.g., affect-triggered caution) are
    /// recorded by [`crate::security::affect_governance::AffectGovernanceBridge`].
    ///
    /// Note: violations carry a larger magnitude than successes by design — it
    /// takes many successful interactions to build trust, but few failures to
    /// erode it.
    pub fn record_violation(&self, domain: &str) {
        let mut domains = self
            .domains
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = domains
            .entry(domain.to_string())
            .or_insert_with_key(|k| DomainTrust::new(k.clone()));
        apply_decay(entry, &self.config);
        entry.score = (entry.score - self.config.violation_penalty).max(0.0);
        entry.violation_count += 1;
        entry.last_updated = Utc::now();
        maybe_demote(entry, &self.config);
    }

    /// Get all tracked domains and their current trust state.
    #[must_use]
    pub fn all_domains(&self) -> Vec<DomainTrust> {
        let mut domains = self
            .domains
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in domains.values_mut() {
            apply_decay(entry, &self.config);
        }
        domains.values().cloned().collect()
    }
}

impl Default for DomainTrustTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply exponential time decay to a trust score.
fn apply_decay(trust: &mut DomainTrust, config: &TrustConfig) {
    let now = Utc::now();
    #[allow(clippy::cast_precision_loss)]
    let elapsed_hours = (now - trust.last_updated).num_seconds() as f64 / 3600.0;
    if elapsed_hours <= 0.0 {
        return;
    }
    // Exponential decay: at each half-life period, the gap between the current
    // score and neutral (0.5) shrinks by 50%.  Formula:
    //   new_score = 0.5 + (old_score − 0.5) × 0.5^(elapsed / half_life)
    // A score of 0.9 after one half-life becomes 0.5 + 0.4×0.5 = 0.7.
    let half_life = config.decay_half_life_hours;
    #[allow(clippy::cast_possible_truncation)]
    let decay_factor = (0.5_f64).powf(elapsed_hours / half_life) as f32;
    let neutral = 0.5_f32;
    trust.score = neutral + (trust.score - neutral) * decay_factor;
    trust.last_updated = now;
}

/// Promote the domain's autonomy level if it has earned it.
///
/// Promotion requires *both* a high score (≥ `promotion_threshold`) *and* a
/// minimum number of recorded interactions (`min_interactions_for_promotion`).
/// The interaction gate prevents a domain from being promoted immediately
/// after a single lucky operation that happens to push the score above the
/// threshold.
///
/// Transition: `ReadOnly → Supervised`, `Supervised → Full`.
/// A domain already at `Full` remains at `Full`.
fn maybe_promote(trust: &mut DomainTrust, config: &TrustConfig) {
    if trust.score >= config.promotion_threshold
        && trust.success_count >= config.min_interactions_for_promotion
    {
        trust.autonomy = match trust.autonomy {
            AutonomyLevel::ReadOnly => AutonomyLevel::Supervised,
            AutonomyLevel::Supervised | AutonomyLevel::Full => AutonomyLevel::Full,
        };
    }
}

/// Demote the domain's autonomy level when its trust score falls too low.
///
/// Demotion is unconditional on score alone — no interaction minimum applies.
/// A single large violation can push a domain below the threshold immediately.
///
/// Transition: `Full → Supervised`, `Supervised → ReadOnly`.
/// A domain already at `ReadOnly` remains at `ReadOnly`.
fn maybe_demote(trust: &mut DomainTrust, config: &TrustConfig) {
    if trust.score < config.demotion_threshold {
        trust.autonomy = match trust.autonomy {
            AutonomyLevel::Full => AutonomyLevel::Supervised,
            AutonomyLevel::Supervised | AutonomyLevel::ReadOnly => AutonomyLevel::ReadOnly,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_domain_starts_neutral() {
        let tracker = DomainTrustTracker::new();
        let trust = tracker.get_trust("shell");
        assert!((trust.score - 0.5).abs() < 0.01);
        assert_eq!(trust.autonomy, AutonomyLevel::Supervised);
    }

    #[test]
    fn success_boosts_score() {
        let tracker = DomainTrustTracker::new();
        tracker.record_success("shell");
        tracker.record_success("shell");
        tracker.record_success("shell");
        let trust = tracker.get_trust("shell");
        // 3 boosts of 0.02 from 0.5 base → ~0.56
        assert!(trust.score > 0.55, "score={}, expected > 0.55", trust.score);
        assert!(trust.score < 0.7, "score={}, expected < 0.7", trust.score);
        assert_eq!(trust.success_count, 3);
        assert_eq!(trust.autonomy, AutonomyLevel::Supervised);
    }

    #[test]
    fn time_decay_moves_score_toward_neutral() {
        let tracker = DomainTrustTracker::new();
        tracker.record_success("shell");
        tracker.record_success("shell");

        // Manually backdate the last_updated to simulate time passing
        {
            let mut domains = tracker.domains.lock().unwrap();
            let entry = domains.get_mut("shell").unwrap();
            // Set last_updated to 7 days ago (one half-life)
            entry.last_updated = chrono::Utc::now() - chrono::Duration::days(7);
            entry.score = 0.9; // high trust
        }

        let trust = tracker.get_trust("shell");
        // After one half-life, score should decay halfway toward 0.5
        // 0.5 + (0.9 - 0.5) * 0.5 = 0.7
        assert!(
            trust.score > 0.65,
            "score={}, expected > 0.65 after 1 half-life",
            trust.score
        );
        assert!(
            trust.score < 0.75,
            "score={}, expected < 0.75 after 1 half-life",
            trust.score
        );
    }

    #[test]
    fn violation_reduces_score_and_demotes() {
        let config = TrustConfig {
            violation_penalty: 0.3,
            demotion_threshold: 0.3,
            ..TrustConfig::default()
        };
        let tracker = DomainTrustTracker::with_config(config);
        tracker.record_violation("shell");
        let trust = tracker.get_trust("shell");
        assert!(trust.score < 0.3);
        assert_eq!(trust.autonomy, AutonomyLevel::ReadOnly);
    }

    #[test]
    fn promotion_requires_min_interactions() {
        let config = TrustConfig {
            success_boost: 0.05,
            promotion_threshold: 0.8,
            min_interactions_for_promotion: 5,
            ..TrustConfig::default()
        };
        let tracker = DomainTrustTracker::with_config(config);
        // 3 successes: not enough interactions for promotion
        for _ in 0..3 {
            tracker.record_success("file:src/");
        }
        let trust = tracker.get_trust("file:src/");
        assert_eq!(trust.autonomy, AutonomyLevel::Supervised);

        // Push past min_interactions + score threshold
        for _ in 0..10 {
            tracker.record_success("file:src/");
        }
        let trust = tracker.get_trust("file:src/");
        assert_eq!(trust.autonomy, AutonomyLevel::Full);
    }

    #[test]
    fn violation_demotes_full_to_supervised() {
        let config = TrustConfig {
            success_boost: 0.05,
            violation_penalty: 0.3,
            demotion_threshold: 0.3,
            promotion_threshold: 0.8,
            min_interactions_for_promotion: 3,
            ..TrustConfig::default()
        };
        let tracker = DomainTrustTracker::with_config(config);
        // Build up to Full autonomy
        for _ in 0..10 {
            tracker.record_success("shell");
        }
        let trust = tracker.get_trust("shell");
        assert_eq!(trust.autonomy, AutonomyLevel::Full);

        // Multiple violations to push below demotion threshold (0.3)
        // Score is ~1.0 after promotions. Need 3 violations of 0.3 to reach ~0.1
        tracker.record_violation("shell");
        tracker.record_violation("shell");
        tracker.record_violation("shell");
        let trust = tracker.get_trust("shell");
        assert!(
            trust.score < 0.3,
            "score={} should be below demotion threshold",
            trust.score
        );
        // Full → Supervised on first demotion trigger
        assert_ne!(
            trust.autonomy,
            AutonomyLevel::Full,
            "Full should have demoted after violations below threshold"
        );
    }

    #[test]
    fn all_domains_returns_tracked() {
        let tracker = DomainTrustTracker::new();
        tracker.record_success("shell");
        tracker.record_success("mcp:github");
        let domains = tracker.all_domains();
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn violation_count_tracks() {
        let tracker = DomainTrustTracker::new();
        tracker.record_violation("shell");
        tracker.record_violation("shell");
        let trust = tracker.get_trust("shell");
        assert_eq!(trust.violation_count, 2);
    }
}
