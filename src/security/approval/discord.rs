//! Discord-based approval broker.
//!
//! Posts an approval request message to a Discord channel and polls
//! for reaction-based responses (checkmark to approve, cross to deny).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::contracts::ids::ChannelId;
use crate::security::approval::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, RiskLevel, run_inline_approval,
    timed_out_decision,
};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const APPROVE_EMOJI: &str = "%E2%9C%85";
const DENY_EMOJI: &str = "%E2%9D%8C";

/// Discord reaction-based approval broker.
pub struct DiscordApprover {
    /// Bot authentication token (private to prevent leakage).
    bot_token: String,
    /// Target channel for approval messages.
    pub channel_id: ChannelId,
    /// Shared HTTP client for Discord API calls.
    pub client: reqwest::Client,
    /// Maximum time to wait for a reaction response.
    pub timeout: Duration,
}

impl DiscordApprover {
    /// Create a Discord approval broker for the given channel.
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

    fn authorization(&self) -> String {
        format!("Bot {}", self.bot_token)
    }

    fn approval_embed_payload(request: &ApprovalRequest) -> Value {
        serde_json::json!({
            "embeds": [
                {
                    "title": "Tool Approval Required",
                    "description": format!(
                        "Tool: `{}`\nArgs: `{}`\nRisk: `{}`\nEntity: `{}`",
                        request.tool_name,
                        request.args_summary,
                        risk_label(request.risk_level),
                        request.entity_id
                    ),
                    "color": embed_color(request.risk_level)
                }
            ]
        })
    }

    /// # Errors
    /// Returns an error if posting the approval embed fails or response parsing is invalid.
    pub async fn send_approval_embed(&self, request: &ApprovalRequest) -> Result<String> {
        let url = format!("{DISCORD_API_BASE}/channels/{}/messages", self.channel_id);
        let response = self
            .client
            .post(&url)
            .header("Authorization", self.authorization())
            .json(&Self::approval_embed_payload(request))
            .send()
            .await
            .context("send Discord approval embed")?
            .error_for_status()
            .context("Discord approval embed rejected")?;

        let body: Value = response
            .json()
            .await
            .context("parse Discord approval embed response")?;
        let message_id = body
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .context("Discord approval response missing message id")?;
        Ok(message_id.to_string())
    }

    /// # Errors
    /// Returns an error if adding either approval reaction fails.
    pub async fn add_reactions(&self, message_id: &str) -> Result<()> {
        for emoji in [APPROVE_EMOJI, DENY_EMOJI] {
            let url = format!(
                "{DISCORD_API_BASE}/channels/{}/messages/{message_id}/reactions/{emoji}/@me",
                self.channel_id
            );
            self.client
                .put(&url)
                .header("Authorization", self.authorization())
                .send()
                .await
                .with_context(|| format!("add Discord approval reaction '{emoji}'"))?
                .error_for_status()
                .with_context(|| format!("Discord rejected approval reaction '{emoji}'"))?;
        }
        Ok(())
    }

    fn has_non_bot_reaction(reactions: &Value) -> bool {
        reactions.as_array().is_some_and(|users| {
            users
                .iter()
                .any(|user| !user.get("bot").and_then(Value::as_bool).unwrap_or(false))
        })
    }

    /// # Errors
    /// Returns an error if polling Discord reactions fails or response parsing is invalid.
    pub async fn poll_reaction(&self, message_id: &str, emoji: &str) -> Result<bool> {
        let url = format!(
            "{DISCORD_API_BASE}/channels/{}/messages/{message_id}/reactions/{emoji}",
            self.channel_id
        );
        let response = self
            .client
            .get(&url)
            .query(&[("limit", "10")])
            .header("Authorization", self.authorization())
            .send()
            .await
            .context("poll Discord approval reactions")?
            .error_for_status()
            .context("Discord reaction poll failed")?;
        let reactions: Value = response
            .json()
            .await
            .context("parse Discord reaction list")?;
        Ok(Self::has_non_bot_reaction(&reactions))
    }
}

