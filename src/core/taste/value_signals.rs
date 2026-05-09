//! Value signal extraction from user messages and interaction
//! outcomes (brevity, detail, caution, autonomy, structure,
//! informality).

use serde::{Deserialize, Serialize};

/// A value signal extracted from user behaviour during a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // Prefers prefix is intentional for domain clarity
pub(crate) enum ValueSignal {
    /// User prefers concise responses ("too long", "shorter").
    PrefersBrevity,
    /// User prefers detailed responses ("more detail", "elaborate").
    PrefersDetail,
    /// User prefers caution (denied tool approval, "be careful").
    PrefersCaution,
    /// User prefers autonomy (approved quickly, "just do it").
    PrefersAutonomy,
    /// User prefers structured output ("list", "steps", "table").
    PrefersStructure,
    /// User prefers informal tone ("no need to be formal").
    PrefersInformal,
}

/// Extract value signals from user message and outcome context.
pub(crate) fn extract_value_signals(
    user_message: &str,
    assistant_answer: &str,
    tool_approval_denied: bool,
) -> Vec<ValueSignal> {
    let mut signals = Vec::new();
    let lower_msg = user_message.to_lowercase();

    // Brevity signals.
    if lower_msg.contains("too long")
        || lower_msg.contains("shorter")
        || lower_msg.contains("brief")
        || lower_msg.contains("concise")
    {
        signals.push(ValueSignal::PrefersBrevity);
    }

    // Detail signals.
    if lower_msg.contains("more detail")
        || lower_msg.contains("elaborate")
        || lower_msg.contains("explain more")
        || lower_msg.contains("go deeper")
    {
        signals.push(ValueSignal::PrefersDetail);
    }

    // Caution signals.
    if tool_approval_denied
        || lower_msg.contains("be careful")
        || lower_msg.contains("don't do that")
        || lower_msg.contains("wait")
    {
        signals.push(ValueSignal::PrefersCaution);
    }

    // Autonomy signals.
    if lower_msg.contains("just do it")
        || lower_msg.contains("go ahead")
        || lower_msg.contains("do whatever")
    {
        signals.push(ValueSignal::PrefersAutonomy);
    }

    // Structure signals.
    if lower_msg.contains("list")
        || lower_msg.contains("steps")
        || lower_msg.contains("table")
        || lower_msg.contains("bullet")
    {
        signals.push(ValueSignal::PrefersStructure);
    }

    // Informal signals.
    if lower_msg.contains("no need to be formal")
        || lower_msg.contains("casual")
        || lower_msg.contains("chill")
    {
        signals.push(ValueSignal::PrefersInformal);
    }

    // Response length as implicit signal.
    let _ = assistant_answer; // reserved for future analysis

    signals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_brevity_preference() {
        let signals = extract_value_signals("That was too long, be more concise", "", false);
        assert!(signals.contains(&ValueSignal::PrefersBrevity));
    }

    #[test]
    fn detects_caution_on_denied_approval() {
        let signals = extract_value_signals("ok", "", true);
        assert!(signals.contains(&ValueSignal::PrefersCaution));
    }

    #[test]
    fn detects_autonomy_preference() {
        let signals = extract_value_signals("just do it", "", false);
        assert!(signals.contains(&ValueSignal::PrefersAutonomy));
    }

    #[test]
    fn detects_structure_preference() {
        let signals = extract_value_signals("give me a list of steps", "", false);
        assert!(signals.contains(&ValueSignal::PrefersStructure));
    }

    #[test]
    fn no_signals_on_neutral_message() {
        let signals = extract_value_signals("What is the weather?", "", false);
        assert!(signals.is_empty());
    }
}
