//! Channel-aware approval broker factory.
//!
//! Selects the appropriate approval broker (CLI, Discord, Slack,
//! Telegram, or Matrix) based on the active transport channel.
//!
//! # Watchlist Status: Incomplete — Governance Grammar Pending
//!
//! **Current state**: Text-reply approval is not yet implemented.
//! Any channel that does not have a dedicated broker (Discord, Telegram, Slack, Matrix)
//! falls through to either:
//! - `CliApprovalBroker` when `operator_fallback = true` (default), or
//! - `TextReplyApprovalBroker` when `operator_fallback = false`, which auto-denies.
//!
//! **What is still needed for full channel approval**:
//! 1. **Interactive reply protocol**: each channel broker must send an approval prompt
//!    to the channel and wait for a typed reply (`approve` / `deny`) from an operator.
//!    Currently only CLI, Discord, Telegram, Slack, and Matrix have real polling
//!    implementations; all other channels auto-deny.
//! 2. **`WhatsApp` broker**: `WhatsApp` has no broker in `broker_for_channel`; it falls
//!    through to the `_` arm. A `WhatsAppApprovalBroker` can follow the Telegram
//!    pattern once the Cloud API reply loop is in place.
//!
//! **Completion plan**:
//! - Future: implement `WhatsAppApprovalBroker` + generic `TextReplyBroker` that
//!   sends a prompt to the channel and polls for a reply token.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::cli::CliApprovalBroker;
#[cfg(feature = "discord")]
use super::discord::DiscordApprover;
#[cfg(feature = "matrix")]
use super::matrix::MatrixApprovalBroker;
use super::slack::SlackApprovalBroker;
use super::telegram::TelegramApprover;
use super::{ApprovalBroker, ApprovalDecision, ApprovalRequest};

/// Configuration context for channel-specific approval brokers.
#[derive(Debug, Clone)]
pub struct ChannelApprovalCtx {
    /// Bot/access token for the channel API.
    pub bot_token: Option<String>,
    /// Target channel or room identifier.
    pub channel_id: Option<String>,
    /// Matrix homeserver URL (Matrix only).
    pub homeserver: Option<String>,
    /// Maximum time to wait for an approval response.
    pub timeout: Duration,
    /// Fall back to CLI operator approval when channel credentials missing.
    pub operator_fallback: bool,
}

impl Default for ChannelApprovalCtx {
    fn default() -> Self {
        Self {
            bot_token: None,
            channel_id: None,
            homeserver: None,
            timeout: Duration::from_secs(60),
            operator_fallback: true,
        }
    }
}

/// Stub approval broker that auto-denies until interactive reply is implemented.
pub struct TextReplyApprovalBroker {
    /// Name of the transport channel this broker serves.
    pub channel_name: String,
    /// Maximum approval wait time (currently unused; auto-denies).
    pub timeout: Duration,
}

impl TextReplyApprovalBroker {
    /// Create a new text-reply broker for the given channel.
    pub fn new(channel_name: impl Into<String>, timeout: Duration) -> Self {
        Self {
            channel_name: channel_name.into(),
            timeout,
        }
    }
}

impl ApprovalBroker for TextReplyApprovalBroker {
    fn request_approval<'a>(
        &'a self,
        request: &'a ApprovalRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ApprovalDecision>> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!(
                channel = %self.channel_name,
                tool = %request.tool_name,
                risk = ?request.risk_level,
                timeout_secs = self.timeout.as_secs(),
                "tool approval requested via channel (auto-deny until interactive approval implemented)"
            );

            Ok(ApprovalDecision::Denied {
                reason: format!(
                    "Channel '{}' approval not yet implemented. Set autonomy_level to 'full' or 'read_only' in config.",
                    self.channel_name
                ),
            })
        })
    }
}

fn fallback_broker(
    channel_name: &str,
    channel_config: &ChannelApprovalCtx,
) -> Arc<dyn ApprovalBroker> {
    if channel_config.operator_fallback {
        tracing::info!(
            channel = channel_name,
            timeout_secs = channel_config.timeout.as_secs(),
            "using operator terminal approval fallback broker"
        );
        Arc::new(CliApprovalBroker::new(channel_config.timeout))
    } else {
        Arc::new(TextReplyApprovalBroker::new(
            channel_name,
            channel_config.timeout,
        ))
    }
}