impl ApprovalBroker for DiscordApprover {
    fn request_approval<'a>(
        &'a self,
        request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ApprovalDecision>> + Send + 'a>> {
        Box::pin(async move {
            if self.timeout.is_zero() {
                return Ok(timed_out_decision());
            }

            let message_id = self.send_approval_embed(request).await?;
            self.add_reactions(&message_id).await?;

            run_inline_approval(self.timeout, || async {
                if self.poll_reaction(&message_id, APPROVE_EMOJI).await? {
                    return Ok(Some(ApprovalDecision::Approved));
                }
                if self.poll_reaction(&message_id, DENY_EMOJI).await? {
                    return Ok(Some(ApprovalDecision::Denied {
                        reason: "denied by user".to_string(),
                    }));
                }
                Ok(None)
            })
            .await
        })
    }
}

fn risk_label(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Low => "Low",
        RiskLevel::Medium => "Medium",
        RiskLevel::High => "High",
    }
}

const fn embed_color(risk: RiskLevel) -> u32 {
    match risk {
        RiskLevel::Low => 0x002E_CC71,
        RiskLevel::Medium => 0x00F1_C40F,
        RiskLevel::High => 0x00E7_4C3C,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{APPROVE_EMOJI, DENY_EMOJI, DiscordApprover};
    use crate::contracts::ids::ChannelId;
    use crate::security::approval::{
        ApprovalBroker, ApprovalRequest, RiskLevel, timed_out_decision,
    };

    fn test_request() -> ApprovalRequest {
        ApprovalRequest {
            intent_id: "intent-1".to_string(),
            tool_name: "shell".to_string(),
            args_summary: "ls -la".to_string(),
            risk_level: RiskLevel::High,
            entity_id: "discord:123".into(),
            channel: "discord".to_string(),
        }
    }

    #[test]
    fn discord_broker_constructs() {
        let broker = DiscordApprover::new("token", "chan", Duration::from_secs(7));
        assert_eq!(broker.bot_token, "token");
        assert_eq!(broker.channel_id, ChannelId::new("chan"));
        assert_eq!(broker.timeout, Duration::from_secs(7));
    }

    #[test]
    fn discord_embed_payload_contains_expected_fields() {
        let payload = DiscordApprover::approval_embed_payload(&test_request());
        let embed = &payload["embeds"][0];
        let description = embed["description"].as_str().unwrap_or_default();
        assert_eq!(embed["title"], "Tool Approval Required");
        assert!(description.contains("Tool: `shell`"));
        assert!(description.contains("Args: `ls -la`"));
        assert!(description.contains("Risk: `High`"));
        assert!(description.contains("Entity: `discord:123`"));
    }

    #[test]
    fn discord_embed_payload_uses_risk_color() {
        let payload = DiscordApprover::approval_embed_payload(&test_request());
        assert_eq!(payload["embeds"][0]["color"], 0xE7_4C_3C);
    }

    #[test]
    fn discord_reaction_parser_accepts_non_bot_user() {
        let reactions = serde_json::json!([
            {"id": "1", "bot": true},
            {"id": "2", "bot": false}
        ]);
        assert!(DiscordApprover::has_non_bot_reaction(&reactions));
    }

    #[test]
    fn discord_reaction_parser_rejects_bot_only_reactions() {
        let reactions = serde_json::json!([
            {"id": "1", "bot": true},
            {"id": "2", "bot": true}
        ]);
        assert!(!DiscordApprover::has_non_bot_reaction(&reactions));
    }

    #[test]
    fn discord_emoji_constants_are_url_encoded() {
        assert_eq!(APPROVE_EMOJI, "%E2%9C%85");
        assert_eq!(DENY_EMOJI, "%E2%9D%8C");
    }

    #[tokio::test]
    async fn discord_timeout_path_denies_without_http() {
        let broker = DiscordApprover::new("token", "chan", Duration::ZERO);
        let decision = broker.request_approval(&test_request()).await.unwrap();
        assert_eq!(decision, timed_out_decision());
    }
}
