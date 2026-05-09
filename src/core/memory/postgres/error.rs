//! Internal typed errors for the `PostgreSQL` memory backend.

use crate::contracts::memory_error::MemoryError;

/// Internal error returned by `PostgreSQL` memory backend helpers.
#[allow(dead_code)] // Projection/conversion variants are reserved for the next Postgres sub-slice.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum PostgresMemoryError {
    /// Caller-provided memory input failed validation before hitting the backend.
    #[error("postgres memory validation failed: {0}")]
    Validation(String),
    /// Tenant, privacy, or memory policy rejected the operation before querying.
    #[error("postgres memory policy violation: {0}")]
    Policy(String),
    /// Pool creation or backend connection failed.
    #[error("postgres connection failed: {0}")]
    Connect(String),
    /// Schema migration failed.
    #[error("postgres migration failed: {0}")]
    Migration(String),
    /// Read/query operation failed.
    #[error("postgres query failed: {0}")]
    Query(String),
    /// Write/mutation operation failed.
    #[error("postgres memory write failed: {0}")]
    Write(String),
    /// Graph projection operation failed.
    #[error("postgres graph projection failed: {0}")]
    Projection(String),
    /// Integrity check failed.
    #[error("postgres integrity check failed: {0}")]
    Integrity(String),
    /// Numeric or row conversion failed.
    #[error("postgres conversion failed: {0}")]
    Conversion(String),
    /// Temporary migration escape hatch for unclassified Postgres memory errors.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub(crate) type PostgresMemoryResult<T> = std::result::Result<T, PostgresMemoryError>;

impl PostgresMemoryError {
    pub(crate) fn connect(error: impl std::fmt::Display) -> Self {
        Self::Connect(error.to_string())
    }

    pub(crate) fn migration(error: impl std::fmt::Display) -> Self {
        Self::Migration(error.to_string())
    }

    pub(crate) fn query(error: impl std::fmt::Display) -> Self {
        Self::Query(error.to_string())
    }

    pub(crate) fn validation(error: impl std::fmt::Display) -> Self {
        Self::Validation(error.to_string())
    }

    pub(crate) fn policy(error: impl std::fmt::Display) -> Self {
        Self::Policy(error.to_string())
    }

    pub(crate) fn write(error: impl std::fmt::Display) -> Self {
        Self::Write(error.to_string())
    }

    pub(crate) fn projection(error: impl std::fmt::Display) -> Self {
        Self::Projection(error.to_string())
    }

    pub(crate) fn integrity(error: impl std::fmt::Display) -> Self {
        Self::Integrity(error.to_string())
    }

    pub(crate) fn conversion(error: impl std::fmt::Display) -> Self {
        Self::Conversion(error.to_string())
    }
}

pub(crate) trait PostgresMemoryResultExt<T> {
    fn pg_query(self, context: &'static str) -> PostgresMemoryResult<T>;
    fn pg_write(self, context: &'static str) -> PostgresMemoryResult<T>;
    fn pg_projection(self, context: &'static str) -> PostgresMemoryResult<T>;
    fn pg_integrity(self, context: &'static str) -> PostgresMemoryResult<T>;
}

impl<T, E> PostgresMemoryResultExt<T> for std::result::Result<T, E>
where
    E: std::fmt::Display,
{
    fn pg_query(self, context: &'static str) -> PostgresMemoryResult<T> {
        self.map_err(|error| PostgresMemoryError::query(format!("{context}: {error}")))
    }

    fn pg_write(self, context: &'static str) -> PostgresMemoryResult<T> {
        self.map_err(|error| PostgresMemoryError::write(format!("{context}: {error}")))
    }

    fn pg_projection(self, context: &'static str) -> PostgresMemoryResult<T> {
        self.map_err(|error| PostgresMemoryError::projection(format!("{context}: {error}")))
    }

    fn pg_integrity(self, context: &'static str) -> PostgresMemoryResult<T> {
        self.map_err(|error| PostgresMemoryError::integrity(format!("{context}: {error}")))
    }
}

impl From<PostgresMemoryError> for MemoryError {
    fn from(error: PostgresMemoryError) -> Self {
        match error {
            PostgresMemoryError::Validation(message) => Self::validation(message),
            PostgresMemoryError::Policy(message) => Self::policy(message),
            PostgresMemoryError::Connect(message) | PostgresMemoryError::Migration(message) => {
                Self::backend_unavailable(message)
            }
            PostgresMemoryError::Query(message) | PostgresMemoryError::Conversion(message) => {
                Self::query(message)
            }
            PostgresMemoryError::Write(message) | PostgresMemoryError::Projection(message) => {
                Self::write(message)
            }
            PostgresMemoryError::Integrity(message) => Self::integrity(message),
            PostgresMemoryError::Other(error) => Self::Other(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PostgresMemoryError;
    use crate::contracts::memory_error::MemoryError;

    #[test]
    fn postgres_errors_map_to_memory_categories() {
        assert!(matches!(
            MemoryError::from(PostgresMemoryError::connect("db down")),
            MemoryError::BackendUnavailable(_)
        ));
        assert!(matches!(
            MemoryError::from(PostgresMemoryError::query("select failed")),
            MemoryError::Query(_)
        ));
    }
}
