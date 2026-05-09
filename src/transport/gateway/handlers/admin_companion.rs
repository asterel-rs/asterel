use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Json;

use super::super::AppState;
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use super::super::ws_events::RuntimeUpdatedPayload;
use super::companion::{ingest_context_request, ingest_multimodal_request};
use super::companion_helpers::{
    companion_admin_scope_key, enforce_companion_admin_scope, publish_gateway_event,
};
use super::{request_management_policy_context, require_management_principal};
use crate::config::CompanionBehaviorConfig;
use crate::runtime::services::{load_companion_admin_settings, save_companion_admin_settings};

fn companion_settings_storage_error(
    error: &anyhow::Error,
    code: &'static str,
    detail: &'static str,
) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!(%error, "admin: companion settings persistence failed");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        "Internal Server Error",
        detail,
    )
}

pub(crate) async fn handle_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    let allowed_scope = companion_admin_scope_key(&policy_context)?;

    let caption_keys = state.companion.companion_caption_logs.scope_keys().await;
    let widget_keys = state.companion.companion_widget_runtimes.scope_keys().await;
    let window_keys = state.companion.companion_request_windows.scope_keys().await;
    let gate_keys = state.companion.companion_context_gates.scope_keys().await;

    let mut all_scopes = std::collections::BTreeSet::new();
    all_scopes.extend(caption_keys);
    all_scopes.extend(widget_keys);
    all_scopes.extend(window_keys);
    all_scopes.extend(gate_keys);
    all_scopes.retain(|scope| scope == &allowed_scope);

    let mut items = Vec::new();
    for scope in &all_scopes {
        let caption_count = match state
            .companion
            .companion_caption_logs
            .get_scope(scope)
            .await
        {
            Some(log) => log.lock().await.len(),
            None => 0,
        };
        let widget_count = match state
            .companion
            .companion_widget_runtimes
            .get_scope(scope)
            .await
        {
            Some(rt) => rt.lock().await.snapshot().len(),
            None => 0,
        };
        let window_count = match state
            .companion
            .companion_request_windows
            .get_scope(scope)
            .await
        {
            Some(wins) => wins.lock().await.len(),
            None => 0,
        };
        let gate_entries = match state
            .companion
            .companion_context_gates
            .get_scope(scope)
            .await
        {
            Some(gate) => gate.lock().await.tracked_entries(),
            None => 0,
        };

        items.push(serde_json::json!({
            "scope": scope,
            "captions": caption_count,
            "widgets": widget_count,
            "windows": window_count,
            "context_gate_entries": gate_entries,
        }));
    }

    let settings = state.companion.settings.read().await.clone();

    Ok(Json(serde_json::json!({
        "items": items,
        "count": items.len(),
        "settings": {
            "caption_retention_limit": settings.caption_retention_limit,
            "behavior": settings.behavior,
            "config": settings.config,
        }
    })))
}

pub(crate) async fn handle_captions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    enforce_companion_admin_scope(&scope, &policy_context)?;

    let Some(log) = state
        .companion
        .companion_caption_logs
        .get_scope(&scope)
        .await
    else {
        return Ok(Json(serde_json::json!({
            "scope": scope,
            "items": [],
        })));
    };

    let items: Vec<serde_json::Value> = log
        .lock()
        .await
        .iter()
        .map(|evt| {
            serde_json::json!({
                "caption_id": evt.caption_id,
                "channel": evt.channel,
                "sequence": evt.sequence,
                "text": evt.text,
                "emitted_at": evt.emitted_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "scope": scope,
        "items": items,
    })))
}

pub(crate) async fn handle_widgets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    enforce_companion_admin_scope(&scope, &policy_context)?;

    let Some(rt) = state
        .companion
        .companion_widget_runtimes
        .get_scope(&scope)
        .await
    else {
        return Ok(Json(serde_json::json!({
            "scope": scope,
            "items": [],
        })));
    };

    let widgets = rt.lock().await.snapshot();
    let items: Vec<serde_json::Value> = widgets
        .iter()
        .map(|w| {
            serde_json::json!({
                "widget_id": w.widget_id,
                "payload": w.payload,
                "created_at": w.created_at,
                "updated_at": w.updated_at,
                "expires_at": w.expires_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "scope": scope,
        "items": items,
    })))
}

