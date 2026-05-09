//! Persona forced override stack: allows external sources to temporarily
//! override adaptive/volatile persona fields without touching immutable
//! identity principles or safety posture.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Override source ────────────────────────────────────────────────

/// Who requested this override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverrideSource {
    /// External API call.
    Api,
    /// Configuration file.
    Config,
    /// Tool invocation.
    Tool,
    /// Internal system (e.g., safety escalation).
    System,
}

// ── Override expiry ────────────────────────────────────────────────

/// When this override expires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverrideExpiry {
    /// Expires after N turns.
    TurnCount(u32),
    /// Expires after N seconds from `created_at`.
    Duration(u64),
    /// Until explicitly removed.
    Indefinite,
}

// ── Field overrides ────────────────────────────────────────────────

/// Overridable persona fields. Only adaptive/volatile fields are
/// included — immutable fields (`identity_principles_hash`,
/// `safety_posture`) are deliberately excluded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldOverrides {
    /// Override the current objective.
    #[serde(default)]
    pub current_objective: Option<String>,
    /// Override formality score (0-100).
    #[serde(default)]
    pub formality: Option<u8>,
    /// Override verbosity score (0-100).
    #[serde(default)]
    pub verbosity: Option<u8>,
    /// Temperature adjustment, clamped to [-0.20, 0.20].
    #[serde(default)]
    pub temperature_delta: Option<f64>,
    /// Override reasoning strategy label.
    #[serde(default)]
    pub reasoning_strategy: Option<String>,
    /// Extra system prompt injected before LLM call.
    #[serde(default)]
    pub extra_system_prompt: Option<String>,
}

const MAX_TEMPERATURE_DELTA: f64 = 0.20;
const MAX_FORMALITY: u8 = 100;
const MAX_VERBOSITY: u8 = 100;

impl FieldOverrides {
    /// Clamp all numeric fields to their valid ranges.
    fn clamped(mut self) -> Self {
        if let Some(t) = self.temperature_delta {
            self.temperature_delta = Some(clamp_temperature_delta(t));
        }
        if let Some(f) = self.formality {
            self.formality = Some(f.min(MAX_FORMALITY));
        }
        if let Some(v) = self.verbosity {
            self.verbosity = Some(v.min(MAX_VERBOSITY));
        }
        self
    }

    /// Merge `other` into `self`: fields in `other` win when present.
    fn merge_from(&mut self, other: &Self) {
        if other.current_objective.is_some() {
            self.current_objective.clone_from(&other.current_objective);
        }
        if other.formality.is_some() {
            self.formality = other.formality;
        }
        if other.verbosity.is_some() {
            self.verbosity = other.verbosity;
        }
        if other.temperature_delta.is_some() {
            self.temperature_delta = other.temperature_delta;
        }
        if other.reasoning_strategy.is_some() {
            self.reasoning_strategy
                .clone_from(&other.reasoning_strategy);
        }
        if other.extra_system_prompt.is_some() {
            self.extra_system_prompt
                .clone_from(&other.extra_system_prompt);
        }
    }
}

fn clamp_temperature_delta(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(-MAX_TEMPERATURE_DELTA, MAX_TEMPERATURE_DELTA)
    }
}

// ── Override entry ─────────────────────────────────────────────────

/// A single override entry on the stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaOverrideEntry {
    /// Unique identifier (UUID v4).
    pub override_id: String,
    /// Who requested this override.
    pub source: OverrideSource,
    /// Which fields to override.
    pub field_overrides: FieldOverrides,
    /// When this override expires.
    pub expiry: OverrideExpiry,
    /// RFC 3339 timestamp of creation.
    pub created_at: String,
    /// Higher values take precedence (0-255).
    pub priority: u8,
}

impl PersonaOverrideEntry {
    /// Create a new entry with a fresh UUID and the current timestamp.
    #[must_use]
    pub fn new(
        source: OverrideSource,
        field_overrides: FieldOverrides,
        expiry: OverrideExpiry,
        priority: u8,
    ) -> Self {
        Self {
            override_id: Uuid::new_v4().to_string(),
            source,
            field_overrides,
            expiry,
            created_at: Utc::now().to_rfc3339(),
            priority,
        }
    }
}

// ── Override stack ─────────────────────────────────────────────────

