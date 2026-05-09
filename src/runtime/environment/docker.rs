//! Docker runtime adapter: container-sandboxed execution with
//! workspace-scoped filesystem access.

use std::path::PathBuf;

use super::traits::{RuntimeAdapter, RuntimeSandboxClass};

/// Docker container runtime with workspace-scoped filesystem access.
pub struct DockerRuntime;

impl DockerRuntime {
    /// Create a new Docker runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for DockerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAdapter for DockerRuntime {
    fn name(&self) -> &'static str {
        "docker"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_fs_access(&self) -> bool {
        true
    }

    fn storage_path(&self) -> PathBuf {
        PathBuf::from("/workspace/.asterel")
    }

    fn supports_long_runs(&self) -> bool {
        true
    }

    fn sandbox_class(&self) -> RuntimeSandboxClass {
        RuntimeSandboxClass::Container
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_name() {
        assert_eq!(DockerRuntime::new().name(), "docker");
    }

    #[test]
    fn docker_has_shell_access() {
        assert!(DockerRuntime::new().has_shell_access());
    }

    #[test]
    fn docker_has_filesystem_access() {
        assert!(DockerRuntime::new().has_fs_access());
    }

    #[test]
    fn docker_supports_long_running() {
        assert!(DockerRuntime::new().supports_long_runs());
    }

    #[test]
    fn docker_memory_budget_unlimited() {
        assert_eq!(DockerRuntime::new().memory_budget(), 0);
    }

    #[test]
    fn docker_sandbox_class_is_container() {
        assert_eq!(
            DockerRuntime::new().sandbox_class(),
            RuntimeSandboxClass::Container
        );
    }

    #[test]
    fn docker_storage_path_is_workspace_scoped() {
        let path = DockerRuntime::new().storage_path();
        assert_eq!(path, PathBuf::from("/workspace/.asterel"));
    }
}
