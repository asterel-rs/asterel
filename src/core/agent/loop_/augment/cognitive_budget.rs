//! Cognitive block budget allocator: implements priority-weighted
//! selection over augmentation blocks, ensuring the most relevant
//! cognitive signals fit within the available character budget.
//!
//! Instead of concatenating all non-empty blocks blindly, this module
//! treats each block as a knapsack item with a cost (character count)
//! and a situational utility score.  A greedy solver selects the
//! highest-utility blocks that fit, then orders them to mitigate the
//! "lost-in-the-middle" positional bias (high-priority items at the
//! top and bottom of the prompt, compressible items in the middle).
//!
//! References:
//! [ROI-REASONING] Zhao et al., 2026 — knapsack formulation for
//!   token budgeting across competing prompt sections.
//! [IGP] Song et al., 2026 — information-gain pruning; relevance ≠
//!   utility under budget constraints.
//! [LOST-MIDDLE] Liu et al., 2024 — positional bias in long-context
//!   LLMs; accuracy drops for middle-positioned content.
//! See the public research reference index in the docs site.
//!
//! ## Wiring status — augment
//!
//! **Wired (P-2, 2026-04-06):** `allocate_budget` reads `entry.kind` via
//! `is_anchor_kind()` for per-category guaranteed inclusion, and enforces
//! `budget.min_top_block_chars` as a reserved slot before the greedy pass.
//! `#[allow(dead_code)]` removed from `CognitiveBlockEntry::kind` and
//! `AugmentationBudget::min_top_block_chars`.
#![allow(clippy::cast_precision_loss)]

use super::policy::{DomainTag, SituationFeatures};
use crate::core::affect::AffectLabel;

// ── Block taxonomy ────────────────────────────────────────────────

/// Classification of each cognitive augmentation block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CognitiveBlockKind {
    /// Reasoning strategy directive (Standard/Stepwise/VerifyFirst/AskClarify).
    Reasoning,
    /// Memory grounding contract.
    Grounding,
    /// Taste-guided render contract.
    Taste,
    /// Affect state and response guidance.
    Affect,
    /// Affect causal attribution guidance.
    AffectCause,
    /// Retrieved past experience hints.
    Experience,
    /// Distilled principles from experience clusters.
    Principles,
    /// Attention schema focus block.
    Attention,
    /// Curiosity drive signal.
    Curiosity,
    /// Learned value guidance.
    Values,
    /// User mental model (`ToM`).
    UserModel,
    /// Integrated self/world/relationship model.
    IntegratedModel,
    /// Big Five personality guidance.
    BigFive,
    /// Unified behavior selector guidance.
    Behavior,
    /// Cognitive scaffolding JSON snapshot.
    Scaffolding,
    /// Desire-driven objective prefix.
    Desire,
    /// Affect topology: surfaced/suppressed emotion routes after diffusion.
    Topology,
    /// Session control: conversational mode, density, avoidance constraints.
    SessionControl,
}

/// Prompt placement zone for lost-in-the-middle mitigation.
///
/// High-priority blocks go to `Head` or `Tail`; compressible blocks
/// go to `Middle` where positional attention is weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptPlacement {
    /// Top of the augmentation section (highest attention).
    Head,
    /// Middle of the augmentation section (lowest attention).
    Middle,
    /// Bottom of the augmentation section (recency boost).
    Tail,
}

/// A cognitive block with its content and computed metadata.
#[derive(Debug, Clone)]
pub(crate) struct CognitiveBlockEntry {
    /// Block classification.
    pub kind: CognitiveBlockKind,
    /// Rendered text content.
    pub content: String,
    /// Character cost of the content.
    pub cost: usize,
    /// Computed utility score in `[0.0, 1.0]`.
    pub utility: f64,
    /// Prompt placement zone.
    pub placement: PromptPlacement,
}

/// Budget configuration for the augmentation section.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AugmentationBudget {
    /// Total character budget for all augmentation blocks combined.
    pub total_chars: usize,
    /// Minimum characters reserved for the highest-priority block.
    pub min_top_block_chars: usize,
}

impl Default for AugmentationBudget {
    fn default() -> Self {
        Self {
            total_chars: 8_000,
            min_top_block_chars: 200,
        }
    }
}

