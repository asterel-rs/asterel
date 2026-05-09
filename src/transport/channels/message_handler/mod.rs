//! Channel message handler: dispatches inbound events through ingress
//! policy, tool-loop execution, autosave, and reply delivery.
mod approval;
mod autosave;
mod dispatch;
mod execution_context;
mod media;
mod policy;
mod prompt;
mod reply;
mod routing;
mod stream;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use tokio::task::JoinHandle;

use super::ingress_policy::ExternalIngressPolicyOutcome;
use super::startup::ChannelRuntime;
use super::traits::{ChannelEvent, ChannelMessage, MediaAttachment};
use crate::config::GroupIsolationLevel;
use crate::contracts::ids::EntityId;
use crate::core::agent::{LoopStopReason, TurnExecutionOutcome};
use crate::core::persona::person_identity::channel_entity_id;
#[cfg(feature = "discord")]
use crate::core::providers::StreamEvent;
#[cfg(feature = "discord")]
use crate::core::providers::streaming::StreamSink;
use crate::media::MediaProcessor;
use crate::security::policy::AutonomyLevel;
use crate::utils::text::strip_reasoning;

const EVENT_CONTEXT_HINT: &str =
    "[Channel Context: Ambient event — react only if useful, brief, and easy to ignore]";

#[derive(Clone, Copy)]
struct GroupIsolationProfile {
    filesystem: GroupIsolationLevel,
    process: GroupIsolationLevel,
    network: GroupIsolationLevel,
}

struct ChannelMessageProcessingState {
    effective_autonomy: AutonomyLevel,
    tool_allowlist: Option<HashSet<String>>,
    routing_group: String,
    group_isolation: GroupIsolationProfile,
    ingress: ExternalIngressPolicyOutcome,
    source: String,
    autosave_entity_id: EntityId,
    reply_target: String,
    thinking_key: String,
}

struct ToolLoopStreamState {
    stream_forward_handle: Option<JoinHandle<()>>,
    discord_thinking_embed_handle: Option<JoinHandle<()>>,
    streamed_output: Arc<AtomicBool>,
}

struct ToolLoopExecutionArtifacts {
    result: Result<TurnExecutionOutcome>,
    stream_state: ToolLoopStreamState,
    show_reasoning: bool,
    media_processor: MediaProcessor,
}

struct ChannelToolLoopInput<'a> {
    channel_name: &'a str,
    sender: &'a str,
    conversation_id: Option<&'a str>,
    attachments: &'a [MediaAttachment],
    user_message: &'a str,
    thinking_key: &'a str,
    reply_target: &'a str,
    enable_streaming: bool,
    channel_context_hint: Option<&'a str>,
}

#[cfg(feature = "discord")]
const DISCORD_THINKING_COLOR_COMPLETED: u32 = 0x0010_B981;
#[cfg(feature = "discord")]
const DISCORD_THINKING_COLOR_ACTIVE: u32 = 0x00F5_9E0B;

#[cfg(feature = "discord")]
struct DiscordThinkingEmbedSink {
    sender: tokio::sync::mpsc::Sender<StreamEvent>,
}

#[cfg(feature = "discord")]
impl DiscordThinkingEmbedSink {
    fn new(sender: tokio::sync::mpsc::Sender<StreamEvent>) -> Self {
        Self { sender }
    }
}

#[cfg(feature = "discord")]
impl StreamSink for DiscordThinkingEmbedSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if self.sender.send(event.clone()).await.is_err() {
                tracing::warn!("discord thinking sink: channel closed, event dropped");
            }
        })
    }
}

