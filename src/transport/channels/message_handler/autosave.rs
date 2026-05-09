//! Channel autosave: builds memory ingestion envelopes from inbound
//! messages and persists them through the write-policy guard.
use std::sync::Arc;

use super::super::ingress_policy::channel_autosave_input;
use super::super::startup::ChannelRuntime;
use super::super::traits::ChannelMessage;
use crate::core::memory::ingestion::IngestionPipeline;
use crate::security::writeback_guard::enforce_external_autosave_write_policy;
use crate::utils::text::truncate_ellipsis;

/// Builds memory ingestion envelopes from an inbound channel message,
/// including per-attachment entries for Discord.
pub(super) fn build_channel_ingestion_envelopes(
    msg: &ChannelMessage,
    autosave_entity_id: &str,
    persisted_summary: &str,
) -> Vec<crate::core::memory::SignalEnvelope> {
    let source_kind = match msg.channel.as_str() {
        "discord" => crate::core::memory::SourceKind::Discord,
        "telegram" => crate::core::memory::SourceKind::Telegram,
        "slack" => crate::core::memory::SourceKind::Slack,
        _ => crate::core::memory::SourceKind::Api,
    };

    let mut base = crate::core::memory::SignalEnvelope::new(
        source_kind,
        format!("{}:{}", msg.channel, msg.sender),
        persisted_summary,
        autosave_entity_id,
    )
    .with_metadata("channel", &msg.channel)
    .with_metadata("sender", &msg.sender)
    .with_metadata("timestamp", msg.timestamp.to_string())
    .with_metadata("attachment_count", msg.attachments.len().to_string());

    if let Some(conversation_id) = &msg.conversation_id {
        base = base.with_metadata("conversation_id", conversation_id);
    }
    if let Some(thread_id) = &msg.thread_id {
        base = base.with_metadata("thread_id", thread_id);
    }
    if let Some(message_id) = &msg.message_id {
        base = base.with_metadata("message_id", message_id);
    }

    let mut envelopes = vec![base];
    if msg.channel == "discord" {
        for attachment in &msg.attachments {
            let filename = attachment.filename.as_deref().unwrap_or("unnamed");
            let attachment_content = format!(
                "discord attachment observed: file={filename} mime={}",
                attachment.mime_type
            );
            envelopes.push(
                crate::core::memory::SignalEnvelope::new(
                    source_kind,
                    format!("{}:{}:attachment:{filename}", msg.channel, msg.sender),
                    attachment_content,
                    autosave_entity_id,
                )
                .with_metadata("channel", &msg.channel)
                .with_metadata("sender", &msg.sender)
                .with_metadata("attachment", "true"),
            );
        }
    }

    envelopes
}

/// Persists the message via autosave and runs the ingestion pipeline.
pub(super) async fn autosave_and_ingest(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    autosave_entity_id: &str,
    persisted_summary: &str,
) {
    if rt.config.memory.auto_save {
        let policy_context = rt.tenant_policy_context.clone();
        if let Err(error) = policy_context.enforce_recall_scope(autosave_entity_id) {
            tracing::warn!(error, "channel autosave skipped due to policy context");
        } else {
            let event = channel_autosave_input(
                autosave_entity_id,
                &msg.channel,
                &msg.sender,
                persisted_summary.to_string(),
            );
            if let Err(error) = enforce_external_autosave_write_policy(&event) {
                tracing::warn!(%error, "channel autosave rejected by write policy");
            } else if let Err(error) = rt.mem.append_event(event).await {
                tracing::warn!(%error, "failed to autosave channel input");
            }
        }
    }

    if msg.channel != "cli" {
        let envelopes =
            build_channel_ingestion_envelopes(msg, autosave_entity_id, persisted_summary);
        let pipeline =
            crate::core::memory::ingestion::DefaultIngestPipeline::new(Arc::clone(&rt.mem));
        match pipeline.ingest_batch(envelopes).await {
            Ok(results) => {
                let accepted = results.iter().filter(|r| r.accepted).count();
                let dropped = results.len().saturating_sub(accepted);
                tracing::debug!(
                    channel = %msg.channel,
                    accepted,
                    dropped,
                    "ingestion pipeline processed channel message batch"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "ingestion pipeline failed for channel message");
            }
        }
    }
}