impl AugmentationBudget {
    /// Build a budget scaled to the model's context window capacity.
    #[must_use]
    pub(crate) fn for_model(model: &str) -> Self {
        let model = model.to_ascii_lowercase();
        let total_chars = if model.contains("claude-3-5")
            || model.contains("claude-4")
            || model.contains("gpt-4o")
            || model.contains("gemini")
        {
            12_000
        } else if model.contains("claude-3") || model.contains("gpt-4") {
            8_000
        } else {
            4_000
        };
        Self {
            total_chars,
            min_top_block_chars: 200,
        }
    }
}

// ── Priority computation ─────────────────────────────────────────

/// Base priority for each block kind (higher = more important).
///
/// These represent the intrinsic importance of each cognitive signal
/// independent of the current situation.  Dynamic modulation is
/// applied on top by [`compute_dynamic_utility`].
fn base_priority(kind: CognitiveBlockKind) -> f64 {
    match kind {
        CognitiveBlockKind::Reasoning => 0.95,
        CognitiveBlockKind::BigFive => 0.90,
        CognitiveBlockKind::Behavior => 0.89,
        CognitiveBlockKind::Scaffolding => 0.88,
        CognitiveBlockKind::Affect => 0.85,
        CognitiveBlockKind::Desire => 0.82,
        CognitiveBlockKind::AffectCause => 0.80,
        CognitiveBlockKind::Grounding => 0.75,
        CognitiveBlockKind::Principles => 0.70,
        CognitiveBlockKind::UserModel => 0.65,
        CognitiveBlockKind::Experience => 0.60,
        CognitiveBlockKind::IntegratedModel => 0.55,
        CognitiveBlockKind::Attention => 0.50,
        CognitiveBlockKind::Curiosity => 0.45,
        CognitiveBlockKind::Values => 0.40,
        CognitiveBlockKind::Topology => 0.87,
        CognitiveBlockKind::SessionControl => 0.86,
        CognitiveBlockKind::Taste => 0.35,
    }
}

/// Preferred prompt placement zone for each block kind.
///
/// Reasoning, personality, and affect go to `Head` (highest attention).
/// Memory and experience go to `Middle` (compressible).
/// User model and integrated model go to `Tail` (recency boost for
/// the model's immediate response planning).
fn preferred_placement(kind: CognitiveBlockKind) -> PromptPlacement {
    match kind {
        // Head: identity and strategy signals (always visible)
        CognitiveBlockKind::Reasoning
        | CognitiveBlockKind::BigFive
        | CognitiveBlockKind::Behavior
        | CognitiveBlockKind::Affect
        | CognitiveBlockKind::AffectCause
        | CognitiveBlockKind::Topology
        | CognitiveBlockKind::SessionControl
        | CognitiveBlockKind::Scaffolding
        | CognitiveBlockKind::Desire => PromptPlacement::Head,

        // Middle: retrieved/compressible signals
        CognitiveBlockKind::Grounding
        | CognitiveBlockKind::Experience
        | CognitiveBlockKind::Principles
        | CognitiveBlockKind::Attention
        | CognitiveBlockKind::Curiosity
        | CognitiveBlockKind::Values
        | CognitiveBlockKind::Taste => PromptPlacement::Middle,

        // Tail: user-facing and planning signals (recency)
        CognitiveBlockKind::UserModel | CognitiveBlockKind::IntegratedModel => {
            PromptPlacement::Tail
        }
    }
}

/// Compute a situation-aware utility score for a block.
///
/// The utility combines the block's base priority with situational
/// modifiers derived from the current turn's domain, affect state,
/// and complexity.
#[must_use]
pub(crate) fn compute_dynamic_utility(
    kind: CognitiveBlockKind,
    situation: &SituationFeatures,
) -> f64 {
    let base = base_priority(kind);
    let modifier = situational_modifier(kind, situation);
    (base + modifier).clamp(0.0, 1.0)
}