pub(super) async fn handle_channel_message(rt: &ChannelRuntime, msg: &ChannelMessage) {
    autosave::log_incoming_channel_message(msg);
    let ChannelMessageProcessingState {
        effective_autonomy,
        tool_allowlist,
        routing_group,
        group_isolation,
        ingress,
        source,
        autosave_entity_id,
        reply_target,
        thinking_key,
    } = dispatch::build_message_processing_state(rt, msg);

    if prompt::try_handle_runtime_command(rt, msg, &reply_target, &thinking_key).await {
        return;
    }

    if stream::handle_blocked_ingress_reply(rt, msg, &reply_target, &source, &ingress).await {
        return;
    }

    if should_persist_channel_ingress(&ingress) {
        autosave::autosave_and_ingest(
            rt,
            msg,
            autosave_entity_id.as_str(),
            &ingress.persisted_summary,
        )
        .await;
    }

    let ctx = dispatch::build_execution_context(
        rt,
        msg,
        EntityId::new(
            rt.tenant_policy_context
                .scope_entity_id(&channel_entity_id(&msg.channel, &thinking_key)),
        ),
        routing_group,
        group_isolation,
        effective_autonomy,
        tool_allowlist,
    )
    .await;
    let context_hint = resolve_discord_channel_context_hint(msg);
    let execution = stream::execute_channel_tool_loop(
        rt,
        ChannelToolLoopInput {
            channel_name: &msg.channel,
            sender: &msg.sender,
            conversation_id: msg.conversation_id.as_deref(),
            attachments: &msg.attachments,
            user_message: &ingress.model_input,
            thinking_key: &thinking_key,
            reply_target: &reply_target,
            enable_streaming: true,
            channel_context_hint: context_hint,
        },
        &ctx,
    )
    .await;
    stream::process_tool_loop_result(rt, msg, &reply_target, execution).await;
}

fn resolve_discord_channel_context_hint(msg: &ChannelMessage) -> Option<&str> {
    if msg.channel != "discord" {
        return None;
    }
    if let Some(hint) = msg.context_hint.as_deref() {
        return Some(hint);
    }
    let is_thread = msg.thread_id.is_some();
    if is_thread {
        return Some(
            "[Channel Context: Thread continuation — stay on topic, build on prior context]",
        );
    }
    None
}

fn should_persist_channel_ingress(ingress: &ExternalIngressPolicyOutcome) -> bool {
    !ingress.blocked
}

pub(super) async fn handle_channel_event(rt: &ChannelRuntime, event: &ChannelEvent) {
    let Some(sender) = event.sender() else {
        tracing::debug!(
            channel = event.channel_name(),
            "event sender missing, skipping"
        );
        return;
    };
    let Some(conversation_id) = event.conversation_id() else {
        tracing::debug!(
            channel = event.channel_name(),
            "event conversation missing, skipping"
        );
        return;
    };
    let channel_name = event.channel_name();
    let Some(context_message) = dispatch::build_event_context_message(event) else {
        return;
    };
    let synthetic_message = ChannelMessage {
        id: format!("event::{channel_name}::{conversation_id}"),
        sender: sender.to_string(),
        content: context_message.clone(),
        channel: channel_name.to_string(),
        context_hint: Some(EVENT_CONTEXT_HINT.to_string()),
        conversation_id: Some(conversation_id.to_string()),
        thread_id: None,
        reply_to: None,
        message_id: None,
        timestamp: 0,
        attachments: Vec::new(),
    };
    let thinking_key = prompt::channel_thinking_state_key(&rt.config, &synthetic_message);
    let routing_group = routing::resolve_routing_group(&rt.config, &synthetic_message);
    let group_isolation =
        routing::resolve_group_isolation(&rt.config, &routing_group, rt.runtime_sandbox_class);

    let (effective_autonomy, tool_allowlist) =
        dispatch::resolve_channel_policy_for_name(rt, channel_name);
    let ctx = dispatch::build_event_execution_context(
        rt,
        channel_name,
        sender,
        &thinking_key,
        Some(conversation_id),
        routing_group,
        group_isolation,
        effective_autonomy,
        tool_allowlist,
    )
    .await;

    let execution = stream::execute_channel_tool_loop(
        rt,
        ChannelToolLoopInput {
            channel_name,
            sender,
            conversation_id: Some(conversation_id),
            attachments: &[],
            user_message: &context_message,
            thinking_key: &thinking_key,
            reply_target: conversation_id,
            enable_streaming: false,
            channel_context_hint: Some(EVENT_CONTEXT_HINT),
        },
        &ctx,
    )
    .await;
    let ToolLoopExecutionArtifacts {
        result,
        stream_state,
        show_reasoning,
        media_processor: _,
    } = execution;

    stream::join_stream_task(stream_state.stream_forward_handle, "event stream forward").await;
    stream::join_stream_task(
        stream_state.discord_thinking_embed_handle,
        "event discord thinking embed",
    )
    .await;

    finalize_channel_event_execution(
        rt,
        result,
        show_reasoning,
        channel_name,
        conversation_id,
        sender,
    )
    .await;
}

