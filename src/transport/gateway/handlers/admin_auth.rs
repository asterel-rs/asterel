use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::problem_details::problem_response;
use super::require_management_principal;
use crate::runtime::services::{
    AdminStateError, load_admin_auth_profile, load_admin_auth_profiles,
    set_admin_auth_profile_disabled,
};
use crate::security::auth::AuthProfile;

type AdminProblem = (StatusCode, Json<serde_json::Value>);

fn auth_profiles_load_failed(error: &AdminStateError) -> AdminProblem {
    tracing::error!(%error, "admin: failed to load auth profiles");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "auth_profiles_load_failed",
        "Internal Server Error",
        "Failed to load auth profiles.",
    )
}

fn auth_profiles_save_failed(error: &AdminStateError) -> AdminProblem {
    tracing::error!(%error, "admin: failed to save auth profiles");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "auth_profiles_save_failed",
        "Internal Server Error",
        "Failed to save auth profiles.",
    )
}

fn auth_profile_store_failed(error: &AdminStateError) -> AdminProblem {
    match error {
        AdminStateError::AuthProfilesLoad { .. } => auth_profiles_load_failed(error),
        _ => auth_profiles_save_failed(error),
    }
}

pub(crate) async fn handle_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let profile_store = load_admin_auth_profiles(&state.runtime.config)
        .map_err(|error| auth_profiles_load_failed(&error))?;

    let items: Vec<serde_json::Value> = profile_store
        .profiles
        .iter()
        .map(redacted_profile_json)
        .collect();

    Ok(Json(serde_json::json!({
        "items": items,
        "defaults": profile_store.defaults,
    })))
}

fn redacted_profile_json(profile: &AuthProfile) -> serde_json::Value {
    serde_json::json!({
        "id": profile.id,
        "provider": profile.provider,
        "auth_route": profile.auth_route,
        "label": profile.label,
        "has_api_key": profile.api_key.is_some(),
        "auth_scheme": profile.auth_scheme,
        "oauth_source": profile.oauth_source,
        "disabled": profile.is_disabled,
    })
}

/// `PATCH /admin/v1/auth/profiles/{id}` — update auth profile (toggle disabled).
#[derive(serde::Deserialize)]
pub(crate) struct PatchAuthProfileBody {
    pub disabled: Option<bool>,
}

pub(crate) async fn handle_patch_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    Json(body): Json<PatchAuthProfileBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let profile = if let Some(disabled) = body.disabled {
        set_admin_auth_profile_disabled(&state.runtime.config, &profile_id, disabled)
            .map_err(|error| auth_profile_store_failed(&error))?
    } else {
        load_admin_auth_profile(&state.runtime.config, &profile_id)
            .map_err(|error| auth_profiles_load_failed(&error))?
    };

    let Some(profile) = profile else {
        return Err(problem_response(
            StatusCode::NOT_FOUND,
            "auth_profile_not_found",
            "Not Found",
            format!("Auth profile '{profile_id}' not found."),
        ));
    };

    let mut changes: Vec<&'static str> = Vec::new();
    if body.disabled.is_some() {
        changes.push("disabled");
    }

    let response = redacted_profile_json(&profile);

    Ok(Json(serde_json::json!({
        "profile": response,
        "changes": changes,
        "status": "updated",
    })))
}

pub(crate) use handle_patch_profile as handle_admin_auth_profile_patch;
pub(crate) use handle_profiles as handle_admin_auth_profiles;
