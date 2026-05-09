//! Reply delivery: sends text and media responses back to the originating
//! channel, with reasoning-block stripping and TTS synthesis support.
use std::sync::Arc;

use anyhow::Result;

use super::super::attachments::output_attachment_to_media_attachment;
use super::super::startup::ChannelRuntime;
use super::super::traits::{Channel, ChannelMessage, MediaAttachment, MediaContent};
use super::media::has_audio_input;
use crate::core::agent::tool_loop::{LoopStopReason, ToolLoopResult};
use crate::media::MediaProcessor;
use crate::utils::text::{
    strip_inference_markers, strip_internal_prompt_blocks, strip_reasoning, truncate_ellipsis,
};

pub(super) async fn reply_to_origin(
    channels: &[Arc<dyn Channel>],
    channel_name: &str,
    message: &str,
    sender: &str,
) -> Result<()> {
    for ch in channels {
        if ch.name() == channel_name {
            ch.send_chunked(message, sender).await?;
            break;
        }
    }
    Ok(())
}

pub(super) async fn send_media_to_origin(
    channels: &[Arc<dyn Channel>],
    channel_name: &str,
    attachment: &MediaAttachment,
    sender: &str,
) -> Result<()> {
    for ch in channels {
        if ch.name() == channel_name {
            ch.send_media(attachment, sender).await?;
            break;
        }
    }
    Ok(())
}

pub(super) async fn send_event_reply(
    channels: &[Arc<dyn Channel>],
    channel_name: &str,
    message: &str,
    conversation_id: &str,
) -> Result<()> {
    for ch in channels {
        if ch.name() == channel_name {
            ch.send(message, conversation_id).await?;
            break;
        }
    }
    Ok(())
}

pub(super) async fn send_tool_output_attachments(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    result: &ToolLoopResult,
) {
    for attachment in &result.attachments {
        tracing::trace!(
            channel = %msg.channel,
            sender = %msg.sender,
            mime_type = %attachment.mime_type,
            filename = ?attachment.filename,
            source = ?attachment.source,
            "processing tool output attachment"
        );

        let Some(channel_attachment) = output_attachment_to_media_attachment(attachment).await
        else {
            continue;
        };

        if let Err(error) = send_media_to_origin(
            &rt.channels,
            &msg.channel,
            &channel_attachment,
            reply_target,
        )
        .await
        {
            tracing::trace!(
                channel = %msg.channel,
                sender = %msg.sender,
                error = %error,
                "channel does not support sending tool output media"
            );
        }
    }
}

pub(super) async fn send_tool_loop_error_reply(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    error: &str,
) {
    eprintln!("  ✗ {}", t!("channels.llm_error", error = error));
    if let Err(reply_error) = reply_to_origin(
        &rt.channels,
        &msg.channel,
        &format!("! Error: {error}"),
        reply_target,
    )
    .await
    {
        tracing::warn!(%reply_error, "failed to send channel error reply");
    }
}

pub(super) async fn send_synthesized_voice_reply(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    media_processor: &MediaProcessor,
    text: &str,
) {
    if !has_audio_input(msg) {
        return;
    }

    match media_processor.synthesize_speech(text).await {
        Ok(Some(speech)) => {
            let voice_attachment = MediaAttachment {
                mime_type: speech.mime_type,
                data: MediaContent::Bytes(speech.bytes),
                filename: Some(speech.filename),
            };
            if let Err(error) =
                send_media_to_origin(&rt.channels, &msg.channel, &voice_attachment, reply_target)
                    .await
            {
                tracing::warn!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    error = %error,
                    "failed to send synthesized voice reply"
                );
            }
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                channel = %msg.channel,
                sender = %msg.sender,
                error = %error,
                "failed to synthesize voice reply"
            );
        }
    }
}

pub(super) async fn handle_tool_loop_success(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    result: ToolLoopResult,
    streamed_any_output: bool,
    show_reasoning: bool,
    media_processor: &MediaProcessor,
) {
    let speech_text = strip_inference_markers(&strip_reasoning(&result.final_text));
    let visible_final_text = if show_reasoning {
        strip_internal_prompt_blocks(&result.final_text)
    } else {
        strip_inference_markers(&strip_reasoning(&result.final_text))
    };
    if let LoopStopReason::Error(error) = &result.stop_reason {
        send_tool_loop_error_reply(rt, msg, reply_target, error).await;
        return;
    }

    super::stream::log_stop_reason(msg, &result.stop_reason);
    println!(
        "  › {} {}",
        t!("channels.reply"),
        truncate_ellipsis(&visible_final_text, 80)
    );
    if !streamed_any_output
        && !visible_final_text.is_empty()
        && let Err(error) = reply_to_origin(
            &rt.channels,
            &msg.channel,
            &visible_final_text,
            reply_target,
        )
        .await
    {
        eprintln!(
            "  ✗ {}",
            t!("channels.reply_fail", channel = msg.channel, error = error)
        );
    }

    send_tool_output_attachments(rt, msg, reply_target, &result).await;
    send_synthesized_voice_reply(rt, msg, reply_target, media_processor, &speech_text).await;
}
