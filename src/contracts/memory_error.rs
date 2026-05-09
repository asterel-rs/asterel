//! Typed errors for memory subsystem boundaries.
//!
//! Application entrypoints may erase these into `anyhow::Error`, but memory
//! traits and ingestion seams should preserve these categories so callers can
//! distinguish validation, policy, backend, capability, and persistence failures
//! without parsing error strings.

/// Error returned by memory backend and memory trait boundaries.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MemoryError {
    /// Caller-provided or normalized memory input is invalid.
    #[error("memory validation failed: {0}")]
    Validation(String),

    /// A memory access, writeback, tenant, privacy, or governance policy rejected the operation.
    #[error("memory policy violation: {0}")]
    Policy(String),

    /// The backend intentionally does not implement the requested capability.
    #[error("memory capability unsupported: {0}")]
    Unsupported(String),

    /// The memory backend is unavailable or cannot be connected.
    #[error("memory backend unavailable: {0}")]
    BackendUnavailable(String),

    /// A memory read, recall, count, list, or projection query failed.
    #[error("memory query failed: {0}")]
    Query(String),

    /// A memory write, append, slot update, projection write, or durable mutation failed.
    #[error("memory write failed: {0}")]
    Write(String),

    /// Memory integrity verification failed or could not complete.
    #[error("memory integrity check failed: {0}")]
    Integrity(String),

    /// Temporary migration escape hatch for unclassified memory errors.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result returned by memory subsystem boundaries.
pub type MemoryResult<T> = std::result::Result<T, MemoryError>;

impl MemoryError {
    /// Construct a validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    /// Construct a policy error.
    pub fn policy(message: impl Into<String>) -> Self {
        Self::Policy(message.into())
    }

    /// Construct an unsupported-capability error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    /// Construct a backend-unavailable error.
    pub fn backend_unavailable(message: impl Into<String>) -> Self {
        Self::BackendUnavailable(message.into())
    }

    /// Construct a query error.
    pub fn query(message: impl Into<String>) -> Self {
        Self::Query(message.into())
    }

    /// Construct a write error.
    pub fn write(message: impl Into<String>) -> Self {
        Self::Write(message.into())
    }

    /// Construct an integrity error.
    pub fn integrity(message: impl Into<String>) -> Self {
        Self::Integrity(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryError;

    #[test]
    fn memory_error_formats_stable_categories() {
        assert_eq!(
            MemoryError::validation("bad slot").to_string(),
            "memory validation failed: bad slot"
        );
        assert_eq!(
            MemoryError::policy("tenant mismatch").to_string(),
            "memory policy violation: tenant mismatch"
        );
        assert_eq!(
            MemoryError::unsupported("entity listing").to_string(),
            "memory capability unsupported: entity listing"
        );
    }

    #[test]
    fn memory_error_other_preserves_source_message() {
        let error = MemoryError::from(anyhow::anyhow!("legacy backend failure"));

        assert_eq!(error.to_string(), "legacy backend failure");
    }
}
