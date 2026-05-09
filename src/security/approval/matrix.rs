//! Matrix-based approval broker.
//!
//! Posts an approval request to a Matrix room and polls for
//! reply-based approve/deny decisions from the operator.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::security::approval::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, format_approval_text, run_inline_approval,
    timed_out_decision,
};

/// Matrix reply-based approval broker.
pub struct MatrixApprovalBroker {
    /// Matrix access token (private to prevent leakage).
    access_token: String,
    /// Target room for approval messages.
    pub room_id: String,
    /// Matrix homeserver base URL.
    pub homeserver: String,
    /// Shared HTTP client for Matrix API calls.
    pub client: reqwest::Client,
    /// Maximum time to wait for a reply response.
    pub timeout: Duration,
}

impl MatrixApprovalBroker {
    /// Create a Matrix approval broker for the given room.
    #[must_use]
    pub fn new(
        access_token: impl Into<String>,
        room_id: impl Into<String>,
        homeserver: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let homeserver = homeserver.into();
        let homeserver = homeserver.trim_end_matches('/').to_string();
        Self {
            access_token: access_token.into(),
            room_id: room_id.into(),
            homeserver,
            client: crate::utils::http::build_http_client(),
            timeout,
        }
    }

    fn parse_matrix_decision(text: &str, intent_id: &str) -> Option<ApprovalDecision> {
        let mut parts = text.split_whitespace();
        let action = parts.next()?;
        let id = parts.next()?;
        if parts.next().is_some() || id != intent_id {
            return None;
        }

        if action.eq_ignore_ascii_case("approve") {
            return Some(ApprovalDecision::Approved);
        }
        if action.eq_ignore_ascii_case("deny") {
            return Some(ApprovalDecision::Denied {
                reason: "denied by user".to_string(),
            });
        }

        None
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    async fn whoami_user_id(&self) -> Result<Option<String>> {
        let response = self
            .client
            .get(format!(
                "{}/_matrix/client/v3/account/whoami",
                self.homeserver
            ))
            .header("Authorization", self.auth())
            .send()
            .await
            .context("send Matrix whoami request for approval broker")?;
        if !response.status().is_success() {
            return Ok(None);
        }

        let body: Value = response
            .json()
            .await
            .context("parse Matrix whoami response for approval broker")?;
        Ok(body
            .get("user_id")
            .and_then(Value::as_str)
            .map(ToString::to_string))
    }

    /// # Errors
    /// Returns an error if sending the approval message fails or response parsing is invalid.
    pub async fn send_approval_message(&self, request: &ApprovalRequest) -> Result<String> {
        let txn_id = format!("approval_{}", request.intent_id);
        let response = self
            .client
            .put(format!(
                "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
                self.homeserver, self.room_id, txn_id
            ))
            .header("Authorization", self.auth())
            .json(&serde_json::json!({
                "msgtype": "m.text",
                "body": format_approval_text(request),
            }))
            .send()
            .await
            .context("send Matrix approval message")?;

        let status = response.status();
        let body: Value = response
            .json()
            .await
            .context("parse Matrix approval send response body")?;
        if !status.is_success() {
            let error = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("Matrix approval send failed: {error}");
        }

        let event_id = body
            .get("event_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Matrix approval response missing event_id")?;
        Ok(event_id.to_string())
    }

    /// # Errors
    /// Returns an error if polling Matrix context fails or response parsing is invalid.
    pub async fn poll_decision_once(
        &self,
        event_id: &str,
        intent_id: &str,
        my_user_id: Option<&str>,
    ) -> Result<Option<ApprovalDecision>> {
        let response = self
            .client
            .get(format!(
                "{}/_matrix/client/v3/rooms/{}/context/{}",
                self.homeserver, self.room_id, event_id
            ))
            .query(&[("limit", "20")])
            .header("Authorization", self.auth())
            .send()
            .await
            .context("poll Matrix approval context")?;

        let status = response.status();
        let body: Value = response
            .json()
            .await
            .context("parse Matrix approval context response body")?;
        if !status.is_success() {
            let error = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("Matrix approval poll failed: {error}");
        }

        let events_after = body
            .get("events_after")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for event in events_after {
            if event.get("type").and_then(Value::as_str) != Some("m.room.message") {
                continue;
            }
            let sender = event.get("sender").and_then(Value::as_str);
            if sender == my_user_id {
                continue;
            }
            let text = event
                .get("content")
                .and_then(|content| content.get("body"))
                .and_then(Value::as_str);
            let Some(text) = text else {
                continue;
            };
            if let Some(decision) = Self::parse_matrix_decision(text, intent_id) {
                return Ok(Some(decision));
            }
        }

        Ok(None)
    }
}

impl ApprovalBroker for MatrixApprovalBroker {
    fn request_approval<'a>(
        &'a self,
        request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ApprovalDecision>> + Send + 'a>> {
        Box::pin(async move {
            if self.timeout.is_zero() {
                return Ok(timed_out_decision());
            }

            let event_id = self.send_approval_message(request).await?;
            let my_user_id = self.whoami_user_id().await?;
            run_inline_approval(self.timeout, || async {
                self.poll_decision_once(&event_id, &request.intent_id, my_user_id.as_deref())
                    .await
            })
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::MatrixApprovalBroker;
    use crate::security::approval::{
        ApprovalBroker, ApprovalDecision, ApprovalRequest, RiskLevel, format_approval_text,
        timed_out_decision,
    };

    fn test_request() -> ApprovalRequest {
        ApprovalRequest {
            intent_id: "intent-1".to_string(),
            tool_name: "shell".to_string(),
            args_summary: "ls -la".to_string(),
            risk_level: RiskLevel::High,
            entity_id: "matrix:!room:example.org".into(),
            channel: "matrix".to_string(),
        }
    }

    #[test]
    fn matrix_broker_constructs() {
        let broker = MatrixApprovalBroker::new(
            "matrix-token",
            "!room:example.org",
            "https://matrix.example.org/",
            Duration::from_secs(9),
        );
        assert_eq!(broker.access_token, "matrix-token");
        assert_eq!(broker.room_id, "!room:example.org");
        assert_eq!(broker.homeserver, "https://matrix.example.org");
        assert_eq!(broker.timeout, Duration::from_secs(9));
    }

    #[test]
    fn matrix_approval_text_contains_core_fields() {
        let text = format_approval_text(&test_request());
        assert!(text.contains("Tool approval required"));
        assert!(text.contains("ID: intent-1"));
        assert!(text.contains("Tool: shell"));
        assert!(text.contains("Args: ls -la"));
        assert!(text.contains("approve intent-1"));
        assert!(text.contains("deny intent-1"));
    }

    #[test]
    fn matrix_decision_parser_accepts_approve_and_deny() {
        assert_eq!(
            MatrixApprovalBroker::parse_matrix_decision("approve intent-1", "intent-1"),
            Some(ApprovalDecision::Approved)
        );
        assert_eq!(
            MatrixApprovalBroker::parse_matrix_decision("deny intent-1", "intent-1"),
            Some(ApprovalDecision::Denied {
                reason: "denied by user".to_string()
            })
        );
        assert!(
            MatrixApprovalBroker::parse_matrix_decision("approve intent-2", "intent-1").is_none()
        );
        assert!(
            MatrixApprovalBroker::parse_matrix_decision("unknown intent-1", "intent-1").is_none()
        );
    }

    #[tokio::test]
    async fn matrix_timeout_path_denies_without_http() {
        let broker = MatrixApprovalBroker::new(
            "matrix-token",
            "!room:example.org",
            "https://matrix.example.org",
            Duration::ZERO,
        );
        let decision = broker.request_approval(&test_request()).await.unwrap();
        assert_eq!(decision, timed_out_decision());
    }
}