pub(crate) async fn handle_windows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    enforce_companion_admin_scope(&scope, &policy_context)?;

    let Some(wins) = state
        .companion
        .companion_request_windows
        .get_scope(&scope)
        .await
    else {
        return Ok(Json(serde_json::json!({
            "scope": scope,
            "items": [],
        })));
    };

    let guard = wins.lock().await;
    let mut items: Vec<serde_json::Value> = guard
        .values()
        .map(|w| {
            serde_json::json!({
                "window_id": w.window_id,
                "requested_action": w.requested_action,
                "created_at": w.created_at,
                "expires_at": w.expires_at,
                "state": w.state,
            })
        })
        .collect();
    items.sort_by(|a, b| {
        a.get("created_at")
            .and_then(|v| v.as_str())
            .cmp(&b.get("created_at").and_then(|v| v.as_str()))
    });

    Ok(Json(serde_json::json!({
        "scope": scope,
        "items": items,
    })))
}

pub(crate) async fn handle_window_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((scope, window_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    enforce_companion_admin_scope(&scope, &policy_context)?;

    let Some(handle) = state
        .companion
        .companion_request_windows
        .get_scope(&scope)
        .await
    else {
        return Err(problem_response(
            StatusCode::NOT_FOUND,
            "companion_window_scope_not_found",
            "Not Found",
            format!("No companion request-window scope named '{scope}'."),
        ));
    };

    let now = chrono::Utc::now();
    let window = {
        let mut windows = handle.lock().await;
        let Some(window) = windows.get_mut(&window_id) else {
            return Err(problem_response(
                StatusCode::NOT_FOUND,
                "companion_window_not_found",
                "Not Found",
                format!("No companion request window named '{window_id}'."),
            ));
        };
        window.confirm(now).map_err(|error| {
            problem_response(
                StatusCode::CONFLICT,
                "companion_window_confirm_rejected",
                "Conflict",
                error.to_string(),
            )
        })?;
        window.clone()
    };

    publish_gateway_event(
        &state,
        ServerMessage::companion_request_window(scope.clone(), "confirmed", window.clone()),
    );

    Ok(Json(serde_json::json!({
        "status": "ok",
        "scope": scope,
        "window": window,
    })))
}

pub(crate) async fn handle_window_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((scope, window_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    enforce_companion_admin_scope(&scope, &policy_context)?;

    let Some(handle) = state
        .companion
        .companion_request_windows
        .get_scope(&scope)
        .await
    else {
        return Err(problem_response(
            StatusCode::NOT_FOUND,
            "companion_window_scope_not_found",
            "Not Found",
            format!("No companion request-window scope named '{scope}'."),
        ));
    };

    let window = {
        let mut windows = handle.lock().await;
        let Some(window) = windows.get_mut(&window_id) else {
            return Err(problem_response(
                StatusCode::NOT_FOUND,
                "companion_window_not_found",
                "Not Found",
                format!("No companion request window named '{window_id}'."),
            ));
        };
        window.cancel().map_err(|error| {
            problem_response(
                StatusCode::CONFLICT,
                "companion_window_cancel_rejected",
                "Conflict",
                error.to_string(),
            )
        })?;
        window.clone()
    };

    publish_gateway_event(
        &state,
        ServerMessage::companion_request_window(scope.clone(), "cancelled", window.clone()),
    );

    Ok(Json(serde_json::json!({
        "status": "ok",
        "scope": scope,
        "window": window,
    })))
}

#[derive(serde::Deserialize)]
pub(crate) struct CompanionBehaviorPatch {
    pub explicit_ai_identity: Option<bool>,
    pub allow_public_personalization: Option<bool>,
    pub allow_dense_proactivity: Option<bool>,
    pub public_relationship_cap: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct CompanionUpdateBody {
    pub caption_retention_limit: Option<usize>,
    pub behavior: Option<CompanionBehaviorPatch>,
    pub config: Option<serde_json::Value>,
}

fn apply_behavior_patch(
    current: &mut CompanionBehaviorConfig,
    patch: CompanionBehaviorPatch,
    changes: &mut Vec<&'static str>,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if let Some(explicit_ai_identity) = patch.explicit_ai_identity
        && current.explicit_ai_identity != explicit_ai_identity
    {
        current.explicit_ai_identity = explicit_ai_identity;
        changes.push("behavior.explicit_ai_identity");
    }
    if let Some(allow_public_personalization) = patch.allow_public_personalization
        && current.allow_public_personalization != allow_public_personalization
    {
        current.allow_public_personalization = allow_public_personalization;
        changes.push("behavior.allow_public_personalization");
    }
    if let Some(allow_dense_proactivity) = patch.allow_dense_proactivity
        && current.allow_dense_proactivity != allow_dense_proactivity
    {
        current.allow_dense_proactivity = allow_dense_proactivity;
        changes.push("behavior.allow_dense_proactivity");
    }
    if let Some(public_relationship_cap) = patch.public_relationship_cap {
        let public_relationship_cap = public_relationship_cap.trim().to_string();
        if public_relationship_cap.is_empty() {
            return Err(problem_response(
                StatusCode::BAD_REQUEST,
                "invalid_public_relationship_cap",
                "Bad Request",
                "public_relationship_cap must not be empty.".to_string(),
            ));
        }
        if current.public_relationship_cap != public_relationship_cap {
            current.public_relationship_cap = public_relationship_cap;
            changes.push("behavior.public_relationship_cap");
        }
    }

    Ok(())
}

pub(crate) async fn handle_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CompanionUpdateBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let mut settings = load_companion_admin_settings(&state.runtime.config).map_err(|error| {
        companion_settings_storage_error(
            &error,
            "companion_settings_load_failed",
            "Failed to read companion settings.",
        )
    })?;
    let mut changes = Vec::new();

    if let Some(limit) = body.caption_retention_limit {
        if limit == 0 {
            return Err(problem_response(
                StatusCode::BAD_REQUEST,
                "invalid_caption_retention_limit",
                "Bad Request",
                "caption_retention_limit must be greater than zero.".to_string(),
            ));
        }
        if settings.caption_retention_limit != limit {
            settings.caption_retention_limit = limit;
            changes.push("caption_retention_limit");
        }
    }

    if let Some(behavior) = body.behavior {
        apply_behavior_patch(&mut settings.behavior, behavior, &mut changes)?;
    }

    if let Some(config) = body.config {
        settings.config = Some(config);
        changes.push("config");
    }

    save_companion_admin_settings(&state.runtime.config, &settings).map_err(|error| {
        companion_settings_storage_error(
            &error,
            "companion_settings_save_failed",
            "Failed to save companion settings.",
        )
    })?;
    *state.companion.settings.write().await = settings.clone();
    publish_gateway_event(
        &state,
        ServerMessage::runtime_updated(RuntimeUpdatedPayload {
            component: "companion".to_string(),
            status: "updated".to_string(),
            detail: Some("Companion settings updated.".to_string()),
        }),
    );

    Ok(Json(serde_json::json!({
        "status": "updated",
        "changes": changes,
        "settings": {
            "caption_retention_limit": settings.caption_retention_limit,
            "behavior": settings.behavior,
            "config": settings.config,
        }
    })))
}

#[derive(serde::Deserialize)]
pub(crate) struct CompanionIngressBody {
    pub kind: String,
    pub text: Option<String>,
    pub payload: Option<serde_json::Value>,
}

pub(crate) async fn handle_ingress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
    Json(body): Json<CompanionIngressBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    enforce_companion_admin_scope(&scope, &policy_context)?;

    let tenant_id = scope_tenant_id(&scope)?;
    let mut ingress_headers = headers.clone();
    ingress_headers.insert(
        header::CONTENT_TYPE,
        "application/json"
            .parse()
            .expect("'application/json' is a valid HeaderValue"),
    );
    if let Some(tenant_id) = tenant_id {
        ingress_headers.insert(
            "x-asterel-tenant",
            tenant_id.parse().map_err(|_| {
                problem_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_tenant_id",
                    "Bad Request",
                    "Tenant ID is not a valid header value".to_string(),
                )
            })?,
        );
    } else {
        ingress_headers.remove("x-asterel-tenant");
    }

    let response = match body.kind.trim().to_ascii_lowercase().as_str() {
        "context" => {
            let payload =
                companion_context_ingress_body(body.text.as_deref(), body.payload, &scope)?;
            ingest_context_request(
                &state,
                &ingress_headers,
                Bytes::from(serde_json::to_vec(&payload).map_err(|error| {
                    problem_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "serialization_error",
                        "Internal Server Error",
                        error.to_string(),
                    )
                })?),
            )
            .await
        }
        "multimodal" => {
            let payload =
                companion_multimodal_ingress_body(body.text.as_deref(), body.payload, &scope)?;
            ingest_multimodal_request(
                &state,
                &ingress_headers,
                Bytes::from(serde_json::to_vec(&payload).map_err(|error| {
                    problem_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "serialization_error",
                        "Internal Server Error",
                        error.to_string(),
                    )
                })?),
            )
            .await
        }
        _ => {
            return Err(problem_response(
                StatusCode::BAD_REQUEST,
                "invalid_companion_ingress_kind",
                "Bad Request",
                "kind must be 'context' or 'multimodal'.".to_string(),
            ));
        }
    };

    match response {
        (status, Json(mut value)) if status.is_success() => {
            value["scope"] = serde_json::json!(scope);
            Ok(Json(value))
        }
        error => Err(error),
    }
}

