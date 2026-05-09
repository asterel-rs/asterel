//! Shared gateway request/response and runtime state types.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use tokio::sync::{Mutex, RwLock, broadcast};

use super::companion_bridge::{
    CompanionCaptionEvt, CompanionContextIngressGate, CompanionWidgetRuntime, CompanionWindow,
};
use super::events::ServerMessage;
use super::replay_guard::ReplayGuard;
use crate::config::{ExternalKnowledgeTrustConfig, GatewayDefenseMode, LoopDetectionConfig};
use crate::core::memory::{DefaultIngestPipeline, Memory};
use crate::core::providers::Provider;
use crate::core::sessions::SessionOrchestrator;
use crate::core::subagents::SubagentOrchestrator;
use crate::core::tools::ToolRegistry;
use crate::runtime::services::{A2aTaskStore, GatewayReadinessProfile, TenantBindingStore};
use crate::security::pairing::PairingGuard;
use crate::security::{EntityRateLimiter, PermissionStore, SecurityPolicy};
#[cfg(feature = "whatsapp")]
use crate::transport::channels::WhatsAppChannel;

/// Default request body size for non-media routes (64KB) — prevents memory exhaustion.
pub const MAX_BODY_SIZE: usize = 65_536;
/// Maximum request body size for media-accepting routes (10MB) — file uploads,
/// multimodal ingestion, and other endpoints that legitimately receive large payloads.
pub const MEDIA_BODY_SIZE: usize = 10 * 1024 * 1024;
/// Request timeout (30s) — prevents slow-loris attacks
pub const REQUEST_TIMEOUT_SECS: u64 = 30;
/// Maximum concurrent WebSocket connections — prevents resource exhaustion.
pub const MAX_WS_CONNECTIONS: usize = 128;
/// Version identifier for the gateway's A2A message contract.
pub const A2A_PROTOCOL_VERSION: &str = "asterel.a2a/v1";
/// Version identifier for structured handoff context attached to A2A requests.
pub const A2A_CONTEXT_ENVELOPE_VERSION: &str = "asterel.a2a.context/v1";
/// Supported A2A output mode for gateway-originated responses.
pub const A2A_TEXT_OUTPUT_MODE: &str = "text/plain";
/// Capability token exposed by the gateway's A2A surface.
pub const A2A_CAPABILITY_TOOLS: &str = "tools";

/// Scope-sharded runtime map used by high-churn companion gateway state.
#[derive(Debug)]
pub struct ScopedRuntimeMap<T> {
    scopes: Arc<RwLock<HashMap<String, Arc<Mutex<T>>>>>,
}

impl<T> ScopedRuntimeMap<T> {
    /// Create an empty scoped runtime map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scopes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Return the runtime state for a scope if it exists.
    pub async fn get_scope(&self, scope: &str) -> Option<Arc<Mutex<T>>> {
        self.scopes.read().await.get(scope).cloned()
    }

    /// Remove and return the runtime state for a scope.
    pub async fn remove_scope(&self, scope: &str) -> Option<Arc<Mutex<T>>> {
        self.scopes.write().await.remove(scope)
    }

    /// Return the number of active scopes.
    pub async fn len(&self) -> usize {
        self.scopes.read().await.len()
    }

    /// Return whether there are no active scopes.
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Return the keys of all active scopes.
    pub async fn scope_keys(&self) -> Vec<String> {
        self.scopes.read().await.keys().cloned().collect()
    }

    /// Get or insert a scope using the provided initializer while enforcing
    /// a maximum number of active scopes.
    pub async fn get_or_insert_with<F>(
        &self,
        scope: &str,
        max_scopes: usize,
        init: F,
    ) -> Option<Arc<Mutex<T>>>
    where
        F: FnOnce() -> T,
    {
        if let Some(existing) = self.get_scope(scope).await {
            return Some(existing);
        }

        let mut scopes = self.scopes.write().await;
        if let Some(existing) = scopes.get(scope) {
            return Some(Arc::clone(existing));
        }
        if scopes.len() >= max_scopes {
            return None;
        }

        let entry = Arc::new(Mutex::new(init()));
        scopes.insert(scope.to_string(), Arc::clone(&entry));
        Some(entry)
    }

