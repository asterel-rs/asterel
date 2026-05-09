use asterel::config::{RuntimeConfig, RuntimeKind};
use asterel::runtime::{RuntimeAdapter, create_runtime};

fn assert_native_contract(adapter: &dyn RuntimeAdapter) {
    assert_eq!(adapter.name(), "native");
    assert!(adapter.has_shell_access());
    assert!(adapter.has_fs_access());
    assert!(adapter.supports_long_runs());
    assert_eq!(adapter.memory_budget(), 0);

    let storage_path = adapter.storage_path();
    assert!(
        storage_path.to_string_lossy().contains("asterel"),
        "native storage path should include asterel, got: {}",
        storage_path.display()
    );
}

#[test]
fn native_runtime_adapter_satisfies_contract() {
    let config = RuntimeConfig {
        kind: RuntimeKind::Native,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    let adapter = create_runtime(&config).expect("native runtime should be created");
    assert_native_contract(adapter.as_ref());
}

#[test]
fn native_runtime_contract_is_deterministic_across_instances() {
    let config = RuntimeConfig {
        kind: RuntimeKind::Native,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    let first = create_runtime(&config).expect("first native runtime should be created");
    let second = create_runtime(&config).expect("second native runtime should be created");

    assert_native_contract(first.as_ref());
    assert_native_contract(second.as_ref());
    assert_eq!(first.name(), second.name());
    assert_eq!(first.memory_budget(), second.memory_budget());
}
