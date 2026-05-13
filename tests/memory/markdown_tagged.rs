use asterel::core::memory::{
    ForgetMode, MemoryEventInput, MemoryEventType, MemoryGovernance, MemoryLayer, MemoryProvenance,
    MemoryReader, MemorySource, MemoryWriter, PrivacyLevel,
};

use super::memory_harness;
use super::memory_harness::{append_test_event, memory_count};

fn decode_percent(encoded: &str) -> Option<String> {
    let mut chars = encoded.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }

        let hi = chars.next()?;
        let lo = chars.next()?;
        let byte = u8::from_str_radix(&format!("{hi}{lo}"), 16).ok()?;
        out.push(byte as char);
    }
    Some(out)
}

fn parse_md_tags(line: &str) -> Option<Vec<(String, String)>> {
    let marker = " [md:";
    let suffix = "]: ";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find(suffix)?;
    let raw_tags = &rest[..end];

    Some(
        raw_tags
            .split(';')
            .filter_map(|entry| {
                let (key, raw_value) = entry.split_once('=')?;
                let value = decode_percent(raw_value)?;
                Some((key.to_string(), value))
            })
            .collect(),
    )
}

#[tokio::test]
async fn markdown_tagged_memory_roundtrip() {
    let (tmp, mem) = memory_harness::markdown_fixture();

    let input = MemoryEventInput::new(
        "entity-10",
        "profile.preference",
        MemoryEventType::FactAdded,
        "Prefer semantic, layer-aware memory",
        MemorySource::ToolVerified,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Identity)
    .with_provenance(
        MemoryProvenance::source_reference(MemorySource::ToolVerified, "task10.reference")
            .with_evidence_uri("https://example.test/task-10"),
    );

    mem.append_event(input)
        .await
        .expect("test setup should succeed");

    let core = tmp.path().join("MEMORY.md");
    let contents = std::fs::read_to_string(core).expect("test setup should succeed");
    let entry_line = contents
        .lines()
        .find(|line| line.contains("entity-10:profile.preference"))
        .expect("stored markdown entry should exist");

    let tags = parse_md_tags(entry_line).expect("tag block should parse");
    let tags: std::collections::BTreeMap<_, _> = tags.into_iter().collect();

    assert_eq!(tags.get("layer"), Some(&"identity".to_string()));
    assert_eq!(
        tags.get("provenance_source_class"),
        Some(&"tool_verified".to_string())
    );
    assert_eq!(
        tags.get("provenance_reference"),
        Some(&"task10.reference".to_string())
    );
    assert_eq!(
        tags.get("provenance_evidence_uri"),
        Some(&"https://example.test/task-10".to_string())
    );

    let resolved = mem
        .resolve_slot("entity-10", "profile.preference")
        .await
        .expect("test setup should succeed")
        .expect("slot should resolve after roundtrip");
    assert_eq!(resolved.value, "Prefer semantic, layer-aware memory");

    let recalled = mem
        .recall_scoped(asterel::core::memory::RecallQuery::new(
            "entity-10",
            "semantic",
            5,
        ))
        .await
        .expect("test setup should succeed");
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].value, "Prefer semantic, layer-aware memory");
}

#[tokio::test]
async fn markdown_hard_delete_reports_degraded() {
    let (_tmp, mem) = memory_harness::markdown_fixture();
    append_test_event(
        &mem,
        "entity-10",
        "sensitive_slot",
        "API key: sk-abc-123",
        asterel::core::memory::MemoryCategory::Core,
    )
    .await;

    let before = memory_count(&mem).await;
    let outcome = mem
        .forget_slot(
            "entity-10",
            "sensitive_slot",
            ForgetMode::Hard,
            "task10-delete",
        )
        .await
        .expect("test setup should succeed");
    let after = memory_count(&mem).await;

    assert!(
        !outcome.was_applied,
        "markdown hard delete should remain no-op"
    );
    assert_eq!(before, after, "markdown count should remain unchanged");

    let resolved = mem
        .resolve_slot("entity-10", "sensitive_slot")
        .await
        .expect("test setup should succeed")
        .expect("hard forget should not remove data for markdown");
    assert_eq!(resolved.value, "API key: sk-abc-123");
}
