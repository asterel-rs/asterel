//! RFC 9457 Problem Details helpers for structured JSON error responses.
use axum::http::StatusCode;
use axum::response::Json;
use serde_json::{Value, json};

fn problem_type_uri(code: &str) -> String {
    format!("https://asterel-rs.github.io/asterel/reference/problems#{code}")
}

/// Builds an RFC 9457 Problem Details JSON value.
pub(super) fn problem_json(
    status: StatusCode,
    code: &str,
    title: &str,
    detail: impl Into<String>,
) -> Value {
    let detail = detail.into();
    json!({
        "type": problem_type_uri(code),
        "title": title,
        "status": status.as_u16(),
        "detail": detail,
        "code": code,
    })
}

/// Builds an HTTP response pair with an RFC 9457 Problem Details
/// JSON body.
pub(super) fn problem_response(
    status: StatusCode,
    code: &str,
    title: &str,
    detail: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (status, Json(problem_json(status, code, title, detail)))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::problem_json;

    #[test]
    fn problem_json_contains_rfc9457_fields() {
        let value = problem_json(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid request",
            "detail text",
        );
        assert_eq!(value["status"], 400);
        assert_eq!(value["title"], "Invalid request");
        assert_eq!(value["detail"], "detail text");
        assert_eq!(value["code"], "invalid_request");
        assert!(
            value["type"]
                .as_str()
                .is_some_and(|uri| uri.contains("/reference/problems#invalid_request"))
        );
    }
}
