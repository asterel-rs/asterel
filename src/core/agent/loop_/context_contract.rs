//! Typed context-contract primitives for agent turns.
//!
//! These types separate the logical context fragments from their
//! final rendered prompt representation so the turn loop can evolve
//! toward diff-based and policy-aware context injection.

use crate::utils::text::truncate_ellipsis;

/// High-level kind of context fragment included in a turn contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextFragmentKind {
    /// Stable base instructions that change rarely across turns.
    BaseInstructions,
    /// Short-lived conversation state such as goals and open loops.
    ConversationState,
    /// Prior transcript or compacted turn history.
    History,
    /// Stable or semi-stable fact ledger entries.
    FactLedger,
    /// Recalled long-term memory material.
    Memory,
    /// Runtime metadata such as channel or environment state.
    RuntimeMetadata,
    /// Sanitized or labeled untrusted content.
    UntrustedContent,
}

/// Trust label for a rendered context fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextFragmentTrust {
    /// Fully trusted internal context.
    Trusted,
    /// Content that originated outside the trust boundary and has
    /// been sanitized or reduced to safe placeholders.
    SanitizedUntrusted,
}

/// Whether a context update should send the full seed or only changed fragments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextUpdateMode {
    /// Send the full contract, typically for the first turn or after resume.
    FullSeed,
    /// Send only changed fragments relative to a previous contract.
    Delta,
}

/// One logical context fragment inside a turn contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFragment {
    /// The semantic kind of fragment.
    pub kind: ContextFragmentKind,
    /// Trust label carried into the contract.
    pub trust: ContextFragmentTrust,
    /// Character budget applied to this fragment.
    pub budget_chars: usize,
    /// Rendered content for the fragment.
    pub content: String,
}

impl ContextFragment {
    /// Create a new fragment and clip it to the configured budget.
    #[must_use]
    pub fn new(
        kind: ContextFragmentKind,
        trust: ContextFragmentTrust,
        budget_chars: usize,
        content: impl Into<String>,
    ) -> Option<Self> {
        let content = truncate_ellipsis(content.into().trim(), budget_chars);
        if content.is_empty() {
            return None;
        }

        Some(Self {
            kind,
            trust,
            budget_chars,
            content,
        })
    }
}

/// Typed representation of the context assembled for a single turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnContextContract {
    /// Total budget applied to the fully rendered contract.
    pub total_budget_chars: usize,
    /// Ordered context fragments included in the turn.
    pub fragments: Vec<ContextFragment>,
}

/// A rendered update derived from a context contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnContextUpdate {
    /// Whether this update is a full seed or delta.
    pub mode: ContextUpdateMode,
    /// Total budget inherited from the originating contract.
    pub total_budget_chars: usize,
    /// Ordered fragments included in this update.
    pub fragments: Vec<ContextFragment>,
}

impl TurnContextContract {
    /// Create an empty context contract with the given total budget.
    #[must_use]
    pub fn new(total_budget_chars: usize) -> Self {
        Self {
            total_budget_chars,
            fragments: Vec::new(),
        }
    }

    /// Append a fragment to the contract.
    pub fn push(&mut self, fragment: ContextFragment) {
        self.fragments.push(fragment);
    }

    /// Whether any fragment contains sanitized untrusted content.
    #[must_use]
    pub fn has_sanitized_untrusted_content(&self) -> bool {
        self.fragments
            .iter()
            .any(|fragment| fragment.trust == ContextFragmentTrust::SanitizedUntrusted)
    }

    /// Render the full contract into a flat string for prompt injection.
    #[must_use]
    pub fn render(&self) -> String {
        let mut rendered = String::new();
        for fragment in &self.fragments {
            if !rendered.is_empty() {
                rendered.push('\n');
            }
            rendered.push_str(&fragment.content);
        }

        if rendered.len() > self.total_budget_chars
            && rendered.chars().count() > self.total_budget_chars
        {
            rendered = clip_to_total_budget(&rendered, self.total_budget_chars);
        }
        if !rendered.is_empty() && !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered
    }

