//! Discord Gateway WebSocket client: heartbeat, identify/resume,
//! reconnect with exponential backoff, and event dispatch.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::{Instant, interval};
use tokio_tungstenite::tungstenite::Message;

use super::types::{DEFAULT_HEARTBEAT_INTERVAL_MS, DiscordChannelType, GatewayOpcode};
use crate::contracts::ids::{ChannelId, MessageId, SessionId, UserId};

/// Session + resume URL bundled so they are always read/written atomically.
#[derive(Debug, Clone, Default)]
pub struct ResumeState {
    /// The session ID from the last READY event.
    pub session_id: Option<SessionId>,
    /// The gateway URL to use when resuming a disconnected session.
    pub resume_gateway_url: Option<String>,
}

/// Shared mutable state for the Discord gateway connection.
#[derive(Debug)]
pub struct DiscordGatewayState {
    /// Session ID and resume URL, updated atomically.
    pub resume: Mutex<ResumeState>,
    /// Last received event sequence number (-1 means unset).
    pub sequence: AtomicI64,
}

impl Default for DiscordGatewayState {
    fn default() -> Self {
        Self {
            resume: Mutex::new(ResumeState::default()),
            sequence: AtomicI64::new(-1),
        }
    }
}

/// A file attachment from a Discord message before processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAttachment {
    /// CDN URL of the attachment.
    pub url: String,
    /// Original filename, if provided.
    pub filename: Option<String>,
    /// MIME type, if provided by Discord.
    pub content_type: Option<String>,
}

/// Parsed events received from the Discord gateway WebSocket.
#[derive(Debug, Clone, PartialEq)]
pub enum GatewayEvent {
    /// A new message was sent in a channel.
    MessageCreate {
        channel_id: ChannelId,
        author_id: UserId,
        author_is_bot: bool,
        reply_to_author_id: Option<UserId>,
        content: String,
        guild_id: Option<String>,
        thread_id: Option<String>,
        message_id: MessageId,
        attachments: Vec<RawAttachment>,
    },
    /// A reaction was added to a message.
    ReactionAdd {
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
        guild_id: Option<String>,
    },
    /// A reaction was removed from a message.
    ReactionRemove {
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
        guild_id: Option<String>,
    },
    /// A message was edited.
    MessageUpdate {
        channel_id: ChannelId,
        message_id: MessageId,
        content: Option<String>,
        author_id: Option<UserId>,
        guild_id: Option<String>,
    },
    /// A message was deleted.
    MessageDelete {
        channel_id: ChannelId,
        message_id: MessageId,
        guild_id: Option<String>,
    },
    /// A user started typing in a channel.
    TypingStart {
        channel_id: ChannelId,
        user_id: UserId,
        guild_id: Option<String>,
        timestamp: u64,
    },
    /// A slash command, button, or modal interaction was created.
    InteractionCreate {
        interaction_key: String,
        interaction_token: String,
        interaction_type: u64,
        channel_id: ChannelId,
        user_id: UserId,
        guild_id: Option<String>,
        data: serde_json::Value,
    },
    /// The gateway session is ready (initial handshake complete).
    Ready {
        session_id: SessionId,
        resume_gateway_url: String,
        user_id: UserId,
    },
}

/// WebSocket client managing the Discord gateway lifecycle.
pub struct DiscordGateway {
    bot_token: String,
    intents: u64,
    state: Arc<DiscordGatewayState>,
    presence: Option<serde_json::Value>,
}

impl DiscordGateway {
    /// Create a new gateway client with the given token and intents.
    pub fn new(
        bot_token: String,
        intents: u64,
        state: Arc<DiscordGatewayState>,
        presence: Option<serde_json::Value>,
    ) -> Self {
        Self {
            bot_token,
            intents,
            state,
            presence,
        }
    }

