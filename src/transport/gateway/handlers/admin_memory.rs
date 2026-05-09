//! Admin endpoints for memory review, correction, forgetting, and checkpointing.

use axum::Json;
use axum::extract::{Path, State, rejection::JsonRejection};
use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::contracts::ids::{EntityId, SlotKey};
use crate::core::memory::checkpoint::{load_checkpoint_registry, save_checkpoint_registry};
use crate::core::memory::{ForgetMode, MemoryCheckpoint, RecallQuery};
use crate::runtime::diagnostics::control_plane_read_models::{
    MemoryConsolidationStatusReadModel, MemoryCorrectionReadModel, MemoryEntityListReadModel,
    MemoryExposureStatusReadModel, MemorySlotListReadModel, SelfAmendmentApprovalReadModel,
    SelfAmendmentReviewReadModel,
};
use crate::runtime::services::{
    approve_self_amendment_candidate, correct_admin_memory_slot, forget_admin_memory_slot,
    list_admin_memory_entities, load_admin_memory_consolidation_statuses,
    load_admin_memory_exposure_status, load_admin_memory_slots,
    load_self_amendment_candidate_review_for_tenant,
};

use super::super::AppState;
use super::super::problem_details::problem_response;
use super::require_management_principal;

#[derive(Debug, Deserialize)]
pub struct CreateCheckpointRequest {
    pub entity_id: EntityId,
    pub label: String,
    pub current_event_count: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct CreateCheckpointResponse {
    pub checkpoint: MemoryCheckpoint,
    pub persisted: bool,
}

#[derive(Debug, Deserialize)]
pub struct MemoryCorrectRequest {
    pub entity_id: EntityId,
    pub slot_key: SlotKey,
    pub old_value: String,
    pub new_value: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct MemoryForgetRequest {
    pub entity_id: EntityId,
    pub slot_key: SlotKey,
    pub reason: String,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SelfAmendmentApproveRequest {
    pub candidate_id: String,
    pub reason: String,
}

pub(crate) async fn create_checkpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateCheckpointRequest>,
) -> Result<Json<CreateCheckpointResponse>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;

    let CreateCheckpointRequest {
        entity_id,
        label,
        current_event_count,
    } = request;
    policy_context
        .enforce_recall_scope(entity_id.as_str())
        .map_err(|error| {
            super::super::problem_details::problem_response(
                StatusCode::FORBIDDEN,
                "tenant_scope_mismatch",
                "Forbidden",
                error.to_string(),
            )
        })?;
    let checkpoint = MemoryCheckpoint {
        checkpoint_id: Uuid::new_v4().to_string(),
        entity_id: entity_id.clone(),
        watermark: current_event_count,
        label,
        created_by: principal,
        created_at: Utc::now().to_rfc3339(),
    };

    let workspace_dir = &state.runtime.config.workspace_dir;
    let persisted = match load_checkpoint_registry(workspace_dir, entity_id.as_str()) {
        Ok(mut registry) => {
            registry.register(checkpoint.clone());
            match save_checkpoint_registry(workspace_dir, entity_id.as_str(), &registry) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "checkpoint registry save failed");
                    false
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "checkpoint registry load failed");
            false
        }
    };

    Ok(Json(CreateCheckpointResponse {
        checkpoint,
        persisted,
    }))
}

pub(crate) async fn list_memory_entities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MemoryEntityListReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;

    let mut entities = list_admin_memory_entities(state.runtime.mem.as_ref())
        .await
        .map_err(|error| {
            tracing::error!(%error, "admin: failed to list memory entities");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_entities_list_failed",
                "Internal Server Error",
                "Failed to list memory entities.".to_string(),
            )
        })?;

    entities.items.retain(|entry| {
        RecallQuery::new(&entry.entity_id, "", 1)
            .with_policy_context(policy_context.clone())
            .enforce_policy()
            .is_ok()
    });
    entities.count = entities.items.len();

    Ok(Json(entities))
}

pub(crate) async fn list_memory_consolidation_statuses(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MemoryConsolidationStatusReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    Ok(Json(load_admin_memory_consolidation_statuses()))
}

pub(crate) async fn get_memory_exposure_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MemoryExposureStatusReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    Ok(Json(load_admin_memory_exposure_status()))
}

pub(crate) async fn list_self_amendment_candidates(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SelfAmendmentReviewReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;
    Ok(Json(load_self_amendment_candidate_review_for_tenant(
        &state.runtime.self_amendment_candidate_review,
        policy_context.tenant_id.as_deref(),
    )))
}

pub(crate) async fn approve_self_amendment_candidate_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Result<Json<SelfAmendmentApproveRequest>, JsonRejection>,
) -> Result<Json<SelfAmendmentApprovalReadModel>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;
    let Json(request) = request.map_err(|rejection| {
        problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_self_amendment_approval_request",
            "Bad Request",
            rejection.body_text(),
        )
    })?;
    let candidate_id = request.candidate_id.trim().to_string();
    let reason = request.reason.trim().to_string();
    if candidate_id.is_empty() || reason.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_self_amendment_approval_request",
            "Bad Request",
            "candidate_id and reason must not be empty.".to_string(),
        ));
    }

    approve_self_amendment_candidate(
        state.runtime.mem.as_ref(),
        &state.runtime.self_amendment_candidate_review,
        &principal,
        &policy_context,
        &candidate_id,
        &reason,
    )
    .await
    .map(Json)
    .map_err(|error| {
        let message = error.to_string();
        if message.contains("candidate not found") {
            return problem_response(
                StatusCode::NOT_FOUND,
                "self_amendment_candidate_not_found",
                "Not Found",
                "No matching self-amendment candidate is available for review.".to_string(),
            );
        }
        if message.contains("tenant") || message.contains("scope") {
            return problem_response(
                StatusCode::FORBIDDEN,
                "self_amendment_scope_denied",
                "Forbidden",
                message,
            );
        }
        tracing::error!(%error, "admin: self-amendment approval failed");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "self_amendment_approval_failed",
            "Internal Server Error",
            "Failed to persist reviewed self-amendment.".to_string(),
        )
    })
}

