use std::fmt::Write as _;

use super::{EXAMPLE_LIMIT, finalize_compaction, truncate_preview};
use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

#[derive(Debug, Default)]
pub(crate) struct MemoryRecallFormatter;

impl SemanticFormatter for MemoryRecallFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Some((header, entries)) = raw.split_once('\n') else {
            return SemanticCompactionOutcome::Passthrough;
        };
        let Some(count) = header
            .strip_prefix("Found ")
            .and_then(|rest| rest.strip_suffix(" memories:"))
            .and_then(|value| value.parse::<usize>().ok())
        else {
            return SemanticCompactionOutcome::Passthrough;
        };

        let mut compacted = String::from("memory recall\n");
        let _ = writeln!(compacted, "count: {count}");

        let mut seen = 0usize;
        for line in entries.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some(rest) = trimmed.strip_prefix("- [") else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let Some((identity, value)) = rest.split_once("] ") else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let Some((entity_id, slot_key)) = identity.split_once(':') else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            if seen < EXAMPLE_LIMIT {
                let _ = writeln!(
                    compacted,
                    "entry: {entity_id}:{slot_key} {}",
                    truncate_preview(value.trim(), 160)
                );
            }
            seen += 1;
        }

        if seen == 0 {
            return SemanticCompactionOutcome::FallbackRaw;
        }

        finalize_compaction(raw, compacted.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryRecallFormatter;
    use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
    use crate::core::tools::traits::ToolResultSemanticMetadata;

    #[test]
    fn compacts_memory_recall_listing() {
        let formatter = MemoryRecallFormatter;
        let mut raw = String::from("Found 12 memories:\n");
        for index in 0..12 {
            raw.push_str(&format!(
                "- [default:slot_{index}] value {index} {}\n",
                "expanded memory content ".repeat(6)
            ));
        }

        let outcome = formatter.compact(&raw, &ToolResultSemanticMetadata::default());

        match outcome {
            SemanticCompactionOutcome::Compacted { content, .. } => {
                assert!(content.contains("memory recall"));
                assert!(content.contains("count: 12"));
            }
            other => panic!("expected compacted outcome, got {other:?}"),
        }
    }
}
