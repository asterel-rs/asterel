//! Runtime-owned read-model queries for runtime status and lightweight operator snapshots.

use crate::runtime::diagnostics::control_plane_read_models::{
    RuntimeCapabilitiesReadModel, RuntimeCapabilityDetailReadModel, RuntimeStatusReadModel,
    build_runtime_status_read_model,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatusSnapshot {
    pub status: String,
    pub memory_backend: String,
    pub persistence_status: String,
    pub model: String,
    pub ws_connections: usize,
    pub max_ws_connections: usize,
    pub capabilities: RuntimeCapabilitiesReadModel,
    pub capability_details: Vec<RuntimeCapabilityDetailReadModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GatewayRestartReadModel {
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MoodReadModel {
    pub label: String,
    pub confidence: f64,
    pub description: String,
}

#[must_use]
pub fn load_admin_runtime_status(snapshot: RuntimeStatusSnapshot) -> RuntimeStatusReadModel {
    build_runtime_status_read_model(
        snapshot.status,
        snapshot.memory_backend,
        snapshot.persistence_status,
        snapshot.model,
        snapshot.ws_connections,
        snapshot.max_ws_connections,
        snapshot.capabilities,
        snapshot.capability_details,
    )
}

#[must_use]
pub fn request_gateway_restart() -> GatewayRestartReadModel {
    GatewayRestartReadModel {
        status: "restart_requested".to_string(),
        message: "Gateway restart has been requested. The daemon will restart shortly.".to_string(),
    }
}

#[must_use]
pub fn load_admin_mood() -> MoodReadModel {
    MoodReadModel {
        label: "calm".to_string(),
        confidence: 0.8,
        description: "Ready and attentive".to_string(),
    }
}
