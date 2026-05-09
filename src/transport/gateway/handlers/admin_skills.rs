use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use super::super::ws_events::RuntimeUpdatedPayload;
use super::companion_helpers::publish_gateway_event;
use super::require_management_principal;
use crate::runtime::services::{
    install_admin_skill, list_admin_skills, remove_admin_skill, update_admin_skill,
};

fn skill_problem(
    code: &'static str,
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    let detail = error.to_string();
    let status = if detail.contains("not found") {
        StatusCode::NOT_FOUND
    } else if detail.contains("missing")
        || detail.contains("required")
        || detail.contains("disabled")
        || detail.contains("Source path")
        || detail.contains("Invalid skill")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    problem_response(
        status,
        code,
        status.canonical_reason().unwrap_or("Error"),
        detail,
    )
}

pub(crate) async fn handle_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let metadata =
        list_admin_skills(&state.runtime.config, &state.runtime.security).map_err(|error| {
            tracing::error!(%error, "admin: failed to list skills");
            skill_problem("skill_list_failed", &error)
        })?;

    let items: Vec<serde_json::Value> = metadata
        .iter()
        .map(|skill| {
            serde_json::json!({
                "name": skill.name,
                "description": skill.description,
                "version": skill.version,
                "author": skill.author,
                "tags": skill.tags,
                "enabled": skill.enabled,
                "tools": skill.tools.iter().map(|t| serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "kind": t.kind,
                })).collect::<Vec<_>>(),
                "location": skill.location,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "items": items,
        "count": items.len(),
    })))
}

#[derive(serde::Deserialize)]
pub(crate) struct SkillInstallBody {
    pub source: String,
}

pub(crate) async fn handle_install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SkillInstallBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let source = body.source.trim();
    if source.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "missing_source",
            "Bad Request",
            "Installation source is required.".to_string(),
        ));
    }

    install_admin_skill(&state.runtime.config, &state.runtime.security, source).map_err(
        |error| {
            tracing::error!(%error, "admin: failed to install skill");
            skill_problem("skill_install_failed", &error)
        },
    )?;

    Ok(Json(serde_json::json!({
        "status": "installed",
        "source": source,
    })))
}

pub(crate) async fn handle_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    remove_admin_skill(&state.runtime.config, &state.runtime.security, &name).map_err(|error| {
        tracing::error!(%error, %name, "admin: failed to remove skill");
        skill_problem("skill_remove_failed", &error)
    })?;

    Ok(Json(serde_json::json!({
        "status": "removed",
        "name": name,
    })))
}

#[derive(serde::Deserialize)]
pub(crate) struct SkillUpdateBody {
    pub enabled: Option<bool>,
}

pub(crate) async fn handle_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(skill_id): Path<String>,
    Json(body): Json<SkillUpdateBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let Some(enabled) = body.enabled else {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "missing_skill_enabled_state",
            "Bad Request",
            "Skill update requires an explicit enabled flag.".to_string(),
        ));
    };

    let result = update_admin_skill(
        &state.runtime.config,
        &state.runtime.security,
        &skill_id,
        enabled,
    )
    .map_err(|error| {
        tracing::error!(%error, %skill_id, "admin: failed to update skill");
        skill_problem("skill_update_failed", &error)
    })?;
    publish_gateway_event(
        &state,
        ServerMessage::runtime_updated(RuntimeUpdatedPayload {
            component: "skills".to_string(),
            status: "updated".to_string(),
            detail: Some(format!(
                "skill '{}' {}",
                result.skill_id,
                if result.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            )),
        }),
    );

    Ok(Json(serde_json::json!({
        "status": "updated",
        "skill_id": result.skill_id,
        "enabled": result.enabled,
        "changes": result.changes,
        "apply_mode": result.apply_mode.as_str(),
    })))
}

pub(crate) use handle_install as handle_admin_skills_install;
pub(crate) use handle_list as handle_admin_skills_list;
pub(crate) use handle_remove as handle_admin_skills_remove;
pub(crate) use handle_update as handle_admin_skills_update;
