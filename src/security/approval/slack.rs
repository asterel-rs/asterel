//! Slack-based approval broker.
//!
//! Posts an approval request to a Slack channel and polls for
//! reaction-based approve/deny decisions from the operator.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::contracts::ids::ChannelId;
use crate::security::approval::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, format_approval_text, run_inline_approval,
    timed_out_decision,
};

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack reply-based approval broker.
pub struct SlackApprovalBroker {
    /// Slack bot token (private to prevent leakage).
    bot_token: String,
    /// Target Slack channel for approval messages.
    pub channel_id: ChannelId,
    /// Shared HTTP client for Slack API calls.
    pub client: reqwest::Client,
    /// Maximum time to wait for a reply response.
    pub timeout: Duration,
}

impl SlackApprovalBroker {
    /// Create a Slack approval broker for the given channel.
    #[must_use]
    pub fn new(
        bot_token: impl Into<String>,
        channel_id: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            bot_token: bot_token.into(),
            channel_id: ChannelId::new(channel_id),
            client: crate::utils::http::build_http_client(),
            timeout,
        }
    }

    fn parse_slack_decision(text: &str, intent_id: &str) -> Option<ApprovalDecision> {
        let mut parts = text.split_whitespace();
        let action = parts.next()?;
        let id = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        if id != intent_id {
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

    /// # Errors
    /// Returns an error if posting the approval message fails or response parsing is invalid.
    pub async fn send_approval_message(&self, request: &ApprovalRequest) -> Result<String> {
        let response = self
            .client
            .post(format!("{SLACK_API_BASE}/chat.postMessage"))
            .bearer_auth(&self.bot_token)
            .json(&serde_json::json!({
                "channel": self.channel_id,
                "text": format_approval_text(request),
            }))
            .send()
            .await
            .context("send Slack approval message")?;

        let status = response.status();
        let body: Value = response
            .json()
            .await
            .context("parse Slack approval response body")?;
        if !status.is_success() || body.get("ok") == Some(&Value::Bool(false)) {
            let error = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("Slack approval message failed: {error}");
        }

        let ts = body
            .get("ts")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Slack approval response missing ts")?;
        Ok(ts.to_string())
    }

    /// # Errors
    /// Returns an error if polling replies fails or response parsing is invalid.
    pub async fn poll_decision_once(
        &self,
        message_ts: &str,
        intent_id: &str,
    ) -> Result<Option<ApprovalDecision>> {
        let response = self
            .client
            .get(format!("{SLACK_API_BASE}/conversations.history"))
            .bearer_auth(&self.bot_token)
            .query(&[
                ("channel", self.channel_id.as_str()),
                ("oldest", message_ts),
                ("inclusive", "false"),
                ("limit", "20"),
            ])
            .send()
            .await
            .context("poll Slack approval replies")?;

        let status = response.status();
        let body: Value = response
            .json()
            .await
            .context("parse Slack conversations.history response body")?;
        if !status.is_success() || body.get("ok") == Some(&Value::Bool(false)) {
            let error = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("Slack approval polling failed: {error}");
        }

        let messages = body
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for message in messages {
            if message.get("subtype").is_some() {
                continue;
            }
            let Some(text) = message.get("text").and_then(Value::as_str) else {
                continue;
            };
            if let Some(decision) = Self::parse_slack_decision(text, intent_id) {
                return Ok(Some(decision));
            }
        }

        Ok(None)
    }
}

impl ApprovalBroker for SlackApprovalBroker {
    fn request_approval<'a>(
        &'a self,
        request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ApprovalDecision>> + Send + 'a>> {
        Box::pin(async move {
            if self.timeout.is_zero() {
                return Ok(timed_out_decision());
            }

            let message_ts = self.send_approval_message(request).await?;
            run_inline_approval(self.timeout, || async {
                self.poll_decision_once(&message_ts, &request.intent_id)
                    .await
            })
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SlackApprovalBroker;
    use crate::contracts::ids::ChannelId;
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
            entity_id: "slack:C123".into(),
            channel: "slack".to_string(),
        }
    }

    #[test]
    fn slack_broker_constructs() {
        let broker = SlackApprovalBroker::new("xoxb-token", "C123", Duration::from_secs(9));
        assert_eq!(broker.bot_token, "xoxb-token");
        assert_eq!(broker.channel_id, ChannelId::new("C123"));
        assert_eq!(broker.timeout, Duration::from_secs(9));
    }

    #[test]
    fn slack_approval_text_contains_core_fields() {
        let text = format_approval_text(&test_request());
        assert!(text.contains("Tool approval required"));
        assert!(text.contains("ID: intent-1"));
        assert!(text.contains("Tool: shell"));
        assert!(text.contains("Args: ls -la"));
        assert!(text.contains("approve intent-1"));
        assert!(text.contains("deny intent-1"));
    }

    #[test]
    fn slack_decision_parser_accepts_approve_and_deny() {
        assert_eq!(
            SlackApprovalBroker::parse_slack_decision("approve intent-1", "intent-1"),
            Some(ApprovalDecision::Approved)
        );
        assert_eq!(
            SlackApprovalBroker::parse_slack_decision("deny intent-1", "intent-1"),
            Some(ApprovalDecision::Denied {
                reason: "denied by user".to_string()
            })
        );
        assert!(
            SlackApprovalBroker::parse_slack_decision("approve intent-2", "intent-1").is_none()
        );
        assert!(
            SlackApprovalBroker::parse_slack_decision("unknown intent-1", "intent-1").is_none()
        );
    }

    #[tokio::test]
    async fn slack_timeout_path_denies_without_http() {
        let broker = SlackApprovalBroker::new("xoxb-token", "C123", Duration::ZERO);
        let decision = broker.request_approval(&test_request()).await.unwrap();
        assert_eq!(decision, timed_out_decision());
    }
}
