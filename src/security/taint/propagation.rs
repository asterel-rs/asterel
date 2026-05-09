//! Taint propagation for the tool pipeline.
//!
//! Implements a monotone-increasing taint lattice: once a [`TaintLabel`] is
//! attached to a piece of data, it can never be removed.  Labels accumulate
//! as data flows through tools — union is the only merge operation; there is
//! no downgrade path.  This mirrors Denning's 1976 lattice model of secure
//! information flow (see the public research reference index §TAINT-LATTICE).
//!
//! # Why taint propagation exists
//!
//! Downstream consumers (logging, persistence, output routing) need to know
//! the sensitivity of data without inspecting its content.  A file written
//! with a command that incorporated user input should carry the `UserInput`
//! taint so the audit log can flag it.  A summarisation of a web page should
//! carry `ExternalNetwork` so the output sanitiser knows to apply extra
//! scrutiny before the result reaches the model context.
//!
//! The monotone guarantee means no tool can "launder" tainted data.  If a
//! shell command is invoked with an argument derived from a web fetch, the
//! shell's output carries `ExternalNetwork` regardless of what the command
//! actually does.
//!
//! # Automatic network-boundary tainting
//!
//! [`propagate`] adds `ExternalNetwork` unconditionally to the output of any
//! tool in the [`NETWORK_TOOLS`] list, and to any tool whose name starts with
//! `mcp_`.  This covers the case where no tainted input was passed but the
//! tool itself crosses the network trust boundary (e.g. a bare `web_fetch`
//! call with a static URL).  The security guarantee is:
//!
//! > *Every byte that crossed a network boundary is labelled `ExternalNetwork`
//! > before it enters the pipeline, regardless of what triggered the fetch.*
//!
//! # Default propagation rules
//!
//! Each [`TaintPropagationRule`] maps one input label to one output label.
//! All five current labels propagate identity (`ExternalNetwork →
//! ExternalNetwork`, etc.) — there are no cross-label upgrade rules today.
//! Future rules (e.g. `UserInput + shell → EscalationRisk`) would be added
//! here and slot naturally into the [`propagate`] loop.
//!
//! [`TaintLabel`]: super::label::TaintLabel

use super::label::{TaintLabel, TaintSet};

/// A rule mapping an input taint label to an output taint label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaintPropagationRule {
    /// If the input carries this label...
    pub input_label: TaintLabel,
    /// ...the output receives this label.
    pub output_label: TaintLabel,
}

/// Network-boundary tool names that automatically taint output.
const NETWORK_TOOLS: &[&str] = &[
    "browser",
    "browser_open",
    "web_fetch",
    "web_search",
    "web_scrape",
    "websearch",
    "duckduckgo_search",
    "composio",
];

/// Returns the default propagation rules.
///
/// - `ExternalNetwork` propagates to `ExternalNetwork`
/// - `UserInput` propagates to `UserInput`
/// - `Pii` propagates to `Pii`
/// - `Secret` propagates to `Secret`
/// - `UntrustedAgent` propagates to `UntrustedAgent`
#[must_use]
pub fn default_rules() -> Vec<TaintPropagationRule> {
    vec![
        TaintPropagationRule {
            input_label: TaintLabel::ExternalNetwork,
            output_label: TaintLabel::ExternalNetwork,
        },
        TaintPropagationRule {
            input_label: TaintLabel::UserInput,
            output_label: TaintLabel::UserInput,
        },
        TaintPropagationRule {
            input_label: TaintLabel::Pii,
            output_label: TaintLabel::Pii,
        },
        TaintPropagationRule {
            input_label: TaintLabel::Secret,
            output_label: TaintLabel::Secret,
        },
        TaintPropagationRule {
            input_label: TaintLabel::UntrustedAgent,
            output_label: TaintLabel::UntrustedAgent,
        },
    ]
}

/// Propagate taint labels from input to output, applying default rules
/// and tool-specific automatic tainting.
///
/// Network-boundary tools automatically receive an `ExternalNetwork`
/// taint on their output. MCP tools (prefixed with `mcp_`) are also
/// treated as network-boundary.
#[must_use]
pub fn propagate(input_taints: &TaintSet, tool_name: &str) -> TaintSet {
    let rules = default_rules();
    let mut output = TaintSet::new();

    // Apply propagation rules based on input taints.
    for rule in &rules {
        if input_taints.contains(&rule.input_label) {
            output.insert(rule.output_label);
        }
    }

    // Auto-taint network-boundary tools.
    let is_network = NETWORK_TOOLS.contains(&tool_name) || tool_name.starts_with("mcp_");
    if is_network {
        output.insert(TaintLabel::ExternalNetwork);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagate_empty_input_no_network_tool() {
        let input = TaintSet::new();
        let output = propagate(&input, "file_read");
        assert!(output.is_empty());
    }

    #[test]
    fn propagate_empty_input_network_tool_adds_external() {
        let input = TaintSet::new();
        let output = propagate(&input, "web_fetch");
        assert!(output.contains(&TaintLabel::ExternalNetwork));
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn propagate_mcp_tool_adds_external() {
        let input = TaintSet::new();
        let output = propagate(&input, "mcp_filesystem_list");
        assert!(output.contains(&TaintLabel::ExternalNetwork));
    }

    #[test]
    fn propagate_user_input_carries_through() {
        let input = TaintSet::from_labels([TaintLabel::UserInput]);
        let output = propagate(&input, "file_write");
        assert!(output.contains(&TaintLabel::UserInput));
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn propagate_multiple_labels() {
        let input = TaintSet::from_labels([TaintLabel::Pii, TaintLabel::Secret]);
        let output = propagate(&input, "shell");
        assert!(output.contains(&TaintLabel::Pii));
        assert!(output.contains(&TaintLabel::Secret));
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn propagate_network_tool_with_existing_taints() {
        let input = TaintSet::from_labels([TaintLabel::UserInput]);
        let output = propagate(&input, "browser");
        assert!(output.contains(&TaintLabel::UserInput));
        assert!(output.contains(&TaintLabel::ExternalNetwork));
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn default_rules_cover_all_labels() {
        let rules = default_rules();
        assert_eq!(rules.len(), 5);
        let input_labels: Vec<_> = rules.iter().map(|r| r.input_label).collect();
        assert!(input_labels.contains(&TaintLabel::ExternalNetwork));
        assert!(input_labels.contains(&TaintLabel::UserInput));
        assert!(input_labels.contains(&TaintLabel::Pii));
        assert!(input_labels.contains(&TaintLabel::Secret));
        assert!(input_labels.contains(&TaintLabel::UntrustedAgent));
    }

    #[test]
    fn propagate_untrusted_agent() {
        let input = TaintSet::from_labels([TaintLabel::UntrustedAgent]);
        let output = propagate(&input, "delegate");
        assert!(output.contains(&TaintLabel::UntrustedAgent));
    }
}
