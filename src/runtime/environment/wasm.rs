//! WASM runtime adapter: restricted sandbox with 128 MB memory budget
//! and no filesystem or network access.

use std::path::PathBuf;

use super::traits::{RuntimeAdapter, RuntimeSandboxClass};

/// WASM runtime adapter with restricted sandbox and 128 MB memory cap.
pub struct WasmRuntime;

impl WasmRuntime {
    /// Create a new WASM runtime adapter.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for WasmRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAdapter for WasmRuntime {
    fn name(&self) -> &'static str {
        "wasm"
    }

    fn has_shell_access(&self) -> bool {
        false
    }

    fn has_fs_access(&self) -> bool {
        false
    }

    fn storage_path(&self) -> PathBuf {
        PathBuf::from(".asterel/wasm")
    }

    fn supports_long_runs(&self) -> bool {
        false
    }

    fn sandbox_class(&self) -> RuntimeSandboxClass {
        RuntimeSandboxClass::Restricted
    }

    fn memory_budget(&self) -> u64 {
        128 * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_name() {
        assert_eq!(WasmRuntime::new().name(), "wasm");
    }

    #[test]
    fn wasm_sandbox_capabilities_are_restricted() {
        let runtime = WasmRuntime::new();
        assert!(!runtime.has_shell_access());
        assert!(!runtime.has_fs_access());
        assert!(!runtime.supports_long_runs());
    }

    #[test]
    fn wasm_memory_budget_is_bounded() {
        assert_eq!(WasmRuntime::new().memory_budget(), 128 * 1024 * 1024);
    }

    #[test]
    fn wasm_sandbox_class_is_restricted() {
        assert_eq!(
            WasmRuntime::new().sandbox_class(),
            RuntimeSandboxClass::Restricted
        );
    }
}