/// Situation-dependent modifier for each block kind.
fn situational_modifier(kind: CognitiveBlockKind, situation: &SituationFeatures) -> f64 {
    let domain = situation.domain;
    let affect = situation.affect_label;
    let complexity = f64::from(situation.complexity);

    match kind {
        // Affect blocks are more important when emotion is strong.
        CognitiveBlockKind::Affect
        | CognitiveBlockKind::AffectCause
        | CognitiveBlockKind::Topology
        | CognitiveBlockKind::SessionControl => {
            if is_emotional_affect(affect) {
                0.10
            } else {
                -0.15
            }
        }

        // Experience and principles matter more for complex tasks.
        CognitiveBlockKind::Experience | CognitiveBlockKind::Principles => {
            (complexity - 0.5) * 0.15
        }

        // Curiosity is boosted for creative and personal domains.
        CognitiveBlockKind::Curiosity => match domain {
            DomainTag::Creative | DomainTag::Personal => 0.10,
            _ => -0.05,
        },

        // User model is more important for personal interactions.
        CognitiveBlockKind::UserModel => match domain {
            DomainTag::Personal => 0.15,
            DomainTag::Creative => 0.05,
            _ => 0.0,
        },

        // Technical domains boost grounding (factual recall).
        CognitiveBlockKind::Grounding => match domain {
            DomainTag::Technical => 0.10,
            _ => 0.0,
        },

        // Reasoning strategy is more important for complex tasks.
        CognitiveBlockKind::Reasoning => (complexity - 0.5) * 0.10,

        _ => 0.0,
    }
}

fn is_emotional_affect(affect: AffectLabel) -> bool {
    !matches!(affect, AffectLabel::Neutral)
}

// ── Budget allocation (greedy knapsack) ──────────────────────────

