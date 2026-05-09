//! Read models for the `/status` runtime health endpoint.

use serde::{Deserialize, Serialize};

/// Top-level health and capability snapshot returned by the `/status` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatusReadModel {
    /// Overall runtime health label.
    pub status: String,
    /// `Asterel` package version string (from `CARGO_PKG_VERSION`).
    pub version: String,
    /// Active LLM model identifier used by this runtime instance.
    pub model: String,
    /// Database connectivity summary.
    pub db: RuntimeDbStatusReadModel,
    /// WebSocket gateway load summary.
    pub gateway: RuntimeGatewayStatusReadModel,
    /// Feature flags indicating which subsystems are enabled.
    pub capabilities: RuntimeCapabilitiesReadModel,
    /// Reason-bearing capability states for degraded/unsupported operator diagnosis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_details: Vec<RuntimeCapabilityDetailReadModel>,
}

/// Database connectivity sub-object within the runtime status response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDbStatusReadModel {
    /// Storage engine label (e.g. `"postgres"`, `"markdown"`, `"none"`).
    pub engine: String,
    /// Persistence status: `"connected"`, `"degraded"`, `"unsupported"`, or `"unavailable"`.
    pub status: String,
}

/// WebSocket gateway load sub-object within the runtime status response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeGatewayStatusReadModel {
    /// Number of currently open WebSocket connections.
    pub ws_connections: usize,
    /// Configured ceiling for simultaneous WebSocket connections.
    pub max_ws_connections: usize,
}

/// Feature-flag sub-object within the runtime status response.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilitiesReadModel {
    /// Whether the companion persona subsystem is active.
    pub companion: bool,
    /// Whether runtime trust / approval inspection is active.
    pub governance: bool,
    /// Whether operator memory review/correction endpoints are active.
    pub memory_review: bool,
    /// Whether channel posture inspection is active.
    pub channel_posture: bool,
    /// Whether session review is active.
    pub session_review: bool,
    /// Whether agent-to-agent (A2A) routing is active.
    pub a2a: bool,
    /// Whether multi-tenant session isolation is active.
    pub multi_tenant: bool,
}

/// Reason-bearing capability state used by operator/admin diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilityDetailReadModel {
    /// Stable capability name.
    pub name: String,
    /// `supported`, `degraded`, or `unsupported`.
    pub status: String,
    /// Human-readable reason when the capability is degraded or unsupported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[must_use]
pub fn build_runtime_status_read_model(
    overall_status: impl Into<String>,
    memory_backend: String,
    db_status: impl Into<String>,
    model: String,
    ws_connections: usize,
    max_ws_connections: usize,
    capabilities: RuntimeCapabilitiesReadModel,
    capability_details: Vec<RuntimeCapabilityDetailReadModel>,
) -> RuntimeStatusReadModel {
    RuntimeStatusReadModel {
        status: overall_status.into(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        model,
        db: RuntimeDbStatusReadModel {
            engine: memory_backend,
            status: db_status.into(),
        },
        gateway: RuntimeGatewayStatusReadModel {
            ws_connections,
            max_ws_connections,
        },
        capabilities,
        capability_details,
    }
}
