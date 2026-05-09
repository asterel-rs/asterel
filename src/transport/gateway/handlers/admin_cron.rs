use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use uuid::Uuid;

use super::super::AppState;
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use super::super::ws_events::CronRunUpdatedPayload;
use super::companion_helpers::publish_gateway_event;
use super::require_management_principal;
use crate::contracts::ids::RunId;
use crate::platform::cron::{self, CronCommandValidationError, validate_main_runtime_cron_command};

fn map_command_validation_error(
    error: CronCommandValidationError,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        error.code(),
        "Bad Request",
        error.message().to_string(),
    )
}

pub(crate) async fn handle_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let jobs = cron::list_jobs(&state.runtime.config).map_err(|error| {
        tracing::error!(%error, "admin: failed to list cron jobs");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cron_list_failed",
            "Internal Server Error",
            error.to_string(),
        )
    })?;

    let items: Vec<serde_json::Value> = jobs.iter().map(cron_job_json).collect();

    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(serde::Deserialize)]
pub(crate) struct CronJobCreateBody {
    pub expression: String,
    pub command: String,
    pub enabled: Option<bool>,
}

pub(crate) async fn handle_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CronJobCreateBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let expression = body.expression.trim();
    let command = body.command.trim();

    if expression.is_empty() || command.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "missing_fields",
            "Bad Request",
            "Both expression and command are required.".to_string(),
        ));
    }
    validate_main_runtime_cron_command(command).map_err(map_command_validation_error)?;

    let job = cron::add_job(&state.runtime.config, expression, command).map_err(|error| {
        tracing::error!(%error, "admin: failed to create cron job");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cron_create_failed",
            "Internal Server Error",
            error.to_string(),
        )
    })?;

    let job = if body.enabled == Some(false) {
        cron::update_job(&state.runtime.config, &job.id, None, None, Some(false)).map_err(
            |error| {
                tracing::error!(%error, job_id = %job.id, "admin: failed to disable cron job after create");
                problem_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cron_create_failed",
                    "Internal Server Error",
                    error.to_string(),
                )
            },
        )?
    } else {
        job
    };

    Ok((StatusCode::CREATED, Json(cron_job_json(&job))))
}

pub(crate) async fn handle_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    cron::remove_job(&state.runtime.config, &job_id).map_err(|error| {
        tracing::error!(%error, %job_id, "admin: failed to remove cron job");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cron_remove_failed",
            "Internal Server Error",
            error.to_string(),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "removed",
        "id": job_id,
    })))
}

fn cron_job_json(job: &cron::CronJob) -> serde_json::Value {
    serde_json::json!({
        "id": job.id,
        "enabled": job.enabled,
        "expression": job.expression,
        "command": job.command,
        "next_run": job.next_run.to_rfc3339(),
        "last_run": job.last_run.map(|d| d.to_rfc3339()),
        "last_status": job.last_status,
        "job_kind": job.job_kind.as_db(),
        "origin": job.origin.as_db(),
        "expires_at": job.expires_at.map(|d| d.to_rfc3339()),
        "max_attempts": job.max_attempts,
        "consecutive_failures": job.consecutive_failures,
        "breaker_open_until": job.breaker_open_until.map(|d| d.to_rfc3339()),
    })
}

#[derive(serde::Deserialize)]
pub(crate) struct CronJobUpdateBody {
    #[serde(alias = "expression")]
    pub schedule: Option<String>,
    pub command: Option<String>,
    pub enabled: Option<bool>,
}

pub(crate) async fn handle_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(body): Json<CronJobUpdateBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    if let Some(command) = body.command.as_deref() {
        validate_main_runtime_cron_command(command).map_err(map_command_validation_error)?;
    }

    let updated = cron::update_job(
        &state.runtime.config,
        &job_id,
        body.schedule.as_deref(),
        body.command.as_deref(),
        body.enabled,
    )
    .map_err(|error| {
        let error_message = error.to_string();
        let status = if error_message.contains("not found") {
            StatusCode::NOT_FOUND
        } else if error_message.contains("must not be empty") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        problem_response(
            status,
            "cron_update_failed",
            "Request Failed",
            error_message,
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "updated",
        "job": cron_job_json(&updated),
    })))
}

pub(crate) async fn handle_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let job = cron::list_jobs(&state.runtime.config)
        .map_err(|error| {
            tracing::error!(%error, "admin: failed to list cron jobs before manual run");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "cron_run_failed",
                "Internal Server Error",
                error.to_string(),
            )
        })?
        .into_iter()
        .find(|job| job.id == job_id)
        .ok_or_else(|| {
            problem_response(
                StatusCode::NOT_FOUND,
                "cron_job_not_found",
                "Not Found",
                format!("Cron job '{job_id}' not found."),
            )
        })?;

    let run_id = RunId::new(format!("cron_manual_{}", Uuid::new_v4().simple()));
    let started_at = chrono::Utc::now().to_rfc3339();
    publish_gateway_event(
        &state,
        ServerMessage::cron_run_updated(CronRunUpdatedPayload {
            job_id: job.id.clone(),
            run_id: run_id.clone(),
            status: "running".to_string(),
            detail: None,
            started_at: Some(started_at.clone()),
            finished_at: None,
        }),
    );

    let (success, output) =
        cron::scheduler::run_job_once(&state.runtime.config, state.runtime.security.as_ref(), &job)
            .await;
    cron::reschedule_after_run(&state.runtime.config, &job, success, &output).map_err(|error| {
        tracing::error!(%error, job_id = %job.id, "admin: failed to persist cron manual run");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cron_run_failed",
            "Internal Server Error",
            error.to_string(),
        )
    })?;

    let finished_at = chrono::Utc::now().to_rfc3339();
    publish_gateway_event(
        &state,
        ServerMessage::cron_run_updated(CronRunUpdatedPayload {
            job_id: job.id.clone(),
            run_id: run_id.clone(),
            status: if success {
                "completed".to_string()
            } else {
                "failed".to_string()
            },
            detail: Some(output.clone()),
            started_at: Some(started_at.clone()),
            finished_at: Some(finished_at.clone()),
        }),
    );

    Ok(Json(serde_json::json!({
        "status": if success { "completed" } else { "failed" },
        "job_id": job.id,
        "run_id": run_id,
        "started_at": started_at,
        "finished_at": finished_at,
        "output": output,
    })))
}

pub(crate) use handle_create as handle_admin_cron_create;
pub(crate) use handle_list as handle_admin_cron_list;
pub(crate) use handle_remove as handle_admin_cron_remove;
pub(crate) use handle_run as handle_admin_cron_run;
pub(crate) use handle_update as handle_admin_cron_update;
