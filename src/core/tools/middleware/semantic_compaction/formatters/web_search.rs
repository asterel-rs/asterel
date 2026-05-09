use std::fmt::Write as _;

use serde_json::Value;

use super::{EXAMPLE_LIMIT, finalize_compaction, truncate_preview};
use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

#[derive(Debug, Default)]
pub(crate) struct WebSearchFormatter;

impl SemanticFormatter for WebSearchFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(query) = value.get("query").and_then(Value::as_str) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(results) = value.get("results").and_then(Value::as_array) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(total_found) = value.get("total_found").and_then(Value::as_u64) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let mut compacted = String::from("web_search\n");
        let _ = writeln!(compacted, "query: {query}");
        let _ = writeln!(compacted, "results: {total_found}");
        for (index, result) in results.iter().take(EXAMPLE_LIMIT).enumerate() {
            let Some(title) = result.get("title").and_then(Value::as_str) else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let Some(url) = result.get("url").and_then(Value::as_str) else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let Some(snippet) = result.get("snippet").and_then(Value::as_str) else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let _ = writeln!(
                compacted,
                "result {}: {}\nurl: {url}\nsnippet: {}",
                index + 1,
                truncate_preview(title, 120),
                truncate_preview(snippet, 180)
            );
        }

        finalize_compaction(raw, compacted.trim_end().to_string())
    }
}