    /// Get or insert a default scope while enforcing a maximum number of
    /// active scopes.
    pub async fn get_or_insert_default(
        &self,
        scope: &str,
        max_scopes: usize,
    ) -> Option<Arc<Mutex<T>>>
    where
        T: Default,
    {
        self.get_or_insert_with(scope, max_scopes, T::default).await
    }
}

impl<T> Default for ScopedRuntimeMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for ScopedRuntimeMap<T> {
    fn clone(&self) -> Self {
        Self {
            scopes: Arc::clone(&self.scopes),
        }
    }
}

/// Projection store for companion context ingress gates (derived from companion context).
pub type CompanionContextGateStore = ScopedRuntimeMap<CompanionContextIngressGate>;
/// Projection store for companion caption events (derived from caption events).
pub type CompanionCaptionLogStore = ScopedRuntimeMap<VecDeque<CompanionCaptionEvt>>;
/// Projection store for companion widget runtime state (derived from widget state).
pub type CompanionWidgetRuntimeStore = ScopedRuntimeMap<CompanionWidgetRuntime>;
/// Projection store for companion request windows (derived from request events).
pub type CompanionRequestWindowStore = ScopedRuntimeMap<HashMap<String, CompanionWindow>>;

pub use crate::contracts::channels::GatewayCompanionSettings;

/// Shared state for all axum handlers
#[derive(Clone)]
pub struct AppState {
    pub runtime: GatewayRuntimeState,
    pub access: GatewayAccessState,
    pub companion: GatewayCompanionState,
    pub connections: GatewayConnectionState,
    #[cfg(feature = "whatsapp")]
    pub whatsapp: GatewayWhatsAppState,
}

#[derive(Clone)]
pub struct GatewayRuntimeState {
    pub provider: Arc<dyn Provider>,
    pub registry: Arc<ToolRegistry>,
    pub subagent_manager: Arc<SubagentOrchestrator>,
    pub rate_limiter: Arc<EntityRateLimiter>,
    pub max_tool_loop_iterations: u32,
    pub loop_detection: LoopDetectionConfig,
    pub permission_store: Arc<PermissionStore>,
    pub model: String,
    pub temperature: f64,
    pub session_history_max_tokens: usize,
    pub mem: Arc<dyn Memory>,
    pub observer: Arc<dyn crate::contracts::observability::Observer>,
    pub auto_save: bool,
    pub security: Arc<SecurityPolicy>,
    pub external_knowledge_trust: ExternalKnowledgeTrustConfig,
    pub session_manager: Option<Arc<SessionOrchestrator>>,
    pub self_amendment_candidate_review:
        crate::runtime::services::SelfAmendmentCandidateReviewStore,
    pub readiness_profile: GatewayReadinessProfile,
    pub config: Arc<crate::config::Config>,
}

/// Shared auth, exposure, and policy gates for gateway handlers.
#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct GatewayAccessState {
    pub webhook_secret: Option<Arc<str>>,
    pub pairing: Arc<PairingGuard>,
    pub defense_mode: GatewayDefenseMode,
    pub defense_kill_switch: bool,
}

/// Shared companion surfaces and event fan-out state.
#[derive(Clone)]
pub struct GatewayCompanionState {
    pub replay_guard: Arc<ReplayGuard>,
    pub settings: Arc<RwLock<GatewayCompanionSettings>>,
    pub companion_context_gates: CompanionContextGateStore,
    pub companion_context_ingestion: Arc<DefaultIngestPipeline>,
    pub companion_caption_logs: CompanionCaptionLogStore,
    pub companion_widget_runtimes: CompanionWidgetRuntimeStore,
    pub companion_request_windows: CompanionRequestWindowStore,
    pub gateway_events: broadcast::Sender<ServerMessage>,
}

