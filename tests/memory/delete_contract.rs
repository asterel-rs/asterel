use asterel::core::memory::{ForgetMode, ForgetStatus, MemoryGovernance};

use super::memory_harness;

#[tokio::test]
async fn memory_delete_contract_degraded_backend() {
    let (_tmp_markdown, markdown) = memory_harness::markdown_fixture();
    memory_harness::append_test_event(
        &markdown,
        "entity-degraded",
        "slot.degraded",
        "value",
        asterel::core::memory::MemoryCategory::Core,
    )
    .await;

    let markdown_hard = markdown
        .forget_slot(
            "entity-degraded",
            "slot.degraded",
            ForgetMode::Hard,
            "degraded-hard",
        )
        .await
        .expect("markdown hard forget should return explicit degraded result");
    assert!(!markdown_hard.was_applied);
    assert!(!markdown_hard.is_complete);
    assert!(markdown_hard.is_degraded);
    assert_eq!(markdown_hard.status, ForgetStatus::DegradedNonComplete);
}
