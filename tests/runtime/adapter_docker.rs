use asterel::config::{RuntimeConfig, RuntimeKind};
use asterel::runtime::create_runtime;

#[test]
fn docker_runtime_contract_is_gated_by_default() {
    let config = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    let message = match create_runtime(&config) {
        Ok(_) => panic!("docker runtime should stay gated until rollout is enabled"),
        Err(error) => error.to_string(),
    };

    assert!(message.contains("runtime.kind='docker'"));
    assert!(message.contains("disabled by rollout gate"));
    assert!(message.contains("runtime.enable_docker_runtime=true"));
}

#[test]
fn docker_runtime_contract_has_native_parity_for_supported_capabilities() {
    let native = create_runtime(&RuntimeConfig {
        kind: RuntimeKind::Native,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    })
    .expect("native runtime should be created");
    let docker = create_runtime(&RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: true,
        ..RuntimeConfig::default()
    })
    .expect("docker runtime should be created when rollout gate is enabled");

    assert_eq!(docker.name(), "docker");
    assert_eq!(docker.has_shell_access(), native.has_shell_access());
    assert_eq!(docker.has_fs_access(), native.has_fs_access());
    assert_eq!(docker.supports_long_runs(), native.supports_long_runs());
    assert_eq!(docker.memory_budget(), native.memory_budget());

    let docker_storage_path = docker.storage_path();
    assert!(
        docker_storage_path.to_string_lossy().contains("asterel"),
        "docker storage path should include asterel, got: {}",
        docker_storage_path.display()
    );
}