pub(crate) async fn list_memory_entity_slots(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(entity_id): Path<String>,
) -> Result<Json<MemorySlotListReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;
    RecallQuery::new(&entity_id, "", 1)
        .with_policy_context(policy_context)
        .enforce_policy()
        .map_err(|error| {
            problem_response(
                StatusCode::FORBIDDEN,
                "memory_scope_denied",
                "Forbidden",
                error.to_string(),
            )
        })?;

    let slots = load_admin_memory_slots(state.runtime.mem.as_ref(), &entity_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, %entity_id, "admin: failed to list memory slots");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "memory_slots_list_failed",
                "Internal Server Error",
                "Failed to list memory slots.".to_string(),
            )
        })?;

    Ok(Json(slots))
}

pub(crate) async fn correct_memory_slot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryCorrectRequest>,
) -> Result<Json<MemoryCorrectionReadModel>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;

    let entity_id = request.entity_id;
    let slot_key = request.slot_key.as_str().trim().to_string();
    let old_value = request.old_value;
    let new_value = request.new_value;
    let reason = request.reason.trim().to_string();

    if slot_key.is_empty() || old_value.is_empty() || new_value.is_empty() || reason.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_memory_correction_request",
            "Bad Request",
            "slot_key, old_value, new_value, and reason must not be empty.".to_string(),
        ));
    }
    RecallQuery::new(entity_id.as_str(), "", 1)
        .with_policy_context(policy_context)
        .enforce_policy()
        .map_err(|error| {
            problem_response(
                StatusCode::FORBIDDEN,
                "memory_scope_denied",
                "Forbidden",
                error.to_string(),
            )
        })?;

    let corrected = correct_admin_memory_slot(
        state.runtime.mem.as_ref(),
        &principal,
        entity_id.as_str(),
        &slot_key,
        &old_value,
        &new_value,
        &reason,
    )
    .await
    .map_err(|error| {
        let message = error.to_string();
        if message.contains("slot not found") {
            return problem_response(
                StatusCode::NOT_FOUND,
                "memory_slot_not_found",
                "Not Found",
                format!("No slot found for {entity_id}:{slot_key}."),
            );
        }
        if message.contains("matches old_value") {
            return problem_response(
                StatusCode::CONFLICT,
                "memory_slot_changed",
                "Conflict",
                "Current slot value no longer matches old_value exactly.".to_string(),
            );
        }
        tracing::error!(%error, entity_id = %entity_id, slot_key, "admin: memory correction failed");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "memory_correction_failed",
            "Internal Server Error",
            "Failed to persist corrected memory.".to_string(),
        )
    })?;

    Ok(Json(corrected))
}

pub(crate) async fn forget_memory_slot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MemoryForgetRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(&state, &headers)?;
    let policy_context = super::request_management_policy_context(&state, &headers)?;

    let entity_id = request.entity_id;
    let slot_key = request.slot_key.as_str().trim().to_string();
    let reason = request.reason.trim().to_string();
    if slot_key.is_empty() || reason.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_memory_forget_request",
            "Bad Request",
            "slot_key and reason must not be empty.".to_string(),
        ));
    }
    RecallQuery::new(entity_id.as_str(), "", 1)
        .with_policy_context(policy_context)
        .enforce_policy()
        .map_err(|error| {
            problem_response(
                StatusCode::FORBIDDEN,
                "memory_scope_denied",
                "Forbidden",
                error.to_string(),
            )
        })?;
    let mode = parse_forget_mode(request.mode.as_deref())?;
    let outcome = forget_admin_memory_slot(
        state.runtime.mem.as_ref(),
        &principal,
        entity_id.as_str(),
        &slot_key,
        mode,
        &reason,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, entity_id = %entity_id, slot_key, "admin: memory forget failed");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "memory_forget_failed",
            "Internal Server Error",
            "Failed to apply memory forget action.".to_string(),
        )
    })?;

    Ok(Json(serde_json::to_value(outcome).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

fn parse_forget_mode(
    mode: Option<&str>,
) -> Result<ForgetMode, (StatusCode, Json<serde_json::Value>)> {
    match mode.unwrap_or("soft").trim().to_ascii_lowercase().as_str() {
        "soft" => Ok(ForgetMode::Soft),
        "hard" => Ok(ForgetMode::Hard),
        "tombstone" => Ok(ForgetMode::Tombstone),
        _ => Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_forget_mode",
            "Bad Request",
            "mode must be one of soft, hard, or tombstone.".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_forget_mode;
    use crate::core::memory::ForgetMode;

    #[test]
    fn parse_forget_mode_defaults_to_soft() {
        assert!(matches!(parse_forget_mode(None).unwrap(), ForgetMode::Soft));
        assert!(matches!(
            parse_forget_mode(Some("tombstone")).unwrap(),
            ForgetMode::Tombstone
        ));
    }
}
