//! Axum-based HTTP gateway with proper HTTP/1.1 compliance, body limits, and timeouts.
//!
//! This module replaces the raw TCP implementation with axum for:
//! - Proper HTTP/1.1 parsing and compliance
//! - Content-Length validation (handled by hyper)
//! - Request body size limits (64KB max)
//! - Request timeouts (30s) to prevent slow-loris attacks
//! - Header sanitization (handled by axum/hyper)

mod admin_contract;
mod autosave;
mod companion_bridge;
mod contract;
mod defense;
mod events;
mod handlers;
mod problem_details;
mod replay_guard;
mod server;
mod signature;
pub(crate) mod types;
mod websocket;
pub mod ws_events;
pub(super) mod ws_stream_sink;

// Re-exported for integration tests (tests/persona/scope_regression.rs).
#[cfg(test)]
use axum::http::StatusCode;
#[cfg(test)]
use handlers::{
    a2a_text_message, handle_a2a_message, handle_a2a_tasks_get, handle_agent_card,
    handle_companion_context_ingest, handle_companion_multimodal_ingest,
    handle_companion_surface_caption_emit, handle_companion_surface_request_window_cancel,
    handle_companion_surface_request_window_confirm, handle_companion_surface_request_window_get,
    handle_companion_surface_request_window_open, handle_companion_surface_widget_command,
    handle_health, handle_openapi_contract, handle_pair, handle_ready, handle_webhook,
};
#[cfg(all(test, feature = "whatsapp"))]
use handlers::{handle_whatsapp_message, handle_whatsapp_verify};
#[cfg(test)]
use replay_guard::ReplayGuard;
pub use server::{run_gateway, run_gateway_with_listener, run_gateway_with_profile};
#[cfg(feature = "whatsapp")]
pub use signature::verify_wa_signature;
pub use types::*;

#[cfg(test)]
mod tests;
