//! Per-backend capability matrices for memory forget semantics.
//!
//! Maps each memory backend to its supported forget modes (soft, hard,
//! tombstone) and provides query helpers for capability negotiation.

use super::traits::{CapabilitySupport, ForgetMode, Memory, MemoryCapMatrix};
use crate::contracts::memory_error::MemoryResult;

const POSTGRES_CAPABILITY_MATRIX: MemoryCapMatrix = MemoryCapMatrix {
    backend: "postgres",
    forget_soft: CapabilitySupport::Supported,
    forget_hard: CapabilitySupport::Supported,
    forget_tombstone: CapabilitySupport::Supported,
    unsupported_contract: "postgres supports soft/hard/tombstone forget semantics",
};

const MARKDOWN_CAPABILITY_MATRIX: MemoryCapMatrix = MemoryCapMatrix {
    backend: "markdown",
    forget_soft: CapabilitySupport::Degraded,
    forget_hard: CapabilitySupport::Unsupported,
    forget_tombstone: CapabilitySupport::Degraded,
    unsupported_contract: "markdown is append-only; hard forget cannot physically delete",
};

const BACKEND_CAPABILITY_MATRIX: [MemoryCapMatrix; 2] =
    [POSTGRES_CAPABILITY_MATRIX, MARKDOWN_CAPABILITY_MATRIX];

/// Return the full capability matrix for all known backends.
#[must_use]
pub fn backend_capability_matrix() -> &'static [MemoryCapMatrix] {
    &BACKEND_CAPABILITY_MATRIX
}

/// Look up the capability matrix for a backend by name.
#[must_use]
pub fn capability_matrix_for_backend(backend: &str) -> Option<MemoryCapMatrix> {
    let normalized = if backend == "none" {
        "markdown"
    } else {
        backend
    };
    BACKEND_CAPABILITY_MATRIX
        .iter()
        .find(|capability| capability.backend == normalized)
        .copied()
}

/// Look up the capability matrix for a live memory instance.
#[must_use]
pub fn capability_matrix_for_memory(memory: &dyn Memory) -> MemoryCapMatrix {
    capability_matrix_for_backend(memory.name()).unwrap_or(MARKDOWN_CAPABILITY_MATRIX)
}

/// # Errors
///
/// Returns an error when the selected backend does not support the requested
/// forget mode.
pub fn ensure_forget_mode_supported(memory: &dyn Memory, mode: ForgetMode) -> MemoryResult<()> {
    capability_matrix_for_memory(memory).require_forget_mode(mode)
}
