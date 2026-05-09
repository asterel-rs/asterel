#![allow(unsafe_code)]
//! Test-only helpers for safe, serialised environment variable
//! manipulation via `EnvVarGuard` and `ENV_LOCK`.

use std::sync::{LazyLock, Mutex};

/// Global mutex that serializes all test-time environment variable access.
pub(super) static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// RAII guard that sets or unsets an environment variable and
/// restores its original value on drop.
pub(super) struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    /// Set an environment variable, returning a guard that restores
    /// the previous value on drop.
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Test-only helper. All tests using EnvVarGuard acquire
        // ENV_LOCK first, serializing concurrent env-var access.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    /// Remove an environment variable, returning a guard that
    /// restores the previous value on drop.
    pub(super) fn unset(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Test-only helper. ENV_LOCK serializes access;
        // the guard restores the original value on drop.
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.previous {
            // SAFETY: Test-only restoration. ENV_LOCK is still held by
            // the enclosing test, so no concurrent env mutation.
            unsafe {
                std::env::set_var(self.key, value);
            }
        } else {
            // SAFETY: Test-only cleanup. Removes a variable introduced
            // by EnvVarGuard::set; ENV_LOCK serializes access.
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}
