//! Typed errors for the memory signal ingestion boundary.

use crate::contracts::memory_error::MemoryError;

/// Error returned by signal ingestion normalization, policy, and persistence.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IngestionError {
    /// Signal envelope validation or normalization failed.
    #[error("signal validation failed: {0}")]
    Validation(String),

    /// Signal ingestion was rejected by memory write policy.
    #[error("ingestion policy rejected signal: {0}")]
    Policy(String),

    /// Ingestion-local state failed, such as a poisoned deduplication cache lock.
    #[error("ingestion state failed: {0}")]
    State(String),

    /// Signal persistence through the memory backend failed.
    #[error("memory persistence failed: {0}")]
    Persistence(#[from] MemoryError),

    /// Temporary migration escape hatch for unclassified ingestion errors.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result returned by memory signal ingestion boundaries.
pub type IngestionPipelineResult<T> = std::result::Result<T, IngestionError>;

impl IngestionError {
    /// Construct a validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    /// Construct a policy error.
    pub fn policy(message: impl Into<String>) -> Self {
        Self::Policy(message.into())
    }

    /// Construct a state error.
    pub fn state(message: impl Into<String>) -> Self {
        Self::State(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::IngestionError;
    use crate::contracts::memory_error::MemoryError;

    #[test]
    fn ingestion_error_formats_stable_categories() {
        assert_eq!(
            IngestionError::validation("empty content").to_string(),
            "signal validation failed: empty content"
        );
        assert_eq!(
            IngestionError::policy("bad source").to_string(),
            "ingestion policy rejected signal: bad source"
        );
        assert_eq!(
            IngestionError::state("cache poisoned").to_string(),
            "ingestion state failed: cache poisoned"
        );
    }

    #[test]
    fn ingestion_error_preserves_memory_error_category() {
        let error = IngestionError::from(MemoryError::write("append failed"));

        assert_eq!(
            error.to_string(),
            "memory persistence failed: memory write failed: append failed"
        );
    }
}
