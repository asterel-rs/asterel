//! Companion browser extension handlers: context ingestion (tab text,
//! clipboard, etc.) and multimodal ingestion (screenshots, audio clips).
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};

use super::super::AppState;
use super::super::companion_bridge::{
    CompanionContextBridgeInput, CompanionContextIngressDecision, CompanionCtxEvent,
    CompanionMultimodalMemoryRecord, build_context_event,
};
use super::super::defense::policy_accounting_response;
use super::super::events::{
    CompanionContextIngressEvent, CompanionMultimodalIngressEvent, ServerMessage,
};
use super::super::problem_details::problem_response;
use super::companion_helpers::{
    companion_context_content, companion_context_ingest_failed,
    companion_context_ingest_invalid_payload, companion_context_ingress_reason_label,
    companion_context_source_kind, companion_multimodal_ingest_invalid_payload,
    companion_replay_scope, companion_scope_key, normalize_optional_entity_id,
    publish_gateway_event,
};
use super::{
    CompanionContextIngestPayload, CompanionMultimodalIngestPayload, enforce_entity_rate_limit,
    enforce_json_request_guards, gateway_entity_id, request_policy_context,
    verified_source_identifier_from_headers, webhook_replay_ack_response,
};
use crate::contracts::ids::EntityId;
use crate::core::memory::{
    IngestionError, IngestionPipeline, MemoryError, SignalEnvelope, SignalTier,
};

pub(super) type GatewayJsonResponse = (StatusCode, Json<serde_json::Value>);

/// Build a `SignalEnvelope` from a validated companion context event.
fn build_context_signal_envelope(
    event: &CompanionCtxEvent,
    entity_id: EntityId,
    signal_tier: SignalTier,
    dedupe_key: &str,
) -> SignalEnvelope {
    let source_kind = companion_context_source_kind(event.kind);
    let source_ref = format!(
        "companion.context/{}/{}/{}/{}",
        event.session_id,
        event.tab_id,
        event.kind.as_str(),
        event.topic
    );
    let mut envelope = SignalEnvelope::new(
        source_kind,
        source_ref,
        companion_context_content(event),
        entity_id,
    )
    .with_signal_tier(signal_tier)
    .with_metadata("companion_context_kind", event.kind.as_str())
    .with_metadata("companion_context_topic", &event.topic)
    .with_metadata("companion_context_source", &event.source)
    .with_metadata("companion_context_dedupe_key", dedupe_key);

    if let Some(source_url) = event.source_url.as_deref() {
        envelope = envelope.with_metadata("companion_context_source_url", source_url);
    }
    if let Some(media_ref) = event.media_ref.as_deref() {
        envelope = envelope.with_metadata("companion_context_media_ref", media_ref);
    }
    envelope
}

/// Roll back replay guard and dedupe gate state after a failed ingestion.
async fn rollback_context_ingest(
    state: &AppState,
    replay_scope: &str,
    body: &Bytes,
    scope_key: &str,
    dedupe_key: &str,
) {
    state
        .companion
        .replay_guard
        .forget_scoped(replay_scope, body);
    let Some(gate_handle) = state
        .companion
        .companion_context_gates
        .get_scope(scope_key)
        .await
    else {
        return;
    };
    let should_remove_scope = {
        let mut gate = gate_handle.lock().await;
        gate.forget(dedupe_key);
        gate.tracked_entries() == 0
    };
    if should_remove_scope {
        let _ = state
            .companion
            .companion_context_gates
            .remove_scope(scope_key)
            .await;
    }
}

use crate::security::policy::TenantPolicyContext;

struct ParsedContextPayload {
    entity_id: Option<EntityId>,
    signal_tier: Option<SignalTier>,
}

/// Build a `CompanionContextIngressEvent` from the event and decision.
fn build_context_ingress_event(
    event: &CompanionCtxEvent,
    decision: &CompanionContextIngressDecision,
    accepted: bool,
    slot_key: Option<String>,
    signal_tier: Option<String>,
) -> CompanionContextIngressEvent {
    CompanionContextIngressEvent {
        session_id: event.session_id.clone(),
        tab_id: event.tab_id.clone(),
        kind: event.kind.as_str().to_string(),
        topic: event.topic.clone(),
        source: event.source.clone(),
        accepted,
        reason: companion_context_ingress_reason_label(decision.reason).to_string(),
        dedupe_key: decision.dedupe_key.clone(),
        slot_key,
        signal_tier,
    }
}

