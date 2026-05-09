use serde_json::Value;

use super::{finalize_compaction, truncate_preview};
use crate::core::tools::middleware::{SemanticCompactionOutcome, SemanticFormatter};
use crate::core::tools::traits::ToolResultSemanticMetadata;

#[derive(Debug, Default)]
pub(crate) struct BrowserFindFormatter;

impl SemanticFormatter for BrowserFindFormatter {
    fn compact(
        &self,
        raw: &str,
        _metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(action) = value.get("action").and_then(Value::as_str) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(locator) = value.get("locator").and_then(Value::as_object) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };
        let Some(match_data) = value.get("match").and_then(Value::as_object) else {
            return SemanticCompactionOutcome::FallbackRaw;
        };

        let locator_by = locator
            .get("by")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let locator_value = locator.get("value").and_then(Value::as_str).unwrap_or("");
        let match_ref = match_data
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let match_role = match_data
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let match_name = match_data.get("name").and_then(Value::as_str).unwrap_or("");
        let confirmation = value
            .get("confirmation")
            .and_then(Value::as_str)
            .unwrap_or("");

        let compacted = format!(
            "browser.find\naction: {action}\nlocator: {locator_by} {}\nmatch: ref {match_ref} role {match_role} {}\nconfirmation: {}",
            truncate_preview(locator_value, 120),
            truncate_preview(match_name, 120),
            truncate_preview(confirmation, 180)
        );

        finalize_compaction(raw, compacted)
    }
}