/// Select the appropriate approval broker for a transport channel.
#[must_use]
pub fn broker_for_channel(
    channel_name: &str,
    channel_config: &ChannelApprovalCtx,
) -> Arc<dyn ApprovalBroker> {
    match channel_name {
        "cli" => Arc::new(CliApprovalBroker::new(channel_config.timeout)),
        #[cfg(feature = "discord")]
        "discord" => channel_config
            .bot_token
            .as_deref()
            .zip(channel_config.channel_id.as_deref())
            .map_or_else(
                || fallback_broker(channel_name, channel_config),
                |(bot_token, channel_id)| {
                    Arc::new(DiscordApprover::new(
                        bot_token,
                        channel_id,
                        channel_config.timeout,
                    )) as Arc<dyn ApprovalBroker>
                },
            ),
        "telegram" => channel_config
            .bot_token
            .as_deref()
            .zip(channel_config.channel_id.as_deref())
            .map_or_else(
                || fallback_broker(channel_name, channel_config),
                |(bot_token, chat_id)| {
                    Arc::new(TelegramApprover::new(
                        bot_token,
                        chat_id,
                        channel_config.timeout,
                    )) as Arc<dyn ApprovalBroker>
                },
            ),
        "slack" => channel_config
            .bot_token
            .as_deref()
            .zip(channel_config.channel_id.as_deref())
            .map_or_else(
                || fallback_broker(channel_name, channel_config),
                |(bot_token, channel_id)| {
                    Arc::new(SlackApprovalBroker::new(
                        bot_token,
                        channel_id,
                        channel_config.timeout,
                    )) as Arc<dyn ApprovalBroker>
                },
            ),
        #[cfg(feature = "matrix")]
        "matrix" => channel_config
            .bot_token
            .as_deref()
            .zip(channel_config.channel_id.as_deref())
            .zip(channel_config.homeserver.as_deref())
            .map_or_else(
                || fallback_broker(channel_name, channel_config),
                |((access_token, room_id), homeserver)| {
                    Arc::new(MatrixApprovalBroker::new(
                        access_token,
                        room_id,
                        homeserver,
                        channel_config.timeout,
                    )) as Arc<dyn ApprovalBroker>
                },
            ),
        _ => fallback_broker(channel_name, channel_config),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ChannelApprovalCtx, TextReplyApprovalBroker, broker_for_channel};
    use crate::security::{ApprovalBroker, ApprovalDecision, ApprovalRequest, RiskLevel};

    fn request_for_channel(channel: &str) -> ApprovalRequest {
        ApprovalRequest {
            intent_id: "intent-1".to_string(),
            tool_name: "shell".to_string(),
            args_summary: "ls".to_string(),
            risk_level: RiskLevel::High,
            entity_id: "entity-1".into(),
            channel: channel.to_string(),
        }
    }

    #[test]
    fn channel_approval_context_default_values() {
        let context = ChannelApprovalCtx::default();
        assert!(context.bot_token.is_none());
        assert!(context.channel_id.is_none());
        assert!(context.homeserver.is_none());
        assert_eq!(context.timeout, Duration::from_secs(60));
        assert!(context.operator_fallback);
    }

    #[tokio::test]
    async fn broker_for_email_uses_operator_fallback_by_default() {
        let request = request_for_channel("email");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("email", &context)
            .request_approval(&request)
            .await
            .expect("email broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("email broker should deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_irc_uses_operator_fallback_by_default() {
        let request = request_for_channel("irc");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("irc", &context)
            .request_approval(&request)
            .await
            .expect("irc broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("irc broker should deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_webhook_uses_operator_fallback_by_default() {
        let request = request_for_channel("webhook");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("webhook", &context)
            .request_approval(&request)
            .await
            .expect("webhook broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("webhook broker should deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_telegram_without_context_uses_operator_fallback_by_default() {
        let request = request_for_channel("telegram");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("telegram", &context)
            .request_approval(&request)
            .await
            .expect("telegram broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("telegram broker should currently deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_slack_without_context_uses_operator_fallback_by_default() {
        let request = request_for_channel("slack");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("slack", &context)
            .request_approval(&request)
            .await
            .expect("slack broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("slack broker should currently deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_matrix_without_context_uses_operator_fallback_by_default() {
        let request = request_for_channel("matrix");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("matrix", &context)
            .request_approval(&request)
            .await
            .expect("matrix broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("matrix broker should currently deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_discord_without_context_uses_operator_fallback_by_default() {
        let request = request_for_channel("discord");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("discord", &context)
            .request_approval(&request)
            .await
            .expect("discord broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("discord broker should currently deny");
        };

        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[cfg(feature = "discord")]
    #[test]
    fn broker_for_discord_with_context_uses_interactive_timeout_path() {
        tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(async {
                let request = request_for_channel("discord");
                let context = ChannelApprovalCtx {
                    bot_token: Some("discord-token".to_string()),
                    channel_id: Some("123".to_string()),
                    homeserver: None,
                    timeout: Duration::ZERO,
                    operator_fallback: false,
                };
                let decision = broker_for_channel("discord", &context)
                    .request_approval(&request)
                    .await
                    .expect("discord interactive broker should not fail on immediate timeout");

                assert_eq!(
                    decision,
                    ApprovalDecision::Denied {
                        reason: "approval timed out".to_string()
                    }
                );
            });
    }

    #[tokio::test]
    async fn broker_for_telegram_with_context_uses_interactive_timeout_path() {
        let request = request_for_channel("telegram");
        let context = ChannelApprovalCtx {
            bot_token: Some("telegram-token".to_string()),
            channel_id: Some("456".to_string()),
            homeserver: None,
            timeout: Duration::ZERO,
            operator_fallback: false,
        };
        let decision = broker_for_channel("telegram", &context)
            .request_approval(&request)
            .await
            .expect("telegram interactive broker should not fail on immediate timeout");

        assert_eq!(
            decision,
            ApprovalDecision::Denied {
                reason: "approval timed out".to_string()
            }
        );
    }

    #[tokio::test]
    async fn broker_for_slack_with_context_uses_interactive_timeout_path() {
        let request = request_for_channel("slack");
        let context = ChannelApprovalCtx {
            bot_token: Some("xoxb-token".to_string()),
            channel_id: Some("C123".to_string()),
            homeserver: None,
            timeout: Duration::ZERO,
            operator_fallback: false,
        };
        let decision = broker_for_channel("slack", &context)
            .request_approval(&request)
            .await
            .expect("slack interactive broker should not fail on immediate timeout");

        assert_eq!(
            decision,
            ApprovalDecision::Denied {
                reason: "approval timed out".to_string()
            }
        );
    }

    #[cfg(feature = "matrix")]
    #[test]
    fn broker_for_matrix_with_context_uses_interactive_timeout_path() {
        tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(async {
                let request = request_for_channel("matrix");
                let context = ChannelApprovalCtx {
                    bot_token: Some("matrix-token".to_string()),
                    channel_id: Some("!room:example.org".to_string()),
                    homeserver: Some("https://matrix.example.org".to_string()),
                    timeout: Duration::ZERO,
                    operator_fallback: false,
                };
                let decision = broker_for_channel("matrix", &context)
                    .request_approval(&request)
                    .await
                    .expect("matrix interactive broker should not fail on immediate timeout");

                assert_eq!(
                    decision,
                    ApprovalDecision::Denied {
                        reason: "approval timed out".to_string()
                    }
                );
            });
    }

    #[tokio::test]
    async fn broker_for_cli_uses_cli_approval_broker_timeout_path() {
        let request = request_for_channel("cli");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("cli", &context)
            .request_approval(&request)
            .await
            .expect("cli broker should not fail on zero timeout");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("cli broker should deny when no interactive input is available");
        };
        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_email_with_operator_fallback_uses_cli_timeout_path() {
        let request = request_for_channel("email");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            operator_fallback: true,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("email", &context)
            .request_approval(&request)
            .await
            .expect("operator fallback broker should not fail on zero timeout");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("operator fallback should deny on zero-timeout");
        };
        assert!(reason == "approval timed out" || reason.contains("input error"));
    }

    #[tokio::test]
    async fn broker_for_email_without_operator_fallback_uses_text_reply_auto_deny() {
        let request = request_for_channel("email");
        let context = ChannelApprovalCtx {
            timeout: Duration::ZERO,
            operator_fallback: false,
            ..ChannelApprovalCtx::default()
        };
        let decision = broker_for_channel("email", &context)
            .request_approval(&request)
            .await
            .expect("text reply broker should not fail on zero-timeout");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("text reply broker should deny");
        };
        assert!(reason.contains("approval not yet implemented"));
    }

    #[tokio::test]
    async fn text_reply_broker_denies_with_informative_message() {
        let broker = TextReplyApprovalBroker::new("slack", Duration::from_secs(60));
        let request = request_for_channel("slack");
        let decision = broker
            .request_approval(&request)
            .await
            .expect("text reply broker should not fail");

        let ApprovalDecision::Denied { reason } = decision else {
            panic!("text reply broker should currently deny");
        };

        assert!(reason.contains("approval not yet implemented"));
        assert!(reason.contains("autonomy_level"));
        assert_eq!(broker.timeout, Duration::from_secs(60));
    }
}
