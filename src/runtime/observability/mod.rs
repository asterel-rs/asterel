//! Re-exports for the observability subsystem (log, Prometheus, OTEL stub,
//! noop, multi).

pub mod log;
pub mod multi;
pub mod otel;
pub mod prometheus;

pub mod traits {
    pub use crate::contracts::observability::{
        AutonomySignal, EntityKpiAxis, MemorySignal, NoopObserver, Observer, ObserverEvent,
        ObserverMetric,
    };
}

pub use traits::{
    AutonomySignal, EntityKpiAxis, MemorySignal, Observer, ObserverEvent, ObserverMetric,
};

pub use self::log::LogObserver;
pub use self::otel::OtelStubObserver;
pub use self::prometheus::PrometheusObserver;

/// Factory: create the right observer from config.
#[must_use]
pub fn create_observer(config: &crate::config::ObservabilityConfig) -> Box<dyn Observer> {
    match config.backend {
        crate::config::ObservabilityBackend::Log => Box::new(LogObserver::new()),
        crate::config::ObservabilityBackend::Prometheus => Box::new(PrometheusObserver::new()),
        crate::config::ObservabilityBackend::Otel => Box::new(OtelStubObserver::new()),
        crate::config::ObservabilityBackend::None => Box::new(traits::NoopObserver),
    }
}

#[cfg(test)]
mod tests;
