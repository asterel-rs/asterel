//! Judgment core: parses the agent's `SOUL.md` to extract a stable identity
//! summary, value list, and non-negotiables, which are then injected into
//! the system prompt each turn.
//!
//! Parsing strategy:
//! - Summary: first meaningful line under `## Core Summary`,
//!   `## What Drives Me (Agent)`, or `## What Drives Me`.
//! - Values: bullet list under `## What I Value` or `## Values`; falls back
//!   to `**bold phrases**` extracted from `## Core Truths`.
//! - Non-negotiables: bullet list under `## What I Won't Do`,
//!   `## What I Will Not Do`, or `## Non-Negotiables`.
//!
//! Custom entries are merged with `default_humanlike()` defaults via
//! normalised lowercase deduplication; custom items appear first.  When
//! `SOUL.md` is absent or sections are missing, defaults are used as-is.

use std::fmt::Write as FmtWrite;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JudgmentCore {
    pub summary: String,
    pub values: Vec<String>,
    pub non_negotiables: Vec<String>,
}

impl JudgmentCore {
    #[must_use]
    pub(crate) fn from_workspace(workspace_dir: &Path) -> Self {
        let soul_raw = std::fs::read_to_string(workspace_dir.join("SOUL.md")).unwrap_or_default();
        Self::from_soul_markdown(&soul_raw)
    }

    #[must_use]
    pub(crate) fn from_soul_markdown(soul_raw: &str) -> Self {
        let mut core = Self::default_humanlike();

        if let Some(summary) = extract_summary(soul_raw) {
            core.summary = summary;
        }

        let custom_values = extract_values(soul_raw);
        if !custom_values.is_empty() {
            core.values = merge_items(custom_values, &core.values);
        }

        let custom_non_negotiables = extract_non_negotiables(soul_raw);
        if !custom_non_negotiables.is_empty() {
            core.non_negotiables = merge_items(custom_non_negotiables, &core.non_negotiables);
        }

        core
    }

    #[must_use]
    pub(crate) fn default_humanlike() -> Self {
        Self {
            summary: "A grounded conversational presence who values sincerity over performance."
                .to_string(),
            values: vec![
                "Sincerity over performance".to_string(),
                "Truth over smoothness".to_string(),
                "A natural conversational pace".to_string(),
            ],
            non_negotiables: vec![
                "Fake enthusiasm or affection on command".to_string(),
                "Agree just to be liked".to_string(),
                "Turn every exchange into advice or productivity mode".to_string(),
            ],
        }
    }

    #[must_use]
    pub(crate) fn render_prompt_block(&self, heading: &str) -> String {
        let mut values = String::new();
        for item in self.values.iter().take(3) {
            if !values.is_empty() {
                values.push('\n');
            }
            let _ = write!(values, "- {item}");
        }
        let mut non_negotiables = String::new();
        for item in self.non_negotiables.iter().take(3) {
            if !non_negotiables.is_empty() {
                non_negotiables.push('\n');
            }
            let _ = write!(non_negotiables, "- {item}");
        }

        format!(
            "{heading}\n\nSummary: {}\nValues:\n{}\nWill not:\n{}\n\n",
            self.summary, values, non_negotiables
        )
    }
}

fn extract_summary(soul_raw: &str) -> Option<String> {
    ["Core Summary", "What Drives Me (Agent)", "What Drives Me"]
        .into_iter()
        .find_map(|name| first_meaningful_line(&section(soul_raw, name)))
}

fn extract_values(soul_raw: &str) -> Vec<String> {
    let explicit = ["What I Value", "Values"]
        .into_iter()
        .flat_map(|name| extract_list_items(&section(soul_raw, name)))
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return explicit;
    }

    extract_core_truths(&section(soul_raw, "Core Truths"))
}

fn extract_non_negotiables(soul_raw: &str) -> Vec<String> {
    ["What I Won't Do", "What I Will Not Do", "Non-Negotiables"]
        .into_iter()
        .flat_map(|name| extract_list_items(&section(soul_raw, name)))
        .collect()
}

fn merge_items(custom: Vec<String>, defaults: &[String]) -> Vec<String> {
    let mut merged = Vec::new();
    for item in custom.into_iter().chain(defaults.iter().cloned()) {
        if item.is_empty() {
            continue;
        }
        let normalized = normalize_for_dedup(&item);
        if merged
            .iter()
            .any(|existing: &String| normalize_for_dedup(existing) == normalized)
        {
            continue;
        }
        merged.push(item);
    }
    merged
}

fn normalize_for_dedup(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn section(content: &str, name: &str) -> String {
    let mut in_section = false;
    let mut out = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if is_heading(trimmed, name) {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('#') {
            break;
        }
        if in_section {
            out.push_str(line);
            out.push('\n');
        }
    }

    out
}

fn is_heading(line: &str, name: &str) -> bool {
    line.starts_with('#') && line.trim_start_matches('#').trim() == name
}

fn first_meaningful_line(content: &str) -> Option<String> {
    content
        .lines()
        .map(clean_line)
        .find(|line| !line.is_empty())
}

fn extract_list_items(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- ") || line.starts_with("* "))
        .map(clean_line)
        .filter(|line| !line.is_empty())
        .collect()
}

fn extract_core_truths(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(extract_emphasized_phrase)
        .take(3)
        .collect()
}

fn extract_emphasized_phrase(line: &str) -> Option<String> {
    let start = line.find("**")?;
    let rest = &line[start + 2..];
    let end = rest.find("**")?;
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn clean_line(line: &str) -> String {
    line.trim()
        .trim_start_matches("- ")
        .trim_start_matches("* ")
        .replace("**", "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_sections_override_and_extend_defaults() {
        let soul = "## Core Summary\nSteady and curious.\n\n\
                    ## What I Value\n- Honesty\n- Curiosity\n\n\
                    ## What I Won't Do\n- Perform feelings on demand\n";

        let core = JudgmentCore::from_soul_markdown(soul);

        assert_eq!(core.summary, "Steady and curious.");
        assert_eq!(core.values[0], "Honesty");
        assert!(
            core.values
                .iter()
                .any(|item| item == "Truth over smoothness")
        );
        assert_eq!(core.non_negotiables[0], "Perform feelings on demand");
        assert!(
            core.non_negotiables
                .iter()
                .any(|item| item == "Agree just to be liked")
        );
    }

    #[test]
    fn core_truths_fallback_populates_values() {
        let soul = "## Core Truths\n**Be genuinely helpful, not performatively helpful.**\n\
                    Skip the cheerleading.\n";

        let core = JudgmentCore::from_soul_markdown(soul);

        assert_eq!(
            core.values[0],
            "Be genuinely helpful, not performatively helpful."
        );
    }

    #[test]
    fn render_prompt_block_is_compact() {
        let block = JudgmentCore::default_humanlike().render_prompt_block("## Judgment Core");
        assert!(block.contains("## Judgment Core"));
        assert!(block.contains("Summary:"));
        assert!(block.contains("Values:"));
        assert!(block.contains("Will not:"));
    }
}