/// Parse a companion context payload and validate it into an event.
fn parse_context_payload_and_event(
    body: &Bytes,
) -> Result<(ParsedContextPayload, CompanionCtxEvent), GatewayJsonResponse> {
    let payload =
        serde_json::from_slice::<CompanionContextIngestPayload>(body).map_err(|error| {
            tracing::debug!(%error, "companion context ingest: JSON parse failed");
            companion_context_ingest_invalid_payload(
                "Invalid JSON payload. Expected companion context ingest format".to_string(),
            )
        })?;

    let CompanionContextIngestPayload {
        session_id,
        tab_id,
        kind,
        topic,
        source,
        source_url,
        media_ref,
        payload,
        entity_id,
        signal_tier,
    } = payload;

    let event = build_context_event(CompanionContextBridgeInput {
        session_id,
        tab_id,
        kind,
        topic,
        source,
        source_url,
        media_ref,
        payload,
    })
    .map_err(companion_context_ingest_invalid_payload)?;

    Ok((
        ParsedContextPayload {
            entity_id,
            signal_tier,
        },
        event,
    ))
}

/// Run dedupe gating for a validated companion context event.
async fn dedupe_context_event(
    state: &AppState,
    scope_key: &str,
    event: &CompanionCtxEvent,
    producer_identity: &str,
) -> Result<CompanionContextIngressDecision, GatewayJsonResponse> {
    let Some(gate_handle) = state
        .companion
        .companion_context_gates
        .get_or_insert_default(scope_key, super::super::COMPANION_MAX_SCOPES)
        .await
    else {
        return Err(problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "capacity_exceeded",
            "Too Many Requests",
            "Companion context gate scope limit exceeded. Try again later.",
        ));
    };
    let mut gate = gate_handle.lock().await;
    gate.ingest_for_producer(event, producer_identity, chrono::Utc::now())
        .map_err(|error| companion_context_ingest_invalid_payload(error.to_string()))
}

/// Resolve the entity ID for a companion context ingest request.
fn resolve_context_entity_id(
    state: &AppState,
    payload_entity_id: Option<&str>,
    headers: &HeaderMap,
    policy_context: &TenantPolicyContext,
) -> Result<EntityId, GatewayJsonResponse> {
    let source_identifier =
        verified_source_identifier_from_headers(state, headers, "companion-context");
    let default_entity_id = gateway_entity_id(&source_identifier, policy_context);
    let entity_id = normalize_optional_entity_id(payload_entity_id).unwrap_or(default_entity_id);

    if let Err(error) = policy_context.enforce_recall_scope(entity_id.as_str()) {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_violation",
            "Forbidden",
            error,
        ));
    }

    Ok(entity_id)
}

fn companion_ingestion_error_response(error: IngestionError) -> GatewayJsonResponse {
    match error {
        IngestionError::Validation(message) => companion_context_ingest_invalid_payload(message),
        IngestionError::Policy(message) => problem_response(
            StatusCode::FORBIDDEN,
            "companion_context_ingest_policy_rejected",
            "Forbidden",
            message,
        ),
        IngestionError::State(message) => problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "companion_context_ingest_state_failed",
            "Internal Server Error",
            message,
        ),
        IngestionError::Persistence(memory_error) => memory_ingestion_error_response(memory_error),
        IngestionError::Other(error) => companion_context_ingest_failed(error.to_string()),
    }
}

fn memory_ingestion_error_response(error: MemoryError) -> GatewayJsonResponse {
    match error {
        MemoryError::Validation(message) => companion_context_ingest_invalid_payload(message),
        MemoryError::Policy(message) => problem_response(
            StatusCode::FORBIDDEN,
            "companion_context_memory_policy_rejected",
            "Forbidden",
            message,
        ),
        MemoryError::BackendUnavailable(message) => problem_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "companion_context_memory_unavailable",
            "Service Unavailable",
            message,
        ),
        MemoryError::Unsupported(message) => problem_response(
            StatusCode::NOT_IMPLEMENTED,
            "companion_context_memory_unsupported",
            "Not Implemented",
            message,
        ),
        MemoryError::Query(message)
        | MemoryError::Write(message)
        | MemoryError::Integrity(message) => problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "companion_context_memory_failed",
            "Internal Server Error",
            message,
        ),
        MemoryError::Other(error) => companion_context_ingest_failed(error.to_string()),
    }
}

