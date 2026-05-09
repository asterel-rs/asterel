//! Observability backend selection (None, Log, Prometheus, OpenTelemetry)
//! and lifecycle metric support flags.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Observability backend.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservabilityBackend {
    /// No observability (disabled).
    #[default]
    None,
    /// Structured logging backend.
    Log,
    /// Prometheus metrics exporter.
    Prometheus,
    /// `OpenTelemetry` (OTLP) exporter.
    Otel,
}

impl fmt::Display for ObservabilityBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Log => write!(f, "log"),
            Self::Prometheus => write!(f, "prometheus"),
            Self::Otel => write!(f, "otel"),
        }
    }
}

impl ObservabilityBackend {
    /// Whether this backend supports lifecycle metrics (autonomy, memory, etc.).
    #[must_use]
    pub fn supports_lifecycle_metrics(self) -> bool {
        matches!(self, Self::Log | Self::Prometheus | Self::Otel)
    }
}

/// Observability configuration section.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Selected observability backend.
    pub backend: ObservabilityBackend,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_observability_config() {
        let config = ObservabilityConfig::default();
        assert_eq!(config.backend, ObservabilityBackend::None);
    }

    #[test]
    fn observability_config_toml_round_trip() {
        let original = ObservabilityConfig {
            backend: ObservabilityBackend::Prometheus,
        };

        let toml = toml::to_string(&original).unwrap();
        let decoded: ObservabilityConfig = toml::from_str(&toml).unwrap();

        assert_eq!(decoded.backend, original.backend);
    }
}