fn scope_tenant_id(scope: &str) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    if scope == "global" {
        return Ok(None);
    }
    let Some(raw_tenant) = scope.strip_prefix("tenant:") else {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_companion_scope",
            "Bad Request",
            "scope must be 'global' or 'tenant:<tenant-id>'.".to_string(),
        ));
    };

    super::super::autosave::sanitize_tenant_id(raw_tenant)
        .map(Some)
        .ok_or_else(|| {
            problem_response(
                StatusCode::BAD_REQUEST,
                "invalid_companion_scope",
                "Bad Request",
                "scope tenant id must contain at least one ASCII letter or digit.".to_string(),
            )
        })
}

fn companion_context_ingress_body(
    text: Option<&str>,
    payload: Option<serde_json::Value>,
    scope: &str,
) -> Result<serde_json::Value, (StatusCode, Json<serde_json::Value>)> {
    if let Some(payload) = payload {
        return Ok(payload);
    }

    let text = text
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            problem_response(
                StatusCode::BAD_REQUEST,
                "invalid_companion_ingress_payload",
                "Bad Request",
                "context ingress requires payload or non-empty text.".to_string(),
            )
        })?;
    let scope_label = scope
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>();

    Ok(serde_json::json!({
        "session_id": "admin-companion",
        "tab_id": "admin",
        "kind": "page",
        "topic": "admin_note",
        "source": format!("admin_{scope_label}"),
        "source_url": "https://admin.local/companion",
        "payload": {
            "text": text,
        },
    }))
}