/// Resolve the entity ID for a companion multimodal ingest request.
fn resolve_multimodal_entity_id(
    state: &AppState,
    payload_entity_id: Option<&str>,
    headers: &HeaderMap,
    policy_context: &TenantPolicyContext,
) -> Result<EntityId, GatewayJsonResponse> {
    let source_identifier =
        verified_source_identifier_from_headers(state, headers, "companion-multimodal");
    let default_entity_id = gateway_entity_id(&source_identifier, policy_context);
    let entity_id = normalize_optional_entity_id(payload_entity_id).unwrap_or(default_entity_id);

    if let Err(error) = policy_context.enforce_recall_scope(entity_id.as_str()) {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_violation",
            "Forbidden",
            error,
        ));
    }

    Ok(entity_id)
}

/// Parse the companion multimodal ingest payload.
fn parse_multimodal_payload(
    body: &Bytes,
) -> Result<CompanionMultimodalIngestPayload, GatewayJsonResponse> {
    serde_json::from_slice::<CompanionMultimodalIngestPayload>(body).map_err(|error| {
        tracing::debug!(%error, "companion multimodal ingest: JSON parse failed");
        companion_multimodal_ingest_invalid_payload(
            "Invalid JSON payload. Expected companion multimodal ingest format".to_string(),
        )
    })
}

/// Build a multimodal memory record from a validated payload.
fn build_multimodal_record(
    payload: CompanionMultimodalIngestPayload,
    entity_id: EntityId,
) -> Result<CompanionMultimodalMemoryRecord, GatewayJsonResponse> {
    let mut record = CompanionMultimodalMemoryRecord::new(
        entity_id,
        payload.source_ref,
        payload.media_kind,
        payload.descriptors,
    )
    .map_err(|error| companion_multimodal_ingest_invalid_payload(error.to_string()))?;
    if let Some(transcript) = payload.transcript {
        record = record.with_transcript(transcript);
    }
    if let Some(emotional_impact) = payload.emotional_impact {
        record = record.with_emotional_impact(emotional_impact);
    }
    Ok(record)
}

/// Perform the ingestion and return the HTTP response.
async fn run_context_ingestion(
    state: &AppState,
    event: CompanionCtxEvent,
    dedupe_decision: &CompanionContextIngressDecision,
    envelope: SignalEnvelope,
    scope_key: String,
    replay_scope: &str,
    body: &Bytes,
) -> GatewayJsonResponse {
    match state
        .companion
        .companion_context_ingestion
        .ingest(envelope)
        .await
    {
        Ok(ingestion_result) => {
            publish_gateway_event(
                state,
                ServerMessage::companion_context_ingress(
                    scope_key,
                    build_context_ingress_event(
                        &event,
                        dedupe_decision,
                        ingestion_result.accepted,
                        Some(ingestion_result.slot_key.to_string()),
                        Some(ingestion_result.signal_tier.to_string()),
                    ),
                ),
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "accepted": ingestion_result.accepted,
                    "slot_key": ingestion_result.slot_key,
                    "signal_tier": ingestion_result.signal_tier,
                    "reason": ingestion_result.reason,
                    "dedupe_key": dedupe_decision.dedupe_key
                })),
            )
        }
        Err(error) => {
            rollback_context_ingest(
                state,
                replay_scope,
                body,
                &scope_key,
                &dedupe_decision.dedupe_key,
            )
            .await;
            companion_ingestion_error_response(error)
        }
    }
}

