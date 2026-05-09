//! Native runtime adapter: full host access on Mac, Linux, and
//! Raspberry Pi with workspace-scoped sandbox class.

use std::path::PathBuf;

use super::traits::{RuntimeAdapter, RuntimeSandboxClass};

/// Native runtime — full access, runs on Mac/Linux/Docker/Raspberry Pi
pub struct NativeRuntime;

impl NativeRuntime {
    /// Create a new native runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAdapter for NativeRuntime {
    fn name(&self) -> &'static str {
        "native"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_fs_access(&self) -> bool {
        true
    }

    fn storage_path(&self) -> PathBuf {
        crate::utils::dirs::asterel_home_dir_or_local()
    }

    fn supports_long_runs(&self) -> bool {
        true
    }

    fn sandbox_class(&self) -> RuntimeSandboxClass {
        RuntimeSandboxClass::Workspace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_name() {
        assert_eq!(NativeRuntime::new().name(), "native");
    }

    #[test]
    fn native_has_shell_access() {
        assert!(NativeRuntime::new().has_shell_access());
    }

    #[test]
    fn native_has_filesystem_access() {
        assert!(NativeRuntime::new().has_fs_access());
    }

    #[test]
    fn native_supports_long_running() {
        assert!(NativeRuntime::new().supports_long_runs());
    }

    #[test]
    fn native_memory_budget_unlimited() {
        assert_eq!(NativeRuntime::new().memory_budget(), 0);
    }

    #[test]
    fn native_sandbox_class_is_workspace() {
        assert_eq!(
            NativeRuntime::new().sandbox_class(),
            RuntimeSandboxClass::Workspace
        );
    }

    #[test]
    fn native_storage_path_contains_asterel() {
        let path = NativeRuntime::new().storage_path();
        assert!(path.to_string_lossy().contains("asterel"));
    }
}
