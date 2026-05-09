use asterel::config::MemoryConfig;
use asterel::core::memory::{
    CapabilitySupport, ForgetMode, backend_capability_matrix, capability_matrix_for_backend,
    capability_matrix_for_memory, create_memory, ensure_forget_mode_supported,
};
use tempfile::TempDir;

#[test]
fn memory_capability_matrix() {
    let matrix = backend_capability_matrix();
    assert_eq!(matrix.len(), 2);

    let markdown = capability_matrix_for_backend("markdown").expect("markdown capability row");
    assert_eq!(markdown.forget_soft, CapabilitySupport::Degraded);
    assert_eq!(markdown.forget_hard, CapabilitySupport::Unsupported);
    assert_eq!(markdown.forget_tombstone, CapabilitySupport::Degraded);
}

#[test]
fn memory_capability_rejects_unsupported() {
    let markdown = capability_matrix_for_backend("markdown").expect("markdown capability row");
    let err = markdown
        .require_forget_mode(ForgetMode::Hard)
        .expect_err("hard delete must be rejected for markdown backend contract");

    assert_eq!(
        err.to_string(),
        "memory capability unsupported: memory backend 'markdown' does not support forget mode 'hard'"
    );
}

#[tokio::test]
async fn memory_capability_matrix_runtime_access() {
    let tmp = TempDir::new().expect("temp dir");
    let markdown_cfg = MemoryConfig {
        backend: asterel::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let markdown_memory = create_memory(&markdown_cfg, tmp.path(), None)
        .await
        .expect("markdown memory");
    let markdown_caps = capability_matrix_for_memory(markdown_memory.as_ref());
    assert_eq!(markdown_caps.backend, "markdown");

    let hard_delete_err = ensure_forget_mode_supported(markdown_memory.as_ref(), ForgetMode::Hard)
        .expect_err("markdown hard forget should be rejected by capability preflight");
    assert_eq!(
        hard_delete_err.to_string(),
        "memory capability unsupported: memory backend 'markdown' does not support forget mode 'hard'"
    );
}
