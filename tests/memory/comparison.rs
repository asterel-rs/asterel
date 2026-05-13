//! Markdown memory backend smoke tests
//!
//! Run with: cargo test --test memory -- comparison --nocapture

use std::time::Instant;

use asterel::core::memory::{
    CapabilitySupport, ForgetMode, MarkdownMemory, Memory, MemoryCategory, MemoryReader,
    MemoryRecallEntry, MemorySource, PrivacyLevel, capability_matrix_for_memory,
    ensure_forget_mode_supported,
};
use tempfile::TempDir;

use super::memory_harness;
use super::memory_harness::{
    ParityRelation, append_test_event, assert_event_count_parity, capture_recall_items_as_csv,
    find_degraded_backends, format_capability_evidence, markdown_memory_from_path, memory_count,
    recall_scoped_values, resolve_slot_value,
};

// -- Helpers ----------------------------------------------------------------

fn markdown_backend(dir: &std::path::Path) -> MarkdownMemory {
    markdown_memory_from_path(dir)
}

async fn store(mem: &dyn Memory, key: &str, content: &str, category: MemoryCategory) {
    append_test_event(mem, "default", key, content, category).await;
}

async fn count(mem: &dyn Memory) -> usize {
    memory_count(mem).await
}

async fn get_value(mem: &dyn Memory, key: &str) -> Option<String> {
    resolve_slot_value(mem, "default", key).await
}

async fn recall(mem: &dyn Memory, query: &str, limit: usize) -> Vec<(String, String, f64)> {
    recall_scoped_values(mem, "default", query, limit).await
}

// -- Test 1: Store performance -----------------------------------------------

#[tokio::test]
async fn compare_store_speed() {
    let tmp_md = TempDir::new().expect("test setup should succeed");
    let md = markdown_backend(tmp_md.path());

    let n = 100;

    let start = Instant::now();
    for i in 0..n {
        store(
            &md,
            &format!("key_{i}"),
            &format!("Memory entry number {i} about Rust programming"),
            MemoryCategory::Core,
        )
        .await;
    }
    let md_dur = start.elapsed();

    println!("\n============================================================");
    println!("STORE {n} entries:");
    println!("  Markdown: {:?}", md_dur);

    let md_count = count(&md).await;
    assert!(md_count >= n, "Markdown stored {md_count}, expected >= {n}");
}

// -- Test 2: Recall / search quality -----------------------------------------

#[tokio::test]
async fn compare_recall_quality() {
    let tmp_md = TempDir::new().expect("test setup should succeed");
    let md = markdown_backend(tmp_md.path());

    let entries = vec![
        (
            "lang_pref",
            "User prefers Rust over Python",
            MemoryCategory::Core,
        ),
        (
            "editor",
            "Uses VS Code with rust-analyzer",
            MemoryCategory::Core,
        ),
        ("tz", "Timezone is EST, works 9-5", MemoryCategory::Core),
        (
            "proj1",
            "Working on Asterel AI assistant",
            MemoryCategory::Daily,
        ),
        (
            "proj2",
            "Previous project was a web scraper in Python",
            MemoryCategory::Daily,
        ),
        (
            "deploy",
            "Deploys to Hetzner VPS via Docker",
            MemoryCategory::Core,
        ),
        (
            "model",
            "Prefers Claude Sonnet for coding tasks",
            MemoryCategory::Core,
        ),
        (
            "style",
            "Likes concise responses, no fluff",
            MemoryCategory::Core,
        ),
        (
            "rust_note",
            "Rust's ownership model prevents memory bugs",
            MemoryCategory::Daily,
        ),
        (
            "perf",
            "Cares about binary size and startup time",
            MemoryCategory::Core,
        ),
    ];

    for (key, content, cat) in &entries {
        store(&md, key, content, cat.clone()).await;
    }

    let queries = vec![
        ("Rust", "Should find Rust-related entries"),
        ("Python", "Should find Python references"),
        ("deploy Docker", "Multi-keyword search"),
        ("Claude", "Specific tool reference"),
        ("javascript", "No matches expected"),
        ("binary size startup", "Multi-keyword partial match"),
    ];

    println!("\n============================================================");
    println!("RECALL QUALITY (10 entries seeded):\n");

    for (query, desc) in &queries {
        let md_results = recall(&md, query, 10).await;

        println!("  Query: \"{query}\" -- {desc}");
        println!("    Markdown: {} results", md_results.len());
        for r in &md_results {
            println!("      [{:.2}] {}: {}", r.2, r.0, &r.1[..r.1.len().min(50)]);
        }
        println!();
    }
}

// -- Test 3: Recall speed at scale -------------------------------------------

#[tokio::test]
async fn compare_recall_speed() {
    let tmp_md = TempDir::new().expect("test setup should succeed");
    let md = markdown_backend(tmp_md.path());

    let n = 200;
    for i in 0..n {
        let content = if i % 3 == 0 {
            format!("Rust is great for systems programming, entry {i}")
        } else if i % 3 == 1 {
            format!("Python is popular for data science, entry {i}")
        } else {
            format!("TypeScript powers modern web apps, entry {i}")
        };
        store(&md, &format!("e{i}"), &content, MemoryCategory::Daily).await;
    }

    let start = Instant::now();
    let md_results = recall(&md, "Rust", 10).await;
    let md_dur = start.elapsed();

    println!("\n============================================================");
    println!("RECALL from {n} entries (query: \"Rust\", limit 10):");
    println!("  Markdown: {:?} -> {} results", md_dur, md_results.len());

    assert!(!md_results.is_empty());
}

// -- Test 4: Persistence -----------------------------------------------------