/// Select blocks that fit within the budget, maximizing total utility.
///
/// Uses a greedy algorithm: sort by utility density (utility / cost),
/// then greedily include blocks until the budget is exhausted.
/// Blocks with zero cost or empty content are silently skipped.
///
/// Kind-aware minimum reservation (P-2): if `budget.min_top_block_chars > 0`,
/// the first anchor-kind block (`Reasoning`, `BigFive`, `Affect`, `Scaffolding`)
/// is guaranteed inclusion when its cost fits within that reservation, even if
/// normal density-ordering would not select it first.  When
/// `min_top_block_chars == 0` this path is a no-op and behavior is unchanged.
pub(crate) fn allocate_budget(
    mut entries: Vec<CognitiveBlockEntry>,
    budget: &AugmentationBudget,
) -> Vec<CognitiveBlockEntry> {
    entries.retain(|entry| entry.cost > 0 && !entry.content.is_empty());
    if entries.is_empty() {
        return Vec::new();
    }

    // Sort by utility density (utility / cost), descending.
    entries.sort_by(|a, b| {
        let density_a = a.utility / (a.cost.max(1) as f64);
        let density_b = b.utility / (b.cost.max(1) as f64);
        density_b
            .partial_cmp(&density_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut selected = Vec::with_capacity(entries.len());
    let mut remaining_budget = budget.total_chars;

    for entry in entries {
        // Kind-aware minimum reservation: anchor kinds get guaranteed inclusion
        // when they fit within min_top_block_chars and no block has been selected
        // yet.  This prevents many small low-density blocks from exhausting the
        // budget before the highest-priority cognitive signal is included.
        let fits_in_reservation = budget.min_top_block_chars > 0
            && selected.is_empty()
            && is_anchor_kind(entry.kind)
            && entry.cost <= budget.min_top_block_chars;

        if fits_in_reservation || entry.cost <= remaining_budget {
            remaining_budget = remaining_budget.saturating_sub(entry.cost);
            selected.push(entry);
        }
    }

    selected
}

/// Returns `true` for high-priority block kinds that qualify for the
/// `min_top_block_chars` guaranteed-inclusion reservation in `allocate_budget`.
fn is_anchor_kind(kind: CognitiveBlockKind) -> bool {
    matches!(
        kind,
        CognitiveBlockKind::Reasoning
            | CognitiveBlockKind::BigFive
            | CognitiveBlockKind::Behavior
            | CognitiveBlockKind::Affect
            | CognitiveBlockKind::Scaffolding
    )
}

// ── Convenience: build entries from TurnAugmentations ────────────

/// Build a list of cognitive block entries from the augmentation
/// fields, computing dynamic utility for each.
pub(crate) fn entries_from_augmentations(
    augmentations: &super::types::TurnAugmentations,
) -> Vec<CognitiveBlockEntry> {
    let situation = &augmentations.situation;

    let candidates: &[(CognitiveBlockKind, &str)] = &[
        (
            CognitiveBlockKind::Reasoning,
            &augmentations.reasoning_block,
        ),
        (
            CognitiveBlockKind::Grounding,
            &augmentations.grounding_block,
        ),
        (CognitiveBlockKind::Taste, &augmentations.taste_block),
        (CognitiveBlockKind::Affect, &augmentations.affect_block),
        (
            CognitiveBlockKind::AffectCause,
            &augmentations.cause_guidance_block,
        ),
        (
            CognitiveBlockKind::Experience,
            &augmentations.experience_block,
        ),
        (
            CognitiveBlockKind::Principles,
            &augmentations.principle_block,
        ),
        (
            CognitiveBlockKind::Attention,
            &augmentations.attention_block,
        ),
        (
            CognitiveBlockKind::Curiosity,
            &augmentations.curiosity_block,
        ),
        (CognitiveBlockKind::Values, &augmentations.value_block),
        (
            CognitiveBlockKind::UserModel,
            &augmentations.user_model_block,
        ),
        (
            CognitiveBlockKind::IntegratedModel,
            &augmentations.integrated_model_block,
        ),
        (CognitiveBlockKind::BigFive, &augmentations.big_five_block),
        (CognitiveBlockKind::Behavior, &augmentations.behavior_block),
        (
            CognitiveBlockKind::Scaffolding,
            &augmentations.scaffolding_block,
        ),
        (CognitiveBlockKind::Desire, &augmentations.desire_block),
        (CognitiveBlockKind::Topology, &augmentations.topology_block),
        (
            CognitiveBlockKind::SessionControl,
            &augmentations.session_control_block,
        ),
    ];

    candidates
        .iter()
        .filter(|(_, content)| !content.is_empty())
        .map(|(kind, content)| {
            let utility = compute_dynamic_utility(*kind, situation);
            CognitiveBlockEntry {
                kind: *kind,
                content: (*content).to_string(),
                cost: content.len(),
                utility,
                placement: preferred_placement(*kind),
            }
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::affect::AffectLabel;
    use crate::core::agent::loop_::augment::policy::{DomainTag, SituationFeatures};

    fn test_situation() -> SituationFeatures {
        SituationFeatures {
            complexity: 0.5,
            affect_label: AffectLabel::Neutral,
            affect_intensity: 0.0,
            domain: DomainTag::General,
        }
    }

    fn make_entry(kind: CognitiveBlockKind, content: &str, utility: f64) -> CognitiveBlockEntry {
        CognitiveBlockEntry {
            kind,
            content: content.to_string(),
            cost: content.len(),
            utility,
            placement: preferred_placement(kind),
        }
    }

    #[test]
    fn allocate_budget_excludes_empty_blocks() {
        let entries = vec![
            make_entry(CognitiveBlockKind::Affect, "", 0.9),
            make_entry(CognitiveBlockKind::BigFive, "[Big Five]", 0.8),
        ];
        let budget = AugmentationBudget {
            total_chars: 1000,
            min_top_block_chars: 0,
        };
        let selected = allocate_budget(entries, &budget);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].kind, CognitiveBlockKind::BigFive);
    }

    #[test]
    fn allocate_budget_respects_total_budget() {
        let entries = vec![
            make_entry(CognitiveBlockKind::Affect, "A".repeat(600).as_str(), 0.9),
            make_entry(CognitiveBlockKind::BigFive, "B".repeat(600).as_str(), 0.8),
        ];
        let budget = AugmentationBudget {
            total_chars: 800,
            min_top_block_chars: 0,
        };
        let selected = allocate_budget(entries, &budget);
        // Only one fits (600 chars each, budget 800, second would exceed).
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].kind, CognitiveBlockKind::Affect);
    }

    #[test]
    fn allocate_budget_prefers_high_density() {
        // Small but high utility vs large but lower utility.
        let entries = vec![
            make_entry(CognitiveBlockKind::Grounding, "G".repeat(500).as_str(), 0.5),
            make_entry(CognitiveBlockKind::Affect, "A".repeat(50).as_str(), 0.8),
        ];
        let budget = AugmentationBudget {
            total_chars: 600,
            min_top_block_chars: 0,
        };
        let selected = allocate_budget(entries, &budget);
        // Affect has higher density (0.8/50 vs 0.5/500), selected first.
        // Then Grounding fits within remaining 550.
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].kind, CognitiveBlockKind::Affect);
        assert_eq!(selected[1].kind, CognitiveBlockKind::Grounding);
    }

    #[test]
    fn render_budgeted_orders_by_placement() {
        let selected = vec![
            make_entry(CognitiveBlockKind::UserModel, "[User]", 0.6),
            make_entry(CognitiveBlockKind::Reasoning, "[Reason]", 0.9),
            make_entry(CognitiveBlockKind::Grounding, "[Ground]", 0.7),
        ];
        let rendered = crate::core::agent::presenter::render_budgeted(selected);
        let lines: Vec<&str> = rendered.split("\n\n").collect();
        // Head: Reasoning, Middle: Grounding, Tail: UserModel.
        assert_eq!(lines[0], "[Reason]");
        assert_eq!(lines[1], "[Ground]");
        assert_eq!(lines[2], "[User]");
    }

    #[test]
    fn dynamic_utility_boosts_affect_for_emotional_state() {
        let neutral = SituationFeatures {
            affect_label: AffectLabel::Neutral,
            ..test_situation()
        };
        let frustrated = SituationFeatures {
            affect_label: AffectLabel::Frustrated,
            ..test_situation()
        };
        let util_neutral = compute_dynamic_utility(CognitiveBlockKind::Affect, &neutral);
        let util_frustrated = compute_dynamic_utility(CognitiveBlockKind::Affect, &frustrated);
        assert!(
            util_frustrated > util_neutral,
            "affect utility should be higher when user is emotional: \
             frustrated={util_frustrated}, neutral={util_neutral}"
        );
    }

    #[test]
    fn dynamic_utility_boosts_grounding_for_technical() {
        let general = SituationFeatures {
            domain: DomainTag::General,
            ..test_situation()
        };
        let technical = SituationFeatures {
            domain: DomainTag::Technical,
            ..test_situation()
        };
        let util_general = compute_dynamic_utility(CognitiveBlockKind::Grounding, &general);
        let util_technical = compute_dynamic_utility(CognitiveBlockKind::Grounding, &technical);
        assert!(
            util_technical > util_general,
            "grounding utility should be higher for technical domain: \
             technical={util_technical}, general={util_general}"
        );
    }

    #[test]
    fn empty_augmentations_produce_empty_render() {
        let aug = super::super::types::TurnAugmentations::default();
        let budget = AugmentationBudget::default();
        let rendered = crate::core::agent::presenter::render_augmentations_budgeted(&aug, &budget);
        assert!(rendered.is_empty());
    }

    #[test]
    fn augmentation_budget_for_model_scales() {
        let large = AugmentationBudget::for_model("claude-4-sonnet");
        let small = AugmentationBudget::for_model("unknown-model");
        assert!(
            large.total_chars > small.total_chars,
            "large model should get bigger budget: large={}, small={}",
            large.total_chars,
            small.total_chars
        );
    }

    #[test]
    fn base_priorities_are_bounded() {
        let kinds = [
            CognitiveBlockKind::Reasoning,
            CognitiveBlockKind::BigFive,
            CognitiveBlockKind::Affect,
            CognitiveBlockKind::AffectCause,
            CognitiveBlockKind::Grounding,
            CognitiveBlockKind::Principles,
            CognitiveBlockKind::UserModel,
            CognitiveBlockKind::Experience,
            CognitiveBlockKind::IntegratedModel,
            CognitiveBlockKind::Attention,
            CognitiveBlockKind::Curiosity,
            CognitiveBlockKind::Values,
            CognitiveBlockKind::Taste,
            CognitiveBlockKind::Scaffolding,
            CognitiveBlockKind::Desire,
        ];
        for kind in kinds {
            let priority = base_priority(kind);
            assert!(
                (0.0..=1.0).contains(&priority),
                "{kind:?} has out-of-bounds priority: {priority}"
            );
        }
    }
}