pub(super) async fn ingest_context_request(
    state: &AppState,
    headers: &HeaderMap,
    body: Bytes,
) -> GatewayJsonResponse {
    if let Some(response) = enforce_json_request_guards(state, headers) {
        return response;
    }

    let policy_context = match request_policy_context(state, headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_replay_scope("companion_context_ingest", &scope_key);

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion context ingest replay detected");
        return webhook_replay_ack_response();
    }

    let (payload, event) = match parse_context_payload_and_event(&body) {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let producer_identity =
        verified_source_identifier_from_headers(state, headers, "companion-context");

    let dedupe_decision =
        match dedupe_context_event(state, &scope_key, &event, &producer_identity).await {
            Ok(decision) => decision,
            Err(response) => return response,
        };

    if !dedupe_decision.accepted {
        publish_gateway_event(
            state,
            ServerMessage::companion_context_ingress(
                scope_key,
                build_context_ingress_event(&event, &dedupe_decision, false, None, None),
            ),
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "duplicate_ignored",
                "dedupe_key": dedupe_decision.dedupe_key,
            })),
        );
    }

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(state, headers, &policy_context) {
        return response;
    }

    let entity_id = match resolve_context_entity_id(
        state,
        payload.entity_id.as_ref().map(EntityId::as_str),
        headers,
        &policy_context,
    ) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let envelope = build_context_signal_envelope(
        &event,
        entity_id,
        payload.signal_tier.unwrap_or(SignalTier::Raw),
        &dedupe_decision.dedupe_key,
    );

    run_context_ingestion(
        state,
        event,
        &dedupe_decision,
        envelope,
        scope_key,
        &replay_scope,
        &body,
    )
    .await
}

pub(super) async fn ingest_multimodal_request(
    state: &AppState,
    headers: &HeaderMap,
    body: Bytes,
) -> GatewayJsonResponse {
    if let Some(response) = enforce_json_request_guards(state, headers) {
        return response;
    }

    let policy_context = match request_policy_context(state, headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_replay_scope("companion_multimodal_ingest", &scope_key);

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion multimodal ingest replay detected");
        return webhook_replay_ack_response();
    }

    let payload = match parse_multimodal_payload(&body) {
        Ok(payload) => payload,
        Err(response) => return response,
    };

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(state, headers, &policy_context) {
        return response;
    }

    let entity_id = match resolve_multimodal_entity_id(
        state,
        payload.entity_id.as_ref().map(EntityId::as_str),
        headers,
        &policy_context,
    ) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let record = match build_multimodal_record(payload, entity_id) {
        Ok(record) => record,
        Err(response) => return response,
    };

    let record_id = record.record_id.clone();
    let envelope = match record.to_signal_envelope() {
        Ok(envelope) => envelope,
        Err(error) => return companion_multimodal_ingest_invalid_payload(error.to_string()),
    };

    match state
        .companion
        .companion_context_ingestion
        .ingest(envelope)
        .await
    {
        Ok(ingestion_result) => {
            publish_gateway_event(
                state,
                ServerMessage::companion_multimodal_ingress(
                    scope_key,
                    CompanionMultimodalIngressEvent {
                        record_id: record_id.clone(),
                        media_kind: record.media_kind.as_str().to_string(),
                        source_ref: record.source_ref.clone(),
                        accepted: ingestion_result.accepted,
                        slot_key: ingestion_result.slot_key.clone(),
                        signal_tier: ingestion_result.signal_tier.to_string(),
                        reason: ingestion_result.reason.clone(),
                    },
                ),
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "record_id": record_id,
                    "accepted": ingestion_result.accepted,
                    "slot_key": ingestion_result.slot_key,
                    "signal_tier": ingestion_result.signal_tier,
                    "reason": ingestion_result.reason
                })),
            )
        }
        Err(error) => {
            state
                .companion
                .replay_guard
                .forget_scoped(&replay_scope, &body);
            companion_ingestion_error_response(error)
        }
    }
}

/// POST /companion/context/ingest — companion context ingress with dedupe + memory ingestion
pub(in super::super) async fn handle_companion_context_ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    ingest_context_request(&state, &headers, body).await
}

/// POST /companion/multimodal/ingest — companion multimodal memory ingress
pub(in super::super) async fn handle_companion_multimodal_ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    ingest_multimodal_request(&state, &headers, body).await
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::companion_ingestion_error_response;
    use crate::core::memory::{IngestionError, MemoryError};

    #[test]
    fn context_ingestion_validation_error_maps_to_bad_request() {
        let (status, body) =
            companion_ingestion_error_response(IngestionError::validation("empty signal"));

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.0["code"], "invalid_companion_context_ingest_payload");
    }

    #[test]
    fn context_ingestion_backend_unavailable_maps_to_service_unavailable() {
        let (status, body) = companion_ingestion_error_response(IngestionError::Persistence(
            MemoryError::backend_unavailable("postgres unavailable"),
        ));

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["code"], "companion_context_memory_unavailable");
    }
}