#[tokio::test]
async fn compare_persistence() {
    let tmp_md = TempDir::new().expect("test setup should succeed");

    {
        let md = markdown_backend(tmp_md.path());
        store(
            &md,
            "persist_test",
            "I should survive",
            MemoryCategory::Core,
        )
        .await;
    }

    let md2 = markdown_backend(tmp_md.path());
    let md_entry = get_value(&md2, "persist_test").await;

    println!("\n============================================================");
    println!("PERSISTENCE (store -> drop -> re-open -> get):");
    println!(
        "  Markdown: {}",
        if md_entry.is_some() {
            "Survived"
        } else {
            "Lost"
        }
    );

    assert!(md_entry.is_some());
}

// -- Test 5: Upsert / update behavior ----------------------------------------

#[tokio::test]
async fn compare_upsert() {
    let tmp_md = TempDir::new().expect("test setup should succeed");
    let md = markdown_backend(tmp_md.path());

    store(&md, "pref", "likes Rust", MemoryCategory::Core).await;
    store(&md, "pref", "loves Rust", MemoryCategory::Core).await;

    let md_count = count(&md).await;
    let md_results = recall(&md, "loves Rust", 5).await;

    println!("\n============================================================");
    println!("UPSERT (store same key twice):");
    println!("  Markdown: count={md_count} (append-only, both entries kept)");
    println!("    Can still find latest: {}", !md_results.is_empty());

    assert!(md_count >= 2, "Markdown should keep both entries");
}

// -- Test 6: Forget / delete capability ---------------------------------------

#[tokio::test]
async fn compare_forget() {
    let tmp_md = TempDir::new().expect("test setup should succeed");
    let md = markdown_backend(tmp_md.path());

    store(&md, "secret", "API key: sk-1234", MemoryCategory::Core).await;

    let md_forgot = memory_harness::forget_hard(&md, "default", "secret").await;

    println!("\n============================================================");
    println!("FORGET (delete sensitive data):");
    println!(
        "  Markdown: {} (append-only by design)",
        if md_forgot {
            "Deleted"
        } else {
            "Cannot delete (audit trail)"
        },
    );

    assert!(!md_forgot);
}

// -- Test 7: Category filtering -----------------------------------------------

#[tokio::test]
async fn compare_category_filter() {
    let tmp_md = TempDir::new().expect("test setup should succeed");
    let md = markdown_backend(tmp_md.path());

    store(&md, "a", "core fact 1", MemoryCategory::Core).await;
    store(&md, "b", "core fact 2", MemoryCategory::Core).await;
    store(&md, "c", "daily note", MemoryCategory::Daily).await;

    let md_a = md
        .resolve_slot("default", "a")
        .await
        .expect("test setup should succeed");
    let md_b = md
        .resolve_slot("default", "b")
        .await
        .expect("test setup should succeed");
    let md_c = md
        .resolve_slot("default", "c")
        .await
        .expect("test setup should succeed");
    let md_core = usize::from(md_a.is_some()) + usize::from(md_b.is_some());
    let md_daily = usize::from(md_c.is_some());
    let md_all = count(&md).await;

    println!("\n============================================================");
    println!("CATEGORY FILTERING:");
    println!(
        "  Markdown: core={}, daily={}, all={}",
        md_core, md_daily, md_all
    );

    assert!(md_core >= 1);
    assert!(md_all >= 1);
}

#[tokio::test]
async fn memory_test_harness_smoke() {
    let (_tmp_md, markdown) = memory_harness::markdown_fixture();

    append_test_event(
        &markdown,
        "h-smoke",
        "smoke.pref",
        "value: rust",
        MemoryCategory::Core,
    )
    .await;

    assert_event_count_parity(
        ParityRelation::Exact,
        memory_count(&markdown).await,
        1,
        "markdown fixture receives one event",
    );

    let markdown_value = resolve_slot_value(&markdown, "h-smoke", "smoke.pref").await;
    assert_eq!(markdown_value.as_deref(), Some("value: rust"));

    let markdown_csv = capture_recall_items_as_csv(&[MemoryRecallEntry {
        entity_id: "h-smoke".into(),
        slot_key: "smoke.pref".into(),
        value: "value: rust".to_string(),
        source: MemorySource::ExplicitUser,
        confidence: 1.0.into(),
        importance: 1.0.into(),
        privacy_level: PrivacyLevel::Private,
        score: 0.0,
        occurred_at: "0000-00-00T00:00:00Z".to_string(),
    }]);
    assert_eq!(markdown_csv, "smoke.pref,h-smoke,0.000000\n");

    let evidence = format_capability_evidence();
    assert!(evidence.contains("backend=markdown"));
}

#[tokio::test]
async fn memory_test_harness_flags_degraded() {
    let (_tmp_md, markdown) = memory_harness::markdown_fixture();

    let markdown_caps = capability_matrix_for_memory(&markdown);

    assert_eq!(markdown_caps.forget_soft, CapabilitySupport::Degraded);
    assert_eq!(markdown_caps.forget_hard, CapabilitySupport::Unsupported);
    assert_eq!(markdown_caps.forget_tombstone, CapabilitySupport::Degraded);

    assert!(
        ensure_forget_mode_supported(&markdown, ForgetMode::Tombstone).is_ok(),
        "markdown tombstone forget is contractually supported"
    );

    assert!(
        find_degraded_backends().contains(&"markdown"),
        "markdown must be marked as degraded/unsupported",
    );

    let markdown_hard = ensure_forget_mode_supported(&markdown, ForgetMode::Hard)
        .expect_err("markdown hard delete should be rejected in test contract");
    assert_eq!(
        markdown_hard.to_string(),
        "memory capability unsupported: memory backend 'markdown' does not support forget mode 'hard'"
    );
}
