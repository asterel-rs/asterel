#![allow(dead_code, clippy::needless_lifetimes, clippy::cast_precision_loss)]

use std::fmt;
use std::path::Path;

use asterel::core::memory::{
    CapabilitySupport, ForgetMode, MarkdownMemory, Memory, MemoryCapMatrix, MemoryCategory,
    MemoryEventInput, MemoryEventType, MemoryRecallEntry, MemorySource, PrivacyLevel, RecallQuery,
    backend_capability_matrix,
};
use tempfile::TempDir;

pub fn markdown_memory_from_path(path: &Path) -> MarkdownMemory {
    MarkdownMemory::new(path)
}

pub fn markdown_fixture() -> (TempDir, MarkdownMemory) {
    let temp_dir = TempDir::new().expect("temp directory should be created");
    let memory = markdown_memory_from_path(temp_dir.path());
    (temp_dir, memory)
}

pub fn source_for_category(category: &MemoryCategory) -> MemorySource {
    match category {
        MemoryCategory::Core => MemorySource::ExplicitUser,
        MemoryCategory::Daily => MemorySource::System,
        MemoryCategory::Conversation => MemorySource::Inferred,
        MemoryCategory::Custom(_) => MemorySource::ToolVerified,
    }
}

pub async fn append_test_event(
    memory: &dyn Memory,
    entity_id: &str,
    slot_key: &str,
    value: &str,
    category: MemoryCategory,
) {
    let source = source_for_category(&category);
    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                slot_key,
                MemoryEventType::FactAdded,
                value,
                source,
                PrivacyLevel::Private,
            )
            .with_confidence(0.95)
            .with_importance(0.6),
        )
        .await
        .expect("test event append should succeed");
}

pub async fn memory_count(memory: &dyn Memory) -> usize {
    memory
        .count_events(None)
        .await
        .expect("count_events should succeed")
}

pub async fn resolve_slot_value(
    memory: &dyn Memory,
    entity_id: &str,
    slot_key: &str,
) -> Option<String> {
    let resolved = memory
        .resolve_slot(entity_id, slot_key)
        .await
        .expect("resolve_slot should succeed")
        .map(|slot| slot.value);

    resolved.map(|value| normalize_slot_value(&value).to_string())
}

fn normalize_slot_value(value: &str) -> &str {
    value
        .strip_prefix("**")
        .and_then(|without_prefix| {
            without_prefix
                .split_once("**: ")
                .map(|(_, payload)| payload)
        })
        .unwrap_or(value)
}

pub async fn recall_scoped_values(
    memory: &dyn Memory,
    entity_id: &str,
    query: &str,
    limit: usize,
) -> Vec<(String, String, f64)> {
    let items = recall_scoped_items(memory, entity_id, query, limit).await;
    items
        .into_iter()
        .map(|item| (item.slot_key.to_string(), item.value, item.score))
        .collect()
}

pub async fn recall_scoped_items(
    memory: &dyn Memory,
    entity_id: &str,
    query: &str,
    limit: usize,
) -> Vec<MemoryRecallEntry> {
    memory
        .recall_scoped(RecallQuery::new(entity_id, query, limit))
        .await
        .expect("recall_scoped should succeed")
}

pub async fn forget_hard(memory: &dyn Memory, entity_id: &str, slot_key: &str) -> bool {
    memory
        .forget_slot(entity_id, slot_key, ForgetMode::Hard, "test")
        .await
        .expect("forget_slot should run")
        .was_applied
}

#[derive(Debug)]
pub enum ParityRelation {
    Exact,
    AtLeast,
}

pub fn assert_event_count_parity(relation: ParityRelation, lhs: usize, rhs: usize, message: &str) {
    match relation {
        ParityRelation::Exact => {
            assert_eq!(lhs, rhs, "{} (lhs={lhs}, rhs={rhs})", message);
        }
        ParityRelation::AtLeast => {
            assert!(lhs >= rhs, "{} (lhs={lhs}, rhs={rhs})", message);
        }
    }
}

pub fn format_capability_evidence() -> String {
    let mut lines = Vec::new();
    for matrix in backend_capability_matrix() {
        lines.push(format_capability_row(matrix));
    }
    lines.join("\n")
}

fn format_capability_row(matrix: &MemoryCapMatrix) -> String {
    let soft = format_support(matrix.forget_soft);
    let hard = format_support(matrix.forget_hard);
    let tombstone = format_support(matrix.forget_tombstone);
    format!(
        "backend={} soft={} hard={} tombstone={} contract={}",
        matrix.backend, soft, hard, tombstone, matrix.unsupported_contract
    )
}

fn format_support(support: CapabilitySupport) -> &'static str {
    match support {
        CapabilitySupport::Supported => "SUPPORTED",
        CapabilitySupport::Degraded => "DEGRADED",
        CapabilitySupport::Unsupported => "UNSUPPORTED",
    }
}

pub fn find_degraded_backends() -> Vec<&'static str> {
    backend_capability_matrix()
        .iter()
        .filter(|entry| {
            entry.forget_soft == CapabilitySupport::Degraded
                || entry.forget_hard == CapabilitySupport::Degraded
                || entry.forget_tombstone == CapabilitySupport::Degraded
                || entry.forget_tombstone == CapabilitySupport::Unsupported
                || entry.forget_hard == CapabilitySupport::Unsupported
                || entry.forget_soft == CapabilitySupport::Unsupported
        })
        .map(|entry| entry.backend)
        .collect()
}

pub fn capture_recall_items_as_csv(items: &[MemoryRecallEntry]) -> String {
    let mut out = String::new();
    for item in items {
        use fmt::Write as _;
        writeln!(
            &mut out,
            "{},{},{:.6}",
            item.slot_key, item.entity_id, item.score
        )
        .expect("string building should not fail");
    }
    out
}