    /// Build a full-seed or delta update relative to a previous contract.
    #[must_use]
    pub fn diff_from(&self, previous: Option<&Self>) -> TurnContextUpdate {
        let Some(previous) = previous else {
            return TurnContextUpdate {
                mode: ContextUpdateMode::FullSeed,
                total_budget_chars: self.total_budget_chars,
                fragments: self.fragments.clone(),
            };
        };

        let fragments = self
            .fragments
            .iter()
            .filter(|fragment| {
                previous
                    .fragments
                    .iter()
                    .find(|candidate| candidate.kind == fragment.kind)
                    != Some(*fragment)
            })
            .cloned()
            .collect();

        TurnContextUpdate {
            mode: ContextUpdateMode::Delta,
            total_budget_chars: self.total_budget_chars,
            fragments,
        }
    }
}

fn clip_to_total_budget(input: &str, budget_chars: usize) -> String {
    if input.chars().count() <= budget_chars {
        return input.to_string();
    }

    if budget_chars == 0 {
        return String::new();
    }

    if budget_chars <= 3 {
        return ".".repeat(budget_chars);
    }

    let keep_chars = budget_chars - 3;
    let keep_until = input
        .char_indices()
        .nth(keep_chars)
        .map_or(input.len(), |(idx, _)| idx);
    format!("{}...", input[..keep_until].trim_end())
}

impl TurnContextUpdate {
    /// Render the update into a flat string for prompt injection.
    #[must_use]
    pub fn render(&self) -> String {
        TurnContextContract {
            total_budget_chars: self.total_budget_chars,
            fragments: self.fragments.clone(),
        }
        .render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_constructor_drops_empty_content() {
        let fragment = ContextFragment::new(
            ContextFragmentKind::ConversationState,
            ContextFragmentTrust::Trusted,
            100,
            "   ",
        );
        assert!(fragment.is_none());
    }

    #[test]
    fn contract_render_clips_to_total_budget() {
        let mut contract = TurnContextContract::new(20);
        contract.push(
            ContextFragment::new(
                ContextFragmentKind::ConversationState,
                ContextFragmentTrust::Trusted,
                20,
                "[Conversation state]\n- focus: finish the refactor",
            )
            .expect("fragment"),
        );
        let rendered = contract.render();
        assert!(rendered.chars().count() <= 21);
    }

    #[test]
    fn contract_detects_sanitized_untrusted_fragments() {
        let mut contract = TurnContextContract::new(200);
        contract.push(
            ContextFragment::new(
                ContextFragmentKind::UntrustedContent,
                ContextFragmentTrust::SanitizedUntrusted,
                120,
                "[Memory context]\n- external.note: [sanitized]",
            )
            .expect("fragment"),
        );
        assert!(contract.has_sanitized_untrusted_content());
    }

    #[test]
    fn diff_from_none_returns_full_seed() {
        let mut contract = TurnContextContract::new(200);
        contract.push(
            ContextFragment::new(
                ContextFragmentKind::RuntimeMetadata,
                ContextFragmentTrust::Trusted,
                80,
                "[Runtime metadata]\n- entity_id: person:test",
            )
            .expect("fragment"),
        );

        let update = contract.diff_from(None);
        assert_eq!(update.mode, ContextUpdateMode::FullSeed);
        assert_eq!(update.fragments.len(), 1);
    }

    #[test]
    fn diff_only_includes_changed_fragments() {
        let mut previous = TurnContextContract::new(200);
        previous.push(
            ContextFragment::new(
                ContextFragmentKind::ConversationState,
                ContextFragmentTrust::Trusted,
                80,
                "[Conversation state]\n- focus: old",
            )
            .expect("fragment"),
        );

        let mut current = TurnContextContract::new(200);
        current.push(
            ContextFragment::new(
                ContextFragmentKind::ConversationState,
                ContextFragmentTrust::Trusted,
                80,
                "[Conversation state]\n- focus: new",
            )
            .expect("fragment"),
        );
        current.push(
            ContextFragment::new(
                ContextFragmentKind::RuntimeMetadata,
                ContextFragmentTrust::Trusted,
                80,
                "[Runtime metadata]\n- entity_id: person:test",
            )
            .expect("fragment"),
        );

        let update = current.diff_from(Some(&previous));
        assert_eq!(update.mode, ContextUpdateMode::Delta);
        assert_eq!(update.fragments.len(), 2);
        assert!(update.render().contains("[Runtime metadata]"));
    }
}