/// Prints a localized one-line summary of an incoming channel message.
pub(super) fn log_incoming_channel_message(msg: &ChannelMessage) {
    println!(
        "  › {}",
        t!(
            "channels.message_in",
            channel = msg.channel,
            sender = msg.sender,
            content = truncate_ellipsis(&msg.content, 80)
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::channels::traits::{MediaAttachment, MediaContent};

    fn discord_message_with_attachments() -> ChannelMessage {
        ChannelMessage {
            id: "msg-1".to_string(),
            sender: "user-42".to_string(),
            content: "hello from discord".to_string(),
            channel: "discord".to_string(),
            context_hint: None,
            conversation_id: Some("channel-77".to_string()),
            thread_id: Some("thread-9".to_string()),
            reply_to: None,
            message_id: Some("discord-msg-abc".to_string()),
            timestamp: 1_716_171_717,
            attachments: vec![
                MediaAttachment {
                    mime_type: "image/png".to_string(),
                    data: MediaContent::Url("https://cdn.discord.test/img.png".to_string()),
                    filename: Some("img.png".to_string()),
                },
                MediaAttachment {
                    mime_type: "application/pdf".to_string(),
                    data: MediaContent::Url("https://cdn.discord.test/doc.pdf".to_string()),
                    filename: Some("doc.pdf".to_string()),
                },
            ],
        }
    }

    #[test]
    fn discord_ingestion_envelopes_include_attachment_metadata_items() {
        let msg = discord_message_with_attachments();
        let envelopes =
            build_channel_ingestion_envelopes(&msg, "person:discord.user_42", "persisted summary");

        assert_eq!(envelopes.len(), 3);

        let base = &envelopes[0];
        assert_eq!(base.source_kind, crate::core::memory::SourceKind::Discord);
        assert_eq!(base.source_ref, "discord:user-42");
        assert_eq!(base.content, "persisted summary");
        assert_eq!(
            base.metadata.get("attachment_count").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            base.metadata.get("conversation_id").map(String::as_str),
            Some("channel-77")
        );
        assert_eq!(
            base.metadata.get("thread_id").map(String::as_str),
            Some("thread-9")
        );
        assert_eq!(
            base.metadata.get("message_id").map(String::as_str),
            Some("discord-msg-abc")
        );

        let attachment_1 = &envelopes[1];
        assert_eq!(
            attachment_1.source_ref,
            "discord:user-42:attachment:img.png"
        );
        assert!(attachment_1.content.contains("file=img.png"));
        assert!(attachment_1.content.contains("mime=image/png"));
        assert_eq!(
            attachment_1.metadata.get("attachment").map(String::as_str),
            Some("true")
        );

        let attachment_2 = &envelopes[2];
        assert_eq!(
            attachment_2.source_ref,
            "discord:user-42:attachment:doc.pdf"
        );
        assert!(attachment_2.content.contains("file=doc.pdf"));
        assert!(attachment_2.content.contains("mime=application/pdf"));
    }

    #[test]
    fn non_discord_ingestion_envelope_does_not_expand_attachments() {
        let mut msg = discord_message_with_attachments();
        msg.channel = "telegram".to_string();

        let envelopes =
            build_channel_ingestion_envelopes(&msg, "person:telegram.user_42", "persisted summary");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(
            envelopes[0].source_kind,
            crate::core::memory::SourceKind::Telegram
        );
        assert_eq!(
            envelopes[0]
                .metadata
                .get("attachment_count")
                .map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn discord_ingestion_envelopes_use_unnamed_fallback_for_missing_filename() {
        let mut msg = discord_message_with_attachments();
        msg.attachments[0].filename = None;

        let envelopes =
            build_channel_ingestion_envelopes(&msg, "person:discord.user_42", "persisted summary");

        assert_eq!(envelopes.len(), 3);
        let attachment = &envelopes[1];
        assert_eq!(attachment.source_ref, "discord:user-42:attachment:unnamed");
        assert!(attachment.content.contains("file=unnamed"));
    }

    #[test]
    fn discord_ingestion_envelopes_keep_single_base_when_no_attachments() {
        let mut msg = discord_message_with_attachments();
        msg.attachments.clear();

        let envelopes =
            build_channel_ingestion_envelopes(&msg, "person:discord.user_42", "persisted summary");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].source_ref, "discord:user-42");
        assert_eq!(
            envelopes[0]
                .metadata
                .get("attachment_count")
                .map(String::as_str),
            Some("0")
        );
    }
}