/// Shared mutable connection/task state for websocket and A2A flows.
#[derive(Clone)]
pub struct GatewayConnectionState {
    pub a2a_tasks: A2aTaskStore,
    pub tenant_bindings: TenantBindingStore,
    /// Active WebSocket connection count for concurrency limiting.
    pub active_ws_connections: Arc<AtomicUsize>,
}

/// Shared `WhatsApp` integration state.
#[cfg(feature = "whatsapp")]
#[derive(Clone)]
pub struct GatewayWhatsAppState {
    pub channel: Option<Arc<WhatsAppChannel>>,
    /// `WhatsApp` app secret for webhook signature verification (`X-Hub-Signature-256`)
    pub app_secret: Option<Arc<str>>,
}

/// Webhook request body
#[derive(serde::Deserialize)]
pub struct WebhookBody {
    pub message: String,
}

/// Agent-card document served by A2A discovery endpoints.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct A2aAgentCard {
    pub schema_version: String,
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub url: String,
    pub authentication: A2aAuthentication,
    pub capabilities: A2aCapabilities,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_contract: Option<A2aMessageContract>,
}

/// A2A authentication hint emitted from the agent card.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct A2aAuthentication {
    #[serde(rename = "type")]
    pub auth_type: String,
}

/// Capability flags advertised by the A2A agent card.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct A2aCapabilities {
    pub streaming: bool,
    pub history: bool,
    pub tools: bool,
}

/// Explicit message-contract metadata for cross-agent callers.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct A2aMessageContract {
    pub protocol_version: String,
    pub context_envelope_version: String,
    pub supported_output_modes: Vec<String>,
}

/// A2A incoming message request payload.
#[derive(serde::Deserialize)]
pub struct A2aMessageRequest {
    #[serde(default)]
    pub schema_version: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub configuration: Option<A2aMessageConfiguration>,
    pub message: A2aInboundMessage,
}

/// Inbound A2A message envelope.
#[derive(serde::Deserialize)]
pub struct A2aInboundMessage {
    pub role: String,
    pub parts: Vec<A2aInboundPart>,
}

/// Inbound message part for A2A requests.
#[derive(serde::Deserialize)]
pub struct A2aInboundPart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// Optional provenance metadata supplied by an upstream agent.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct A2aProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

/// Optional structured handoff context carried with an A2A request.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct A2aContextEnvelope {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Optional A2A request configuration.
#[derive(serde::Deserialize)]
pub struct A2aMessageConfiguration {
    #[serde(default)]
    pub blocking: Option<bool>,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub accepted_output_modes: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub provenance: Option<A2aProvenance>,
    #[serde(default)]
    pub context: Option<A2aContextEnvelope>,
}

/// Structured result metadata emitted alongside synchronous A2A replies.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct A2aResultMetadata {
    pub protocol_version: String,
    pub output_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities_used: Vec<String>,
}

/// A2A synchronous message response payload.
#[derive(serde::Serialize)]
pub struct A2aMessageResponse {
    pub conversation_id: String,
    pub message: A2aOutboundMessage,
    pub result: A2aResultMetadata,
}

pub use crate::contracts::a2a::{
    A2A_MAX_TASKS, A2A_TASK_HARD_TTL_SECS, A2A_TASK_TTL_SECS, A2aOutboundMessage, A2aOutboundPart,
    A2aTask, A2aTaskState,
};

/// Maximum number of unique scope keys per companion state map.
/// Prevents unbounded memory growth from diverse scope keys.
pub const COMPANION_MAX_SCOPES: usize = 1024;

#[cfg(feature = "whatsapp")]
/// `WhatsApp` webhook verification query string parameters.
#[derive(serde::Deserialize)]
pub struct WhatsAppVerifyQuery {
    #[serde(rename = "hub.mode")]
    pub mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}
