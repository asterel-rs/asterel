use serde::{Deserialize, Serialize};
use tauri_plugin_notification::NotificationExt;

const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:3000";

fn daemon_base_url() -> String {
    std::env::var("ASTEREL_DAEMON_URL").unwrap_or_else(|_| DEFAULT_DAEMON_URL.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

#[tauri::command]
pub async fn health_check() -> Result<DaemonResponse, String> {
    let url = format!("{}/health", daemon_base_url());
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.json().await.unwrap_or(serde_json::json!({}));
    Ok(DaemonResponse { status, body })
}

#[derive(Debug, Deserialize)]
pub struct PairRequest {
    pub code: String,
}

#[tauri::command]
pub async fn pair_with_daemon(req: PairRequest) -> Result<DaemonResponse, String> {
    let url = format!("{}/pair", daemon_base_url());
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("X-Pairing-Code", &req.code)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.json().await.unwrap_or(serde_json::json!({}));
    Ok(DaemonResponse { status, body })
}

#[derive(Debug, Deserialize)]
pub struct DaemonRequestParams {
    pub method: String,
    pub path: String,
    pub body: Option<serde_json::Value>,
    pub token: Option<String>,
}

#[tauri::command]
pub async fn daemon_request(params: DaemonRequestParams) -> Result<DaemonResponse, String> {
    let url = format!("{}{}", daemon_base_url(), params.path);
    let client = reqwest::Client::new();

    let mut builder = match params.method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        "PUT" => client.put(&url),
        other => return Err(format!("unsupported HTTP method: {other}")),
    };

    if let Some(ref token) = params.token {
        builder = builder.bearer_auth(token);
    }

    if let Some(ref body) = params.body {
        builder = builder.json(body);
    }

    let resp = builder.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.json().await.unwrap_or(serde_json::json!({}));
    Ok(DaemonResponse { status, body })
}

#[derive(Debug, Deserialize)]
pub struct NotificationParams {
    pub title: String,
    pub body: String,
}

#[tauri::command]
pub async fn send_notification(
    app: tauri::AppHandle,
    params: NotificationParams,
) -> Result<(), String> {
    app.notification()
        .builder()
        .title(&params.title)
        .body(&params.body)
        .show()
        .map_err(|e| e.to_string())
}