fn companion_multimodal_ingress_body(
    text: Option<&str>,
    payload: Option<serde_json::Value>,
    scope: &str,
) -> Result<serde_json::Value, (StatusCode, Json<serde_json::Value>)> {
    let Some(mut payload) = payload else {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_companion_ingress_payload",
            "Bad Request",
            "multimodal ingress requires a payload object.".to_string(),
        ));
    };

    let Some(map) = payload.as_object_mut() else {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_companion_ingress_payload",
            "Bad Request",
            "multimodal payload must be a JSON object.".to_string(),
        ));
    };

    if !map.contains_key("source_ref") {
        map.insert(
            "source_ref".to_string(),
            serde_json::json!(format!("admin.{}.upload", scope.replace(':', "_"))),
        );
    }
    if let Some(text) = text.map(str::trim).filter(|value| !value.is_empty()) {
        map.entry("transcript".to_string())
            .or_insert_with(|| serde_json::json!(text));
    }

    Ok(payload)
}

pub(crate) use handle_captions as handle_admin_companion_captions;
pub(crate) use handle_ingress as handle_admin_companion_ingress;
pub(crate) use handle_list as handle_admin_companions_list;
pub(crate) use handle_update as handle_admin_companions_update;
pub(crate) use handle_widgets as handle_admin_companion_widgets;
pub(crate) use handle_window_cancel as handle_admin_companion_window_cancel;
pub(crate) use handle_window_confirm as handle_admin_companion_window_confirm;
pub(crate) use handle_windows as handle_admin_companion_windows;

#[cfg(test)]
mod tests {
    use super::{
        CompanionBehaviorPatch, apply_behavior_patch, companion_context_ingress_body,
        companion_multimodal_ingress_body, scope_tenant_id,
    };
    use crate::config::CompanionBehaviorConfig;