    /// Connect to the Discord gateway and dispatch events to `tx`.
    ///
    /// # Errors
    ///
    /// Returns an error when gateway URL resolution, websocket connect,
    /// heartbeat setup, or event handling fails.
    pub async fn connect_and_listen(
        &self,
        http: &super::http_client::DiscordHttpClient,
        tx: &tokio::sync::mpsc::Sender<GatewayEvent>,
    ) -> Result<()> {
        let gateway_url = self.resolve_gateway_url(http).await?;
        let ws_url = build_gateway_ws_url(&gateway_url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .with_context(|| format!("connect Discord gateway websocket: {ws_url}"))?;
        let (mut write, mut read) = ws_stream.split();

        let heartbeat_interval_ms = read_hello_heartbeat_interval(&mut read).await?;
        self.send_identify_or_resume(&mut write).await?;

        let mut heartbeat = interval(Duration::from_millis(heartbeat_interval_ms));
        let heartbeat_acked = AtomicBool::new(true);
        let mut ack_deadline: Option<Instant> = None;

        // CANCEL-SAFE: all branches mutate only local state (ack_deadline, heartbeat_acked
        // AtomicBool). Dropping mid-heartbeat doesn't corrupt server state.
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if !self
                        .send_heartbeat_if_healthy(&mut write, &heartbeat_acked, &mut ack_deadline, heartbeat_interval_ms)
                        .await?
                    {
                        tracing::warn!("Discord gateway heartbeat ACK missing; reconnecting");
                        return Ok(());
                    }
                }
                () = wait_for_ack_timeout(ack_deadline) => {
                    if !heartbeat_acked.load(Ordering::SeqCst) {
                        tracing::warn!("Discord gateway heartbeat ACK timeout; reconnecting");
                        return Ok(());
                    }
                    ack_deadline = None;
                }
                message = read.next() => {
                    let Some(message) = message else {
                        tracing::warn!("Discord gateway socket closed; reconnecting");
                        return Ok(());
                    };

                    let message = message.context("read Discord gateway message")?;
                    if !self
                        .handle_gateway_message(message, tx, &mut write, &heartbeat_acked, &mut ack_deadline, heartbeat_interval_ms)
                        .await?
                    {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn resolve_gateway_url(
        &self,
        http: &super::http_client::DiscordHttpClient,
    ) -> Result<String> {
        if let Some(url) = self.state.resume.lock().await.resume_gateway_url.clone()
            && !url.is_empty()
        {
            return Ok(url);
        }

        let gw_resp = http
            .get_gateway_bot()
            .await
            .context("fetch Discord gateway/bot URL")?;
        let url = gw_resp
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("wss://gateway.discord.gg")
            .to_string();
        Ok(url)
    }

    async fn send_identify_or_resume<WsSink>(&self, write: &mut WsSink) -> Result<()>
    where
        WsSink: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        if let Some(session_id) = self.state.resume.lock().await.session_id.clone() {
            let payload = json!({
                "op": GatewayOpcode::Resume as u8,
                "d": {
                    "token": self.bot_token,
                    "session_id": session_id,
                    "seq": self.current_sequence_value(),
                }
            });
            write
                .send(Message::Text(payload.to_string().into()))
                .await
                .context("send Discord gateway resume")?;
            return Ok(());
        }

        let mut identify_data = json!({
            "token": self.bot_token,
            "intents": self.intents,
            "properties": {
                "os": std::env::consts::OS,
                "browser": "asterel",
                "device": "asterel"
            }
        });

        if let Some(presence) = &self.presence {
            identify_data["presence"] = presence.clone();
        }

        let payload = json!({
            "op": GatewayOpcode::Identify as u8,
            "d": identify_data,
        });

        write
            .send(Message::Text(payload.to_string().into()))
            .await
            .context("send Discord gateway identify")
    }

    async fn send_heartbeat_if_healthy<WsSink>(
        &self,
        write: &mut WsSink,
        heartbeat_acked: &AtomicBool,
        ack_deadline: &mut Option<Instant>,
        heartbeat_interval_ms: u64,
    ) -> Result<bool>
    where
        WsSink: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        if !heartbeat_acked.load(Ordering::SeqCst) {
            return Ok(false);
        }

        let payload = json!({
            "op": GatewayOpcode::Heartbeat as u8,
            "d": self.current_sequence(),
        });

        write
            .send(Message::Text(payload.to_string().into()))
            .await
            .context("send Discord gateway heartbeat")?;

        heartbeat_acked.store(false, Ordering::SeqCst);
        *ack_deadline = Some(Instant::now() + Duration::from_millis(heartbeat_interval_ms));
        Ok(true)
    }

    async fn handle_gateway_message<WsSink>(
        &self,
        message: Message,
        tx: &tokio::sync::mpsc::Sender<GatewayEvent>,
        write: &mut WsSink,
        heartbeat_acked: &AtomicBool,
        ack_deadline: &mut Option<Instant>,
        heartbeat_interval_ms: u64,
    ) -> Result<bool>
    where
        WsSink: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        let Some(raw) = websocket_message_to_text(message) else {
            return Ok(true);
        };

        let payload: serde_json::Value =
            serde_json::from_str(&raw).context("parse Discord gateway payload")?;

        if let Some(sequence) = payload.get("s").and_then(serde_json::Value::as_i64) {
            self.state.sequence.store(sequence, Ordering::SeqCst);
        }

        let op = payload
            .get("op")
            .and_then(serde_json::Value::as_u64)
            .and_then(GatewayOpcode::from_u64);

        match op {
            Some(GatewayOpcode::Heartbeat) => {
                if !self
                    .send_heartbeat_if_healthy(
                        write,
                        heartbeat_acked,
                        ack_deadline,
                        heartbeat_interval_ms,
                    )
                    .await?
                {
                    return Ok(false);
                }
                Ok(true)
            }
            Some(GatewayOpcode::HeartbeatAck) => {
                heartbeat_acked.store(true, Ordering::SeqCst);
                *ack_deadline = None;
                Ok(true)
            }
            Some(GatewayOpcode::Reconnect) => {
                tracing::info!("Discord gateway requested reconnect");
                Ok(false)
            }
            Some(GatewayOpcode::InvalidSession) => {
                self.handle_invalid_session(&payload).await?;
                Ok(false)
            }
            Some(GatewayOpcode::Dispatch) => {
                self.handle_dispatch_payload(&payload, tx).await?;
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    async fn handle_dispatch_payload(
        &self,
        payload: &serde_json::Value,
        tx: &tokio::sync::mpsc::Sender<GatewayEvent>,
    ) -> Result<()> {
        let event_type = payload
            .get("t")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let Some(data) = payload.get("d") else {
            return Ok(());
        };

        if let Some(event) = parse_dispatch_event(event_type, data) {
            if let GatewayEvent::Ready {
                session_id,
                resume_gateway_url,
                ..
            } = &event
            {
                self.persist_ready_state(session_id, resume_gateway_url)
                    .await;
            }
            tx.send(event)
                .await
                .context("dispatch parsed Discord gateway event")?;
        }

        Ok(())
    }

    async fn persist_ready_state(&self, session_id: &SessionId, resume_gateway_url: &str) {
        let mut guard = self.state.resume.lock().await;
        guard.session_id = Some(session_id.clone());
        guard.resume_gateway_url = Some(resume_gateway_url.to_string());
    }

    async fn handle_invalid_session(&self, payload: &serde_json::Value) -> Result<()> {
        let can_resume = payload
            .get("d")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        self.state.sequence.store(-1, Ordering::SeqCst);

        {
            let mut guard = self.state.resume.lock().await;
            guard.session_id = None;
            if !can_resume {
                tracing::warn!(
                    "Discord gateway invalid session (resume denied), clearing resume URL"
                );
                guard.resume_gateway_url = None;
            }
        }

        if can_resume {
            let wait_secs = invalid_session_backoff_secs();
            tracing::warn!(
                "Discord gateway invalid session (resume allowed), waiting {wait_secs}s before reconnect"
            );
            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
        }

        Ok(())
    }

    fn current_sequence(&self) -> serde_json::Value {
        let sequence = self.current_sequence_value();
        if sequence < 0 {
            serde_json::Value::Null
        } else {
            json!(sequence)
        }
    }

    fn current_sequence_value(&self) -> i64 {
        self.state.sequence.load(Ordering::SeqCst)
    }
}

/// Parse a raw Discord dispatch event into a typed `GatewayEvent`.
pub fn parse_dispatch_event(event_type: &str, d: &serde_json::Value) -> Option<GatewayEvent> {
    match event_type {
        "READY" => parse_ready_event(d),
        "MESSAGE_CREATE" => parse_message_create_event(d),
        "MESSAGE_UPDATE" => parse_message_update_event(d),
        "MESSAGE_DELETE" => parse_message_delete_event(d),
        "MESSAGE_REACTION_ADD" => parse_reaction_event(d, true),
        "MESSAGE_REACTION_REMOVE" => parse_reaction_event(d, false),
        "TYPING_START" => parse_typing_start_event(d),
        "INTERACTION_CREATE" => parse_interaction_create_event(d),
        "RESUMED" => {
            tracing::info!("Discord gateway session resumed");
            None
        }
        _ => None,
    }
}

fn parse_ready_event(d: &serde_json::Value) -> Option<GatewayEvent> {
    let session_id = SessionId::new(d.get("session_id")?.as_str()?);
    let resume_gateway_url = d.get("resume_gateway_url")?.as_str()?.to_string();
    let user_id = UserId::new(d.get("user")?.get("id")?.as_str()?);

    Some(GatewayEvent::Ready {
        session_id,
        resume_gateway_url,
        user_id,
    })
}

fn parse_message_create_event(d: &serde_json::Value) -> Option<GatewayEvent> {
    let channel_id = ChannelId::new(d.get("channel_id")?.as_str()?);
    let author = d.get("author")?;
    let author_id = UserId::new(author.get("id")?.as_str()?);
    let author_is_bot = author
        .get("bot")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let reply_to_author_id = d
        .get("referenced_message")
        .and_then(|message| message.get("author"))
        .and_then(|author| author.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(UserId::new);
    let content = d
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let guild_id = d
        .get("guild_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let message_id = MessageId::new(d.get("id")?.as_str()?);
    let channel_type = d.get("type").and_then(serde_json::Value::as_u64);
    let thread_id = channel_type
        .and_then(DiscordChannelType::from_u64)
        .filter(|kind| kind.is_thread())
        .map(|_| channel_id.to_string());

    Some(GatewayEvent::MessageCreate {
        channel_id,
        author_id,
        author_is_bot,
        reply_to_author_id,
        content,
        guild_id,
        thread_id,
        message_id,
        attachments: parse_raw_attachments(d),
    })
}

fn parse_reaction_event(d: &serde_json::Value, is_add: bool) -> Option<GatewayEvent> {
    let channel_id = ChannelId::new(d.get("channel_id")?.as_str()?);
    let message_id = MessageId::new(d.get("message_id")?.as_str()?);
    let user_id = UserId::new(d.get("user_id")?.as_str()?);
    let emoji = d
        .get("emoji")?
        .get("name")?
        .as_str()
        .map_or_else(|| String::from("unknown"), str::to_string);
    let guild_id = d
        .get("guild_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    if is_add {
        Some(GatewayEvent::ReactionAdd {
            channel_id,
            message_id,
            user_id,
            emoji,
            guild_id,
        })
    } else {
        Some(GatewayEvent::ReactionRemove {
            channel_id,
            message_id,
            user_id,
            emoji,
            guild_id,
        })
    }
}

fn parse_message_update_event(d: &serde_json::Value) -> Option<GatewayEvent> {
    let channel_id = ChannelId::new(d.get("channel_id")?.as_str()?);
    let message_id = MessageId::new(d.get("id")?.as_str()?);
    let content = d
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let author_id = d
        .get("author")
        .and_then(|author| author.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(UserId::new);
    let guild_id = d
        .get("guild_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    Some(GatewayEvent::MessageUpdate {
        channel_id,
        message_id,
        content,
        author_id,
        guild_id,
    })
}

fn parse_message_delete_event(d: &serde_json::Value) -> Option<GatewayEvent> {
    let channel_id = ChannelId::new(d.get("channel_id")?.as_str()?);
    let message_id = MessageId::new(d.get("id")?.as_str()?);
    let guild_id = d
        .get("guild_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    Some(GatewayEvent::MessageDelete {
        channel_id,
        message_id,
        guild_id,
    })
}

fn parse_typing_start_event(d: &serde_json::Value) -> Option<GatewayEvent> {
    let channel_id = ChannelId::new(d.get("channel_id")?.as_str()?);
    let user_id = UserId::new(d.get("user_id")?.as_str()?);
    let guild_id = d
        .get("guild_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let timestamp = d
        .get("timestamp")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    Some(GatewayEvent::TypingStart {
        channel_id,
        user_id,
        guild_id,
        timestamp,
    })
}

fn parse_interaction_create_event(d: &serde_json::Value) -> Option<GatewayEvent> {
    let interaction_key = d.get("id")?.as_str()?.to_string();
    let interaction_token = d.get("token")?.as_str()?.to_string();
    let interaction_type = d.get("type")?.as_u64()?;
    let channel_id = ChannelId::new(d.get("channel_id")?.as_str()?);
    let guild_id = d
        .get("guild_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let user_id = d
        .get("member")
        .and_then(|member| member.get("user"))
        .and_then(|user| user.get("id"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            d.get("user")
                .and_then(|user| user.get("id"))
                .and_then(serde_json::Value::as_str)
        })?;
    let user_id = UserId::new(user_id);

    Some(GatewayEvent::InteractionCreate {
        interaction_key,
        interaction_token,
        interaction_type,
        channel_id,
        user_id,
        guild_id,
        data: d.get("data").cloned().unwrap_or(serde_json::Value::Null),
    })
}

fn parse_raw_attachments(d: &serde_json::Value) -> Vec<RawAttachment> {
    d.get("attachments")
        .and_then(serde_json::Value::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .filter_map(|attachment| {
                    let url = attachment.get("url")?.as_str()?.to_string();
                    Some(RawAttachment {
                        url,
                        filename: attachment
                            .get("filename")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        content_type: attachment
                            .get("content_type")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn wait_for_ack_timeout(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        futures_util::future::pending::<()>().await;
    }
}

async fn read_hello_heartbeat_interval<WsRead>(read: &mut WsRead) -> Result<u64>
where
    WsRead:
        Stream<Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(message) = read.next().await {
        let message = message.context("read Discord gateway hello payload")?;
        let Some(raw) = websocket_message_to_text(message) else {
            continue;
        };

        let payload: serde_json::Value =
            serde_json::from_str(&raw).context("parse Discord gateway hello JSON")?;

        let op = payload
            .get("op")
            .and_then(serde_json::Value::as_u64)
            .and_then(GatewayOpcode::from_u64);

        if op == Some(GatewayOpcode::Hello) {
            let interval_ms = payload
                .get("d")
                .and_then(|d| d.get("heartbeat_interval"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_MS);
            return Ok(interval_ms);
        }
    }

    Err(anyhow::anyhow!("Discord gateway closed before Hello"))
}

fn websocket_message_to_text(message: Message) -> Option<String> {
    match message {
        Message::Text(text) => Some(text.to_string()),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok(),
        _ => None,
    }
}

fn build_gateway_ws_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/?v=10&encoding=json")
}

fn invalid_session_backoff_secs() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    1 + u64::from(nanos % 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ids::SessionId;

    #[tokio::test]
    async fn gateway_state_default_construction() {
        let state = DiscordGatewayState::default();
        assert_eq!(state.sequence.load(Ordering::SeqCst), -1);
        let resume = state.resume.lock().await;
        assert!(resume.session_id.is_none());
        assert!(resume.resume_gateway_url.is_none());
    }

    #[test]
    fn parse_message_create_event_payload() {
        let payload = json!({
            "id": "msg-1",
            "channel_id": "thread-42",
            "type": 11,
            "guild_id": "guild-7",
            "content": "hello",
            "author": {
                "id": "user-1",
                "bot": false
            },
            "attachments": [
                {
                    "url": "https://cdn.discordapp.com/file.png",
                    "filename": "file.png",
                    "content_type": "image/png"
                }
            ]
        });

        let event = parse_dispatch_event("MESSAGE_CREATE", &payload);
        assert!(event.is_some());

        if let Some(GatewayEvent::MessageCreate {
            channel_id,
            author_id,
            author_is_bot,
            reply_to_author_id,
            content,
            guild_id,
            thread_id,
            message_id,
            attachments,
        }) = event
        {
            assert_eq!(channel_id, ChannelId::new("thread-42"));
            assert_eq!(author_id, UserId::new("user-1"));
            assert!(!author_is_bot);
            assert_eq!(reply_to_author_id, None);
            assert_eq!(content, "hello");
            assert_eq!(guild_id.as_deref(), Some("guild-7"));
            assert_eq!(thread_id.as_deref(), Some("thread-42"));
            assert_eq!(message_id, MessageId::new("msg-1"));
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].filename.as_deref(), Some("file.png"));
        } else {
            panic!("expected MessageCreate event");
        }
    }

    #[test]
    fn parse_message_create_event_records_reply_author_id() {
        let payload = json!({
            "id": "msg-2",
            "channel_id": "thread-42",
            "type": 11,
            "guild_id": "guild-7",
            "content": "replying here",
            "author": {
                "id": "user-1",
                "bot": false
            },
            "referenced_message": {
                "author": {
                    "id": "bot-user",
                    "bot": true
                }
            }
        });

        let event = parse_dispatch_event("MESSAGE_CREATE", &payload);
        assert!(matches!(
            event,
            Some(GatewayEvent::MessageCreate {
                reply_to_author_id: Some(ref author_id),
                ..
            }) if author_id == &UserId::new("bot-user")
        ));
    }

    #[test]
    fn parse_ready_event_payload() {
        let payload = json!({
            "session_id": "session-1",
            "resume_gateway_url": "wss://gateway.discord.gg",
            "user": {
                "id": "bot-user"
            }
        });

        let event = parse_dispatch_event("READY", &payload);
        assert!(event.is_some());

        if let Some(GatewayEvent::Ready {
            session_id,
            resume_gateway_url,
            user_id,
        }) = event
        {
            assert_eq!(session_id, SessionId::new("session-1"));
            assert_eq!(resume_gateway_url, "wss://gateway.discord.gg");
            assert_eq!(user_id, UserId::new("bot-user"));
        } else {
            panic!("expected Ready event");
        }
    }

    #[test]
    fn parse_message_update_event_payload() {
        let payload = json!({
            "id": "msg-edit-1",
            "channel_id": "ch-1",
            "content": "edited content",
            "author": { "id": "user-1" },
            "guild_id": "guild-1"
        });

        let event = parse_dispatch_event("MESSAGE_UPDATE", &payload);
        assert!(event.is_some());

        if let Some(GatewayEvent::MessageUpdate {
            channel_id,
            message_id,
            content,
            author_id,
            guild_id,
        }) = event
        {
            assert_eq!(channel_id, ChannelId::new("ch-1"));
            assert_eq!(message_id, MessageId::new("msg-edit-1"));
            assert_eq!(content.as_deref(), Some("edited content"));
            assert_eq!(author_id.as_ref().map(UserId::as_str), Some("user-1"));
            assert_eq!(guild_id.as_deref(), Some("guild-1"));
        } else {
            panic!("expected MessageUpdate event");
        }
    }

    #[test]
    fn parse_message_update_partial_without_content() {
        let payload = json!({
            "id": "msg-edit-2",
            "channel_id": "ch-2",
            "guild_id": "guild-1"
        });

        let event = parse_dispatch_event("MESSAGE_UPDATE", &payload);
        assert!(event.is_some());

        if let Some(GatewayEvent::MessageUpdate {
            content, author_id, ..
        }) = event
        {
            assert!(content.is_none());
            assert!(author_id.is_none());
        } else {
            panic!("expected MessageUpdate event");
        }
    }

    #[test]
    fn parse_message_delete_event_payload() {
        let payload = json!({
            "id": "msg-del-1",
            "channel_id": "ch-1",
            "guild_id": "guild-1"
        });

        let event = parse_dispatch_event("MESSAGE_DELETE", &payload);
        assert!(event.is_some());

        if let Some(GatewayEvent::MessageDelete {
            channel_id,
            message_id,
            guild_id,
        }) = event
        {
            assert_eq!(channel_id, ChannelId::new("ch-1"));
            assert_eq!(message_id, MessageId::new("msg-del-1"));
            assert_eq!(guild_id.as_deref(), Some("guild-1"));
        } else {
            panic!("expected MessageDelete event");
        }
    }

    #[test]
    fn parse_typing_start_event_payload() {
        let payload = json!({
            "channel_id": "ch-1",
            "user_id": "user-1",
            "guild_id": "guild-1",
            "timestamp": 1_700_000_000
        });

        let event = parse_dispatch_event("TYPING_START", &payload);
        assert!(event.is_some());

        if let Some(GatewayEvent::TypingStart {
            channel_id,
            user_id,
            guild_id,
            timestamp,
        }) = event
        {
            assert_eq!(channel_id, ChannelId::new("ch-1"));
            assert_eq!(user_id, UserId::new("user-1"));
            assert_eq!(guild_id.as_deref(), Some("guild-1"));
            assert_eq!(timestamp, 1_700_000_000);
        } else {
            panic!("expected TypingStart event");
        }
    }

    #[test]
    fn unknown_event_returns_none() {
        let payload = json!({"foo": "bar"});
        assert!(parse_dispatch_event("SOMETHING_ELSE", &payload).is_none());
    }

    #[test]
    fn parse_raw_attachment_from_json() {
        let payload = json!({
            "attachments": [
                {
                    "url": "https://cdn.discordapp.com/attachment.txt",
                    "filename": "attachment.txt",
                    "content_type": "text/plain"
                }
            ]
        });

        let attachments = parse_raw_attachments(&payload);
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            attachments[0].url,
            "https://cdn.discordapp.com/attachment.txt"
        );
        assert_eq!(attachments[0].filename.as_deref(), Some("attachment.txt"));
        assert_eq!(attachments[0].content_type.as_deref(), Some("text/plain"));
    }
}
