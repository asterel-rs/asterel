use std::fmt::Write as _;

use serde_json::Value;

use super::{EXAMPLE_LIMIT, finalize_compaction, truncate_preview};
use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

#[derive(Debug, Default)]
pub(crate) struct BrowserSnapshotFormatter;

impl SemanticFormatter for BrowserSnapshotFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(snapshot) = value.get("snapshot").and_then(Value::as_str) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(refs) = value.get("refs").and_then(Value::as_object) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let mut compacted = String::from("browser.snapshot\n");
        let _ = writeln!(compacted, "refs: {}", refs.len());
        for line in snapshot
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(EXAMPLE_LIMIT)
        {
            let _ = writeln!(compacted, "line: {}", truncate_preview(line, 180));
        }

        for (ref_id, node) in refs.iter().take(EXAMPLE_LIMIT) {
            let role = node
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let name = node.get("name").and_then(Value::as_str).unwrap_or("");
            let _ = writeln!(
                compacted,
                "ref {ref_id}: {} {}",
                role,
                truncate_preview(name, 80)
            );
        }

        finalize_compaction(raw, compacted.trim_end().to_string())
    }
}