async fn finalize_channel_event_execution(
    rt: &ChannelRuntime,
    result: Result<TurnExecutionOutcome>,
    show_reasoning: bool,
    channel_name: &str,
    conversation_id: &str,
    sender: &str,
) {
    match result {
        Ok(outcome) => {
            let result = outcome.result;
            if let LoopStopReason::Error(error) = &result.stop_reason {
                tracing::warn!(channel = channel_name, sender, %error, "event tool loop returned error");
                return;
            }

            let final_text = if show_reasoning {
                result.final_text
            } else {
                strip_reasoning(&result.final_text)
            };

            if final_text.is_empty() {
                return;
            }

            if let Err(error) =
                reply::send_event_reply(&rt.channels, channel_name, &final_text, conversation_id)
                    .await
            {
                tracing::warn!(
                    channel = channel_name,
                    conversation_id,
                    error = %error,
                    "failed to send event-triggered reply"
                );
            }
        }
        Err(error) => {
            tracing::warn!(channel = channel_name, sender, %error, "event tool loop failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_discord_channel_context_hint_prefers_explicit_message_hint() {
        let msg = ChannelMessage {
            id: "msg-1".to_string(),
            sender: "user-1".to_string(),
            content: "anyone know why this broke?".to_string(),
            channel: "discord".to_string(),
            conversation_id: Some("channel-1".to_string()),
            thread_id: None,
            reply_to: None,
            message_id: Some("message-1".to_string()),
            timestamp: 1,
            attachments: Vec::new(),
            context_hint: Some(
                "[Channel Context: Ambient pickup — brief, useful, and easy to ignore]".to_string(),
            ),
        };

        assert_eq!(
            resolve_discord_channel_context_hint(&msg),
            Some("[Channel Context: Ambient pickup — brief, useful, and easy to ignore]")
        );
    }

    #[test]
    fn resolve_discord_channel_context_hint_falls_back_to_thread_context() {
        let msg = ChannelMessage {
            id: "msg-2".to_string(),
            sender: "user-1".to_string(),
            content: "follow-up".to_string(),
            channel: "discord".to_string(),
            conversation_id: Some("channel-1".to_string()),
            thread_id: Some("thread-1".to_string()),
            reply_to: None,
            message_id: Some("message-2".to_string()),
            timestamp: 2,
            attachments: Vec::new(),
            context_hint: None,
        };

        assert_eq!(
            resolve_discord_channel_context_hint(&msg),
            Some("[Channel Context: Thread continuation — stay on topic, build on prior context]")
        );
    }

    #[test]
    fn blocked_channel_ingress_is_not_persistable() {
        let blocked = ExternalIngressPolicyOutcome {
            model_input: "blocked".to_string(),
            persisted_summary: "content_omitted".to_string(),
            blocked: true,
        };
        let allowed = ExternalIngressPolicyOutcome {
            model_input: "allowed".to_string(),
            persisted_summary: "content_omitted".to_string(),
            blocked: false,
        };

        assert!(!should_persist_channel_ingress(&blocked));
        assert!(should_persist_channel_ingress(&allowed));
    }
}
