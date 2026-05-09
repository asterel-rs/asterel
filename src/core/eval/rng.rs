//! Deterministic PRNG and helper functions for reproducible
//! synthetic evaluation runs and fingerprint computation.

use super::types::EvalSuiteSummary;

/// Splitmix64-based PRNG for reproducible eval simulations.
#[derive(Debug, Clone)]
pub(super) struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    /// Seed the PRNG with an initial state value.
    pub(super) fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Generate the next pseudo-random `u64`.
    pub(super) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Generate a random value in `[0, upper_exclusive)`.
    pub(super) fn next_bounded(&mut self, upper_exclusive: u64) -> u64 {
        if upper_exclusive == 0 {
            return 0;
        }
        self.next_u64() % upper_exclusive
    }
}

/// Map a random sample into the inclusive range `[min, max]`.
pub(super) fn bounded_inclusive(min: u32, max: u32, sample: u64) -> u32 {
    if min >= max {
        return min;
    }
    let span = u64::from(max - min) + 1;
    min + u32_saturating_from_u64(sample % span)
}

/// Convert a `u64` to `u32`, clamping at `u32::MAX` on overflow.
pub(super) fn u32_saturating_from_u64(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Derive a per-scenario seed by mixing suite and scenario names.
pub(super) fn mix_seed(base_seed: u64, suite_name: &str, scenario_id: &str) -> u64 {
    let mut mixed = base_seed ^ fnv1a64(suite_name.as_bytes());
    mixed = mixed.rotate_left(17) ^ fnv1a64(scenario_id.as_bytes());
    mixed
}

/// Compute an FNV-based fingerprint over a seed and suite summaries.
pub(super) fn fingerprint_summary(seed: u64, suites: &[EvalSuiteSummary]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64 ^ seed;
    for suite in suites {
        hash = hash_combine(hash, suite.suite.as_bytes());
        hash ^= u64::from(suite.case_count);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
        hash ^= u64::from(suite.success_rate_bps);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
        hash ^= u64::from(suite.avg_cost_cents);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
        hash ^= u64::from(suite.avg_latency_ms);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
        hash ^= u64::from(suite.avg_retries_milli);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
    }
    hash
}

fn hash_combine(mut state: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        state ^= u64::from(byte);
        state = state.wrapping_mul(0x1000_0000_01B3);
    }
    state
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
    }
    hash
}
