use std::fmt::Write as _;

use serde_json::Value;

use super::{EXAMPLE_LIMIT, finalize_compaction, truncate_preview};
use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

#[derive(Debug, Default)]
pub(crate) struct WebScrapeFormatter;

impl SemanticFormatter for WebScrapeFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(url) = value.get("url").and_then(Value::as_str) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(selector) = value.get("selector").and_then(Value::as_str) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(matches) = value.get("matches").and_then(Value::as_array) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(total_found) = value.get("total_found").and_then(Value::as_u64) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let mut compacted = String::from("web_scrape\n");
        let _ = writeln!(compacted, "url: {url}");
        let _ = writeln!(compacted, "selector: {selector}");
        let _ = writeln!(compacted, "total_found: {total_found}");
        for entry in matches.iter().take(EXAMPLE_LIMIT) {
            let Some(index) = entry.get("index").and_then(Value::as_u64) else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let Some(text) = entry.get("text").and_then(Value::as_str) else {
                return SemanticCompactionOutcome::FallbackRaw;
            };
            let _ = writeln!(compacted, "match {index}: {}", truncate_preview(text, 180));
        }

        finalize_compaction(raw, compacted.trim_end().to_string())
    }
}