/// Priority-aware stack of persona field overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaOverrideStack {
    entries: Vec<PersonaOverrideEntry>,
}

impl PersonaOverrideStack {
    /// Create an empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Push an override entry onto the stack.
    pub fn push(&mut self, entry: PersonaOverrideEntry) {
        self.entries.push(entry);
    }

    /// Remove an override by its ID. Returns `true` if found and removed.
    pub fn remove(&mut self, override_id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.override_id != override_id);
        self.entries.len() < before
    }

    /// Remove entries whose `TurnCount` expiry has been exceeded.
    pub fn expire_stale(&mut self, current_turn: u32) {
        self.entries.retain(|e| match e.expiry {
            OverrideExpiry::TurnCount(n) => current_turn < n,
            _ => true,
        });
    }

    /// Remove entries whose `Duration` expiry has been exceeded
    /// relative to wall-clock time.
    pub fn expire_by_time(&mut self) {
        let now = Utc::now();
        self.entries.retain(|e| {
            if let OverrideExpiry::Duration(secs) = e.expiry {
                if let Ok(created) = DateTime::parse_from_rfc3339(&e.created_at) {
                    let deadline = created
                        + chrono::Duration::seconds(i64::try_from(secs).unwrap_or(i64::MAX));
                    return now < deadline;
                }
                // Malformed temporary overrides are treated as expired; use
                // `Indefinite` explicitly for intentionally persistent entries.
                false
            } else {
                true
            }
        });
    }

    /// Merge all active entries by priority (higher wins per-field)
    /// and return the resulting overrides with numeric fields clamped.
    #[must_use]
    pub fn effective_overrides(&self) -> FieldOverrides {
        if self.entries.is_empty() {
            return FieldOverrides::default();
        }

        // Sort a copy by priority ascending so that later (higher) wins.
        let mut sorted: Vec<&PersonaOverrideEntry> = self.entries.iter().collect();
        sorted.sort_by_key(|e| e.priority);

        let mut merged = FieldOverrides::default();
        for entry in &sorted {
            merged.merge_from(&entry.field_overrides);
        }
        merged.clamped()
    }

    /// Whether the stack has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of active entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Slice of current entries.
    #[must_use]
    pub fn active_entries(&self) -> &[PersonaOverrideEntry] {
        &self.entries
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(
        priority: u8,
        overrides: FieldOverrides,
        expiry: OverrideExpiry,
    ) -> PersonaOverrideEntry {
        PersonaOverrideEntry {
            override_id: Uuid::new_v4().to_string(),
            source: OverrideSource::Api,
            field_overrides: overrides,
            expiry,
            created_at: Utc::now().to_rfc3339(),
            priority,
        }
    }

    #[test]
    fn empty_stack_returns_no_overrides() {
        let stack = PersonaOverrideStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        let eff = stack.effective_overrides();
        assert_eq!(eff, FieldOverrides::default());
    }

    #[test]
    fn single_override_applied() {
        let mut stack = PersonaOverrideStack::new();
        let entry = make_entry(
            10,
            FieldOverrides {
                formality: Some(80),
                verbosity: Some(50),
                temperature_delta: Some(0.10),
                current_objective: Some("test objective".into()),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        );
        stack.push(entry);

        assert_eq!(stack.len(), 1);
        let eff = stack.effective_overrides();
        assert_eq!(eff.formality, Some(80));
        assert_eq!(eff.verbosity, Some(50));
        assert!((eff.temperature_delta.unwrap() - 0.10).abs() < f64::EPSILON);
        assert_eq!(eff.current_objective.as_deref(), Some("test objective"));
        assert_eq!(eff.reasoning_strategy, None);
        assert_eq!(eff.extra_system_prompt, None);
    }

    #[test]
    fn priority_ordering_higher_wins() {
        let mut stack = PersonaOverrideStack::new();
        // Low priority sets formality=30
        stack.push(make_entry(
            5,
            FieldOverrides {
                formality: Some(30),
                verbosity: Some(20),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));
        // High priority sets formality=90
        stack.push(make_entry(
            50,
            FieldOverrides {
                formality: Some(90),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));

        let eff = stack.effective_overrides();
        // Higher priority (50) wins for formality.
        assert_eq!(eff.formality, Some(90));
        // Verbosity only set by low priority, so it persists.
        assert_eq!(eff.verbosity, Some(20));
    }

    #[test]
    fn turn_based_expiry() {
        let mut stack = PersonaOverrideStack::new();
        stack.push(make_entry(
            10,
            FieldOverrides {
                formality: Some(60),
                ..Default::default()
            },
            OverrideExpiry::TurnCount(5),
        ));
        stack.push(make_entry(
            20,
            FieldOverrides {
                verbosity: Some(70),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));

        assert_eq!(stack.len(), 2);

        // At turn 4, the first entry is still valid.
        stack.expire_stale(4);
        assert_eq!(stack.len(), 2);

        // At turn 5, the first entry expires (TurnCount(5) means < 5).
        stack.expire_stale(5);
        assert_eq!(stack.len(), 1);
        let eff = stack.effective_overrides();
        assert_eq!(eff.formality, None);
        assert_eq!(eff.verbosity, Some(70));
    }

    #[test]
    fn time_based_expiry() {
        let mut stack = PersonaOverrideStack::new();
        // Entry created 10 seconds ago with 5-second duration → already expired.
        let past = (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        stack.push(PersonaOverrideEntry {
            override_id: Uuid::new_v4().to_string(),
            source: OverrideSource::System,
            field_overrides: FieldOverrides {
                formality: Some(99),
                ..Default::default()
            },
            expiry: OverrideExpiry::Duration(5),
            created_at: past,
            priority: 10,
        });
        // Entry with indefinite expiry.
        stack.push(make_entry(
            20,
            FieldOverrides {
                verbosity: Some(42),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));

        assert_eq!(stack.len(), 2);
        stack.expire_by_time();
        assert_eq!(stack.len(), 1);
        let eff = stack.effective_overrides();
        assert_eq!(eff.formality, None);
        assert_eq!(eff.verbosity, Some(42));
    }

    #[test]
    fn remove_by_id() {
        let mut stack = PersonaOverrideStack::new();
        let entry = make_entry(
            10,
            FieldOverrides {
                formality: Some(50),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        );
        let id = entry.override_id.clone();
        stack.push(entry);
        assert_eq!(stack.len(), 1);

        assert!(stack.remove(&id));
        assert!(stack.is_empty());
        // Removing non-existent ID returns false.
        assert!(!stack.remove("non-existent-id"));
    }

    #[test]
    fn field_clamping_temperature() {
        let mut stack = PersonaOverrideStack::new();
        stack.push(make_entry(
            10,
            FieldOverrides {
                temperature_delta: Some(0.50), // exceeds 0.20
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));
        let eff = stack.effective_overrides();
        assert!((eff.temperature_delta.unwrap() - 0.20).abs() < f64::EPSILON);
    }

    #[test]
    fn field_clamping_negative_temperature() {
        let mut stack = PersonaOverrideStack::new();
        stack.push(make_entry(
            10,
            FieldOverrides {
                temperature_delta: Some(-0.99),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));
        let eff = stack.effective_overrides();
        assert!((eff.temperature_delta.unwrap() - (-0.20)).abs() < f64::EPSILON);
    }

    #[test]
    fn field_clamping_nan_temperature_defaults_to_zero() {
        let mut stack = PersonaOverrideStack::new();
        stack.push(make_entry(
            10,
            FieldOverrides {
                temperature_delta: Some(f64::NAN),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));

        let eff = stack.effective_overrides();
        assert_eq!(eff.temperature_delta, Some(0.0));
    }

    #[test]
    fn malformed_duration_override_expires_instead_of_persisting_forever() {
        let mut stack = PersonaOverrideStack::new();
        stack.push(PersonaOverrideEntry {
            override_id: Uuid::new_v4().to_string(),
            source: OverrideSource::Api,
            field_overrides: FieldOverrides {
                formality: Some(99),
                ..Default::default()
            },
            expiry: OverrideExpiry::Duration(3600),
            created_at: "not-rfc3339".to_string(),
            priority: 10,
        });

        stack.expire_by_time();
        assert!(stack.is_empty());
    }

    #[test]
    fn field_clamping_formality_verbosity() {
        let mut stack = PersonaOverrideStack::new();
        // u8 can't exceed 255, but we clamp to 100.
        stack.push(make_entry(
            10,
            FieldOverrides {
                formality: Some(200),
                verbosity: Some(150),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));
        let eff = stack.effective_overrides();
        assert_eq!(eff.formality, Some(100));
        assert_eq!(eff.verbosity, Some(100));
    }

    #[test]
    fn merge_sparse_overrides() {
        let mut stack = PersonaOverrideStack::new();
        // Entry 1 (priority 5): sets formality and reasoning_strategy.
        stack.push(make_entry(
            5,
            FieldOverrides {
                formality: Some(40),
                reasoning_strategy: Some("chain-of-thought".into()),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));
        // Entry 2 (priority 10): sets verbosity and extra_system_prompt.
        stack.push(make_entry(
            10,
            FieldOverrides {
                verbosity: Some(75),
                extra_system_prompt: Some("Be concise.".into()),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));
        // Entry 3 (priority 3): sets current_objective and temperature_delta.
        stack.push(make_entry(
            3,
            FieldOverrides {
                current_objective: Some("help user".into()),
                temperature_delta: Some(0.05),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
        ));

        let eff = stack.effective_overrides();
        // All non-overlapping fields should be present.
        assert_eq!(eff.formality, Some(40));
        assert_eq!(eff.verbosity, Some(75));
        assert_eq!(eff.reasoning_strategy.as_deref(), Some("chain-of-thought"));
        assert_eq!(eff.extra_system_prompt.as_deref(), Some("Be concise."));
        assert_eq!(eff.current_objective.as_deref(), Some("help user"));
        assert!((eff.temperature_delta.unwrap() - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn serde_roundtrip_stack() {
        let mut stack = PersonaOverrideStack::new();
        stack.push(make_entry(
            10,
            FieldOverrides {
                formality: Some(55),
                verbosity: Some(30),
                temperature_delta: Some(-0.10),
                current_objective: Some("roundtrip test".into()),
                reasoning_strategy: Some("step-by-step".into()),
                extra_system_prompt: Some("Think carefully.".into()),
            },
            OverrideExpiry::TurnCount(10),
        ));
        stack.push(make_entry(
            200,
            FieldOverrides {
                formality: Some(99),
                ..Default::default()
            },
            OverrideExpiry::Duration(3600),
        ));

        let json = serde_json::to_string_pretty(&stack).expect("serialize");
        let restored: PersonaOverrideStack = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.len(), 2);
        // Verify field values survived roundtrip.
        let entries = restored.active_entries();
        assert_eq!(entries[0].priority, 10);
        assert_eq!(entries[0].field_overrides.formality, Some(55));
        assert_eq!(entries[0].expiry, OverrideExpiry::TurnCount(10));
        assert_eq!(entries[1].priority, 200);
        assert_eq!(entries[1].field_overrides.formality, Some(99));
        assert_eq!(entries[1].expiry, OverrideExpiry::Duration(3600));
    }

    #[test]
    fn serde_roundtrip_entry() {
        let entry = PersonaOverrideEntry::new(
            OverrideSource::Tool,
            FieldOverrides {
                extra_system_prompt: Some("hello".into()),
                ..Default::default()
            },
            OverrideExpiry::Indefinite,
            42,
        );
        let json = serde_json::to_string(&entry).expect("serialize");
        let restored: PersonaOverrideEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.source, OverrideSource::Tool);
        assert_eq!(restored.priority, 42);
        assert_eq!(
            restored.field_overrides.extra_system_prompt.as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn active_entries_returns_slice() {
        let mut stack = PersonaOverrideStack::new();
        assert!(stack.active_entries().is_empty());
        stack.push(make_entry(
            1,
            FieldOverrides::default(),
            OverrideExpiry::Indefinite,
        ));
        stack.push(make_entry(
            2,
            FieldOverrides::default(),
            OverrideExpiry::Indefinite,
        ));
        assert_eq!(stack.active_entries().len(), 2);
    }

    #[test]
    fn override_sources_all_variants() {
        // Ensure all variants serialize/deserialize correctly.
        for source in [
            OverrideSource::Api,
            OverrideSource::Config,
            OverrideSource::Tool,
            OverrideSource::System,
        ] {
            let json = serde_json::to_string(&source).expect("serialize");
            let restored: OverrideSource = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored, source);
        }
    }
}
