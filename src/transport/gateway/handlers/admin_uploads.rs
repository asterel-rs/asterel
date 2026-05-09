use std::path::Path;

use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use uuid::Uuid;

use super::super::AppState;
use super::super::problem_details::problem_response;
use super::admin_paths::admin_uploads_dir;
use super::require_management_principal;
use crate::security::{RootBoundPathKind, resolve_relative_path_within_root};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredAdminUpload {
    upload_id: String,
    field_name: Option<String>,
    original_name: Option<String>,
    stored_name: String,
    content_type: Option<String>,
    size_bytes: usize,
    stored_path: String,
    source_ref: String,
}

fn gateway_uploads_dir(workspace_dir: &Path) -> std::path::PathBuf {
    workspace_dir
        .join(".asterel")
        .join("gateway")
        .join("uploads")
}

pub(in crate::transport::gateway) fn resolve_stored_upload_path(
    workspace_dir: &Path,
    upload_id: &str,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let uploads_dir = gateway_uploads_dir(workspace_dir);
    let metadata_path = uploads_dir.join(format!("{upload_id}.json"));
    let metadata = std::fs::read(&metadata_path).ok()?;
    let stored: StoredAdminUpload = serde_json::from_slice(&metadata).ok()?;
    if stored.upload_id != upload_id {
        tracing::warn!(
            expected_upload_id = upload_id,
            actual_upload_id = %stored.upload_id,
            "stored upload metadata id mismatch"
        );
        return None;
    }
    let stored_path = match resolve_relative_path_within_root(
        &uploads_dir,
        Path::new(&stored.stored_name),
        RootBoundPathKind::File,
    ) {
        Ok(stored_path) => stored_path,
        Err(error) => {
            tracing::warn!(
                upload_id,
                stored_name = %stored.stored_name,
                error = %error,
                "stored upload metadata path is invalid"
            );
            return None;
        }
    };
    Some((uploads_dir.clone(), stored_path))
}

fn persist_upload_bytes(
    workspace_dir: &Path,
    uploads_dir: &Path,
    field_name: Option<&str>,
    original_name: Option<&str>,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<StoredAdminUpload, (StatusCode, Json<serde_json::Value>)> {
    let upload_id = format!("upl_{}", Uuid::new_v4().simple());
    let stored_name = original_name.map_or_else(
        || format!("{upload_id}.bin"),
        |name| format!("{upload_id}-{}", sanitize_upload_filename(name)),
    );
    let file_path = uploads_dir.join(&stored_name);
    std::fs::write(&file_path, bytes).map_err(|error| {
        upload_internal_error(
            "admin_upload_store_failed",
            "Failed to persist uploaded content.",
            &error,
        )
    })?;

    let stored = StoredAdminUpload {
        upload_id: upload_id.clone(),
        field_name: field_name.map(ToString::to_string),
        original_name: original_name.map(sanitize_upload_filename),
        stored_name: stored_name.clone(),
        content_type: content_type.map(ToString::to_string),
        size_bytes: bytes.len(),
        stored_path: persisted_upload_path(workspace_dir, uploads_dir, &stored_name),
        source_ref: format!("admin-upload:{upload_id}"),
    };
    let metadata_path = uploads_dir.join(format!("{upload_id}.json"));
    let metadata = serde_json::to_vec_pretty(&stored).map_err(|error| {
        upload_internal_error(
            "admin_upload_store_failed",
            "Failed to serialize upload metadata.",
            &error,
        )
    })?;
    std::fs::write(metadata_path, metadata).map_err(|error| {
        upload_internal_error(
            "admin_upload_store_failed",
            "Failed to persist upload metadata.",
            &error,
        )
    })?;

    Ok(stored)
}

fn sanitize_upload_filename(raw: &str) -> String {
    let mut sanitized = raw
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>();
    sanitized.truncate(96);
    let trimmed = sanitized.trim_matches('.');
    if trimmed.is_empty() {
        "upload.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn upload_internal_error(
    code: &'static str,
    detail: &'static str,
    error: &dyn std::fmt::Display,
) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!(%error, "admin upload failed");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        "Internal Server Error",
        detail.to_string(),
    )
}