    #[test]
    fn behavior_patch_updates_all_fields_and_tracks_changes() {
        let mut current = CompanionBehaviorConfig {
            explicit_ai_identity: false,
            allow_public_personalization: false,
            allow_dense_proactivity: true,
            public_relationship_cap: "strict".to_string(),
        };
        let mut changes = Vec::new();

        apply_behavior_patch(
            &mut current,
            CompanionBehaviorPatch {
                explicit_ai_identity: Some(true),
                allow_public_personalization: Some(true),
                allow_dense_proactivity: Some(false),
                public_relationship_cap: Some("light".to_string()),
            },
            &mut changes,
        )
        .expect("patch should apply");

        assert!(current.explicit_ai_identity);
        assert!(current.allow_public_personalization);
        assert!(!current.allow_dense_proactivity);
        assert_eq!(current.public_relationship_cap, "light");
        assert_eq!(
            changes,
            vec![
                "behavior.explicit_ai_identity",
                "behavior.allow_public_personalization",
                "behavior.allow_dense_proactivity",
                "behavior.public_relationship_cap",
            ]
        );
    }

    #[test]
    fn behavior_patch_rejects_empty_relationship_cap() {
        let mut current = CompanionBehaviorConfig::default();
        let mut changes = Vec::new();

        let error = apply_behavior_patch(
            &mut current,
            CompanionBehaviorPatch {
                explicit_ai_identity: None,
                allow_public_personalization: None,
                allow_dense_proactivity: None,
                public_relationship_cap: Some("   ".to_string()),
            },
            &mut changes,
        )
        .expect_err("empty cap should be rejected");

        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(error.1.0["code"], "invalid_public_relationship_cap");
    }

    #[test]
    fn scope_tenant_id_accepts_global_and_tenant_scope() {
        assert_eq!(scope_tenant_id("global").unwrap(), None);
        assert_eq!(
            scope_tenant_id("tenant:tenant-a").unwrap(),
            Some("tenant-a".to_string())
        );
    }

    #[test]
    fn scope_tenant_id_rejects_invalid_scope() {
        let error = scope_tenant_id("tenant:***").expect_err("invalid tenant id should fail");
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(error.1.0["code"], "invalid_companion_scope");
    }

    #[test]
    fn companion_context_ingress_body_builds_default_payload() {
        let error = companion_context_ingress_body(None, None, "tenant:tenant-a")
            .expect_err("missing text should fail");
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);

        let payload = companion_context_ingress_body(Some("hello"), None, "tenant:tenant-a")
            .expect("text payload should be synthesized");
        assert_eq!(payload["session_id"], "admin-companion");
        assert_eq!(payload["payload"]["text"], "hello");
        assert_eq!(payload["source"], "admin_tenant_tenant-a");
    }

    #[test]
    fn companion_context_ingress_body_preserves_explicit_payload() {
        let payload = companion_context_ingress_body(
            Some("ignored"),
            Some(serde_json::json!({"custom": true})),
            "global",
        )
        .expect("explicit payload should pass through");

        assert_eq!(payload, serde_json::json!({"custom": true}));
    }

    #[test]
    fn companion_multimodal_ingress_body_adds_defaults() {
        let payload = companion_multimodal_ingress_body(
            Some("caption"),
            Some(serde_json::json!({"blob": "x"})),
            "tenant:tenant-a",
        )
        .expect("multimodal payload should be enriched");

        assert_eq!(payload["blob"], "x");
        assert_eq!(payload["transcript"], "caption");
        assert_eq!(payload["source_ref"], "admin.tenant_tenant-a.upload");
    }

    #[test]
    fn companion_multimodal_ingress_body_rejects_non_object_payload() {
        let error =
            companion_multimodal_ingress_body(None, Some(serde_json::json!(["bad"])), "global")
                .expect_err("non object payload should fail");

        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(error.1.0["code"], "invalid_companion_ingress_payload");
    }

    #[test]
    fn companion_multimodal_ingress_body_keeps_existing_fields() {
        let payload = companion_multimodal_ingress_body(
            Some("ignored caption"),
            Some(serde_json::json!({
                "source_ref": "custom.ref",
                "transcript": "existing",
            })),
            "global",
        )
        .expect("existing fields should be preserved");

        assert_eq!(payload["source_ref"], "custom.ref");
        assert_eq!(payload["transcript"], "existing");
    }
}
