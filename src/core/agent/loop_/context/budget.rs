#[derive(Debug, Clone, Copy)]
pub struct ContextBudget {
    /// Total character budget for the assembled context.
    pub total_chars: usize,
    /// Budget for conversation state block.
    pub state_chars: usize,
    /// Budget for fact ledger block.
    pub ledger_chars: usize,
    /// Budget for runtime metadata block.
    pub runtime_metadata_chars: usize,
    /// Budget for recalled memory block.
    pub memory_chars: usize,
    /// Max items in the ledger context.
    pub ledger_max_items: usize,
    /// Max chars per individual memory entry value.
    pub entry_value_max_chars: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            total_chars: 6_000,
            state_chars: 1_320,
            ledger_chars: 1_560,
            runtime_metadata_chars: 480,
            memory_chars: 2_640,
            ledger_max_items: 8,
            entry_value_max_chars: 220,
        }
    }
}

#[must_use]
pub fn context_budget_for_model(model: &str) -> ContextBudget {
    let model = model.to_ascii_lowercase();

    let (total_chars, ledger_max_items, entry_value_max_chars): (usize, usize, usize) = if model
        .contains("claude-3-5")
        || model.contains("claude-4")
        || model.contains("gpt-4o")
        || model.contains("gemini")
    {
        (24_000, 16, 400)
    } else if model.contains("claude-3") || model.contains("gpt-4") {
        (12_000, 12, 300)
    } else {
        (6_000, 8, 220)
    };

    let state_chars = total_chars * 22 / 100;
    let ledger_chars = total_chars * 26 / 100;
    let runtime_metadata_chars = total_chars * 8 / 100;
    let used = state_chars + ledger_chars + runtime_metadata_chars;
    let memory_chars = total_chars.saturating_sub(used);

    ContextBudget {
        total_chars,
        state_chars,
        ledger_chars,
        runtime_metadata_chars,
        memory_chars,
        ledger_max_items,
        entry_value_max_chars,
    }
}

#[must_use]
pub(super) fn split_memory_fragment_budgets(
    budget: &ContextBudget,
    has_memory_block: bool,
    has_untrusted_block: bool,
) -> (usize, usize) {
    match (has_memory_block, has_untrusted_block) {
        (true, true) => (budget.memory_chars * 2 / 3, budget.memory_chars / 3),
        (true, false) => (budget.memory_chars, 0),
        (false, true) => (0, budget.memory_chars),
        (false, false) => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_memory_budget_prefers_trusted_memory_when_both_blocks_exist() {
        let budget = ContextBudget::default();
        let (memory_chars, untrusted_chars) = split_memory_fragment_budgets(&budget, true, true);

        assert_eq!(memory_chars, budget.memory_chars * 2 / 3);
        assert_eq!(untrusted_chars, budget.memory_chars / 3);
    }

    #[test]
    fn split_memory_budget_assigns_all_budget_to_single_present_block() {
        let budget = ContextBudget::default();
        assert_eq!(
            split_memory_fragment_budgets(&budget, true, false),
            (budget.memory_chars, 0)
        );
        assert_eq!(
            split_memory_fragment_budgets(&budget, false, true),
            (0, budget.memory_chars)
        );
    }
}