fn persisted_upload_path(workspace_dir: &Path, uploads_dir: &Path, stored_name: &str) -> String {
    let stored_path = uploads_dir.join(stored_name);
    stored_path
        .strip_prefix(workspace_dir)
        .unwrap_or(&stored_path)
        .display()
        .to_string()
}

pub(crate) async fn handle_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let uploads_dir = admin_uploads_dir(&state.runtime.config);
    tokio::fs::create_dir_all(&uploads_dir)
        .await
        .map_err(|error| {
            upload_internal_error(
                "admin_upload_prepare_failed",
                "Failed to prepare the admin upload store.",
                &error,
            )
        })?;

    let mut items = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|error| {
        upload_internal_error(
            "admin_upload_read_failed",
            "Failed to read multipart upload data.",
            &error,
        )
    })? {
        let field_name = field.name().map(ToString::to_string);
        let original_name = field.file_name().map(ToString::to_string);
        let content_type = field.content_type().map(ToString::to_string);
        let bytes = field.bytes().await.map_err(|error| {
            upload_internal_error(
                "admin_upload_read_failed",
                "Failed to read uploaded file content.",
                &error,
            )
        })?;

        let stored = persist_upload_bytes(
            &state.runtime.config.workspace_dir,
            &uploads_dir,
            field_name.as_deref(),
            original_name.as_deref(),
            content_type.as_deref(),
            &bytes,
        )?;

        items.push(serde_json::to_value(stored).map_err(|error| {
            upload_internal_error(
                "admin_upload_serialize_failed",
                "Failed to serialize stored upload metadata.",
                &error,
            )
        })?);
    }

    if items.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "missing_upload_fields",
            "Bad Request",
            "multipart form did not contain any upload fields.".to_string(),
        ));
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "count": items.len(),
        "items": items,
    })))
}

pub(crate) use handle_upload as handle_admin_upload;

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{persist_upload_bytes, resolve_stored_upload_path, sanitize_upload_filename};

    #[test]
    fn sanitize_upload_filename_removes_path_segments_and_symbols() {
        let sanitized = sanitize_upload_filename("../odd name?.png");
        assert_eq!(sanitized, "_odd_name_.png");
    }

    #[test]
    fn sanitize_upload_filename_falls_back_when_empty() {
        assert_eq!(sanitize_upload_filename("..."), "upload.bin");
    }

    #[test]
    fn persist_upload_bytes_writes_file_and_metadata() {
        let temp = TempDir::new().expect("temp dir");
        let uploads_dir = temp.path().join(".asterel/gateway/uploads");
        std::fs::create_dir_all(&uploads_dir).expect("create uploads dir");

        let stored = persist_upload_bytes(
            temp.path(),
            &uploads_dir,
            Some("file"),
            Some("hello.txt"),
            Some("text/plain"),
            b"hello upload",
        )
        .expect("persist upload");

        assert!(temp.path().join(&stored.stored_path).exists());
        assert!(
            uploads_dir
                .join(format!("{}.json", stored.upload_id))
                .exists()
        );
        assert_eq!(stored.original_name.as_deref(), Some("hello.txt"));
        assert_eq!(stored.size_bytes, 12);
    }

    #[test]
    fn resolve_stored_upload_path_rejects_tampered_metadata_escape() {
        let temp = TempDir::new().expect("temp dir");
        let uploads_dir = temp.path().join(".asterel/gateway/uploads");
        std::fs::create_dir_all(&uploads_dir).expect("create uploads dir");

        let upload_id = "upl_tampered";
        let outside = temp.path().join("outside.png");
        std::fs::write(&outside, b"outside").expect("write outside file");
        std::fs::write(
            uploads_dir.join(format!("{upload_id}.json")),
            serde_json::to_vec(&serde_json::json!({
                "upload_id": upload_id,
                "field_name": "file",
                "original_name": "outside.png",
                "stored_name": "../../../outside.png",
                "content_type": "image/png",
                "size_bytes": 7,
                "stored_path": "outside.png",
                "source_ref": "admin-upload:upl_tampered"
            }))
            .expect("serialize metadata"),
        )
        .expect("write metadata");

        assert!(resolve_stored_upload_path(temp.path(), upload_id).is_none());
    }
}
