//! IRC `Channel` trait implementation: TLS connection, SASL auth, channel
//! join, PRIVMSG listener, CRLF-safe message splitting, and reconnect.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};
// Use tokio_rustls's re-export of rustls types
use tokio_rustls::rustls;

use super::auth::encode_sasl_plain;
use super::message::{IRC_STYLE_PREFIX, SENDER_PREFIX_RESERVE, split_message};
use super::parse::IrcMessage;
use super::tls::NoVerify;
use crate::transport::channels::policy::{AllowlistMatch, is_allowed_user};
use crate::transport::channels::traits::{Channel, ChannelEvent, ChannelMessage};

/// Read timeout for IRC — if no data arrives within this duration, the
/// connection is considered dead. IRC servers typically PING every 60-120s.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Monotonic counter to ensure unique message IDs under burst traffic.
static MSG_SEQ: AtomicU64 = AtomicU64::new(0);

/// IRC over TLS channel.
///
/// Connects to an IRC server using TLS, joins configured channels,
/// and forwards PRIVMSG messages to the `Asterel` message bus.
/// Supports both channel messages and private messages (DMs).
pub struct IrcChannel {
    pub(super) server: String,
    pub(super) port: u16,
    pub(super) nickname: String,
    pub(super) username: String,
    pub(super) channels: Vec<String>,
    pub(super) allowed_users: Vec<String>,
    pub(super) server_password: Option<String>,
    pub(super) nickserv_password: Option<String>,
    pub(super) sasl_password: Option<String>,
    pub(super) verify_tls: bool,
    /// Shared write half of the TLS stream for sending messages.
    writer: Arc<Mutex<Option<WriteHalf>>>,
}

/// Configuration for constructing an `IrcChannel`.
pub struct IrcChannelConfig {
    pub server: String,
    pub port: u16,
    pub nickname: String,
    pub username: Option<String>,
    pub channels: Vec<String>,
    pub allowed_users: Vec<String>,
    pub server_password: Option<String>,
    pub nickserv_password: Option<String>,
    pub sasl_password: Option<String>,
    pub verify_tls: bool,
}

type WriteHalf = tokio::io::WriteHalf<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

impl IrcChannel {
    /// Creates a new IRC channel from the given configuration.
    #[must_use]
    pub fn new(config: IrcChannelConfig) -> Self {
        let IrcChannelConfig {
            server,
            port,
            nickname,
            username,
            channels,
            allowed_users,
            server_password,
            nickserv_password,
            sasl_password,
            verify_tls,
        } = config;
        let username = username.unwrap_or_else(|| nickname.clone());
        Self {
            server,
            port,
            nickname,
            username,
            channels,
            allowed_users,
            server_password,
            nickserv_password,
            sasl_password,
            verify_tls,
            writer: Arc::new(Mutex::new(None)),
        }
    }

    /// Checks if an IRC nickname is in the channel's user allowlist.
    pub(super) fn is_user_allowed(&self, nick: &str) -> bool {
        is_allowed_user(
            &self.allowed_users,
            nick,
            AllowlistMatch::AsciiCaseInsensitive,
        )
    }

    fn credential_kinds_requiring_verified_tls(&self) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        if self.server_password.is_some() {
            kinds.push("server_password");
        }
        if self.nickserv_password.is_some() {
            kinds.push("nickserv_password");
        }
        if self.sasl_password.is_some() {
            kinds.push("sasl_password");
        }
        kinds
    }

    fn ensure_tls_policy_allows_connection(&self) -> anyhow::Result<()> {
        if self.verify_tls {
            return Ok(());
        }

        let credential_kinds = self.credential_kinds_requiring_verified_tls();
        if !credential_kinds.is_empty() {
            anyhow::bail!(
                "IRC verify_tls=false cannot be used with credential-bearing auth ({})",
                credential_kinds.join(", ")
            );
        }

        tracing::warn!(
            server = %self.server,
            port = self.port,
            "IRC TLS certificate verification disabled; continuing only because no IRC credentials are configured"
        );
        Ok(())
    }

    /// Create a TLS connection to the IRC server.
    async fn connect(
        &self,
    ) -> anyhow::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        self.ensure_tls_policy_allows_connection()?;

        let addr = format!("{}:{}", self.server, self.port);
        let tcp = tokio::net::TcpStream::connect(&addr)
            .await
            .context("connect to IRC server")?;

        let tls_config = if self.verify_tls {
            let root_store: rustls::RootCertStore =
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned().collect();
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth()
        } else {
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth()
        };

        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));
        let domain = rustls::pki_types::ServerName::try_from(self.server.clone())
            .context("parse IRC server name for TLS")?;
        let tls = connector
            .connect(domain, tcp)
            .await
            .context("establish TLS connection to IRC")?;

        Ok(tls)
    }

    /// Send a raw IRC line (appends \r\n).
    async fn send_raw(writer: &mut WriteHalf, line: &str) -> anyhow::Result<()> {
        let data = format!("{line}\r\n");
        writer
            .write_all(data.as_bytes())
            .await
            .context("write IRC message to stream")?;
        writer.flush().await.context("flush IRC write stream")?;
        Ok(())
    }
}

impl IrcChannel {
    /// Handle SASL-related CAP, AUTHENTICATE, and numeric responses.
    async fn handle_sasl_message(
        &self,
        msg: &IrcMessage,
        current_nick: &str,
        sasl_pending: &mut bool,
    ) -> anyhow::Result<()> {
        match msg.command.as_str() {
            "CAP" => {
                if *sasl_pending && msg.params.iter().any(|p| p.contains("sasl")) {
                    if msg.params.iter().any(|p| p.contains("ACK")) {
                        let mut guard = self.writer.lock().await;
                        if let Some(ref mut w) = *guard {
                            Self::send_raw(w, "AUTHENTICATE PLAIN")
                                .await
                                .context("send IRC SASL AUTHENTICATE command")?;
                        }
                    } else if msg.params.iter().any(|p| p.contains("NAK")) {
                        tracing::warn!("IRC server does not support SASL, continuing without it");
                        *sasl_pending = false;
                        let mut guard = self.writer.lock().await;
                        if let Some(ref mut w) = *guard {
                            Self::send_raw(w, "CAP END")
                                .await
                                .context("send IRC CAP END after SASL NAK")?;
                        }
                    }
                }
            }

            "AUTHENTICATE" => {
                if *sasl_pending && msg.params.first().is_some_and(|p| p == "+") {
                    let encoded = encode_sasl_plain(
                        current_nick,
                        self.sasl_password.as_deref().unwrap_or(""),
                    );
                    let mut guard = self.writer.lock().await;
                    if let Some(ref mut w) = *guard {
                        Self::send_raw(w, &format!("AUTHENTICATE {encoded}"))
                            .await
                            .context("send IRC SASL credentials")?;
                    }
                }
            }

            // RPL_SASLSUCCESS (903)
            "903" => {
                *sasl_pending = false;
                let mut guard = self.writer.lock().await;
                if let Some(ref mut w) = *guard {
                    Self::send_raw(w, "CAP END")
                        .await
                        .context("send IRC CAP END after SASL success")?;
                }
            }

            // SASL failure (904, 905, 906, 907)
            "904" | "905" | "906" | "907" => {
                tracing::warn!("IRC SASL authentication failed ({})", msg.command);
                *sasl_pending = false;
                let mut guard = self.writer.lock().await;
                if let Some(ref mut w) = *guard {
                    Self::send_raw(w, "CAP END")
                        .await
                        .context("send IRC CAP END after SASL failure")?;
                }
            }

            _ => {}
        }
        Ok(())
    }

    /// Handle `RPL_WELCOME` (`001`): `NickServ` identification and channel joins.
    async fn handle_welcome(&self, current_nick: &str) -> anyhow::Result<()> {
        tracing::info!("IRC registered as {}", current_nick);

        if let Some(ref pass) = self.nickserv_password {
            let mut guard = self.writer.lock().await;
            if let Some(ref mut w) = *guard {
                Self::send_raw(w, &format!("PRIVMSG NickServ :IDENTIFY {pass}"))
                    .await
                    .context("send IRC NickServ identify")?;
            }
        }

        for chan in &self.channels {
            let mut guard = self.writer.lock().await;
            if let Some(ref mut w) = *guard {
                Self::send_raw(w, &format!("JOIN {chan}"))
                    .await
                    .context("send IRC JOIN command")?;
            }
        }
        Ok(())
    }

    /// Handle a single IRC protocol message during the read loop.
    ///
    /// Returns `Some(ChannelEvent)` when a PRIVMSG should be forwarded,
    /// `None` to continue the loop, or an `Err` to abort.
    async fn handle_irc_message(
        &self,
        msg: &IrcMessage,
        current_nick: &mut String,
        registered: &mut bool,
        sasl_pending: &mut bool,
    ) -> anyhow::Result<Option<ChannelEvent>> {
        match msg.command.as_str() {
            "PING" => {
                let token = msg.params.first().map_or("", String::as_str);
                let mut guard = self.writer.lock().await;
                if let Some(ref mut w) = *guard {
                    Self::send_raw(w, &format!("PONG :{token}"))
                        .await
                        .context("send IRC PONG response")?;
                }
            }

            "CAP" | "AUTHENTICATE" | "903" | "904" | "905" | "906" | "907" => {
                self.handle_sasl_message(msg, current_nick, sasl_pending)
                    .await?;
            }

            // RPL_WELCOME — registration complete
            "001" => {
                *registered = true;
                self.handle_welcome(current_nick).await?;
            }

            // ERR_NICKNAMEINUSE (433)
            "433" => {
                // Cap nick length to prevent unbounded growth beyond
                // the typical IRC server limit (30 chars).
                let alt = if current_nick.len() >= 30 {
                    format!("{}_", &current_nick[..current_nick.len() - 1])
                } else {
                    format!("{current_nick}_")
                };
                tracing::warn!("IRC nickname {current_nick} is in use, trying {alt}");
                let mut guard = self.writer.lock().await;
                if let Some(ref mut w) = *guard {
                    Self::send_raw(w, &format!("NICK {alt}"))
                        .await
                        .context("send IRC NICK change")?;
                }
                *current_nick = alt;
            }

            "PRIVMSG" => {
                if let Some(event) = self.handle_privmsg(msg, *registered) {
                    return Ok(Some(event));
                }
            }

            // ERR_PASSWDMISMATCH (464) or other fatal errors
            "464" => {
                anyhow::bail!("IRC password mismatch");
            }

            _ => {}
        }
        Ok(None)
    }

    /// Parse an incoming PRIVMSG into a `ChannelEvent`, applying allowlist and
    /// service-nick filters. Returns `None` when the message should be skipped.
    fn handle_privmsg(&self, msg: &IrcMessage, registered: bool) -> Option<ChannelEvent> {
        if !registered {
            return None;
        }

        let target = msg.params.first().map_or("", String::as_str);
        let text = msg.params.get(1).map_or("", String::as_str);
        let sender_nick = msg.nick().unwrap_or("unknown");

        // Skip messages from NickServ/ChanServ
        if sender_nick.eq_ignore_ascii_case("NickServ")
            || sender_nick.eq_ignore_ascii_case("ChanServ")
        {
            return None;
        }

        if !self.is_user_allowed(sender_nick) {
            return None;
        }

        // Determine reply target: if sent to a channel, reply to channel;
        // if DM (target == our nick), reply to sender
        let is_channel = target.starts_with('#') || target.starts_with('&');
        let reply_to = if is_channel {
            target.to_string()
        } else {
            sender_nick.to_string()
        };
        let content = if is_channel {
            format!("{IRC_STYLE_PREFIX}<{sender_nick}> {text}")
        } else {
            format!("{IRC_STYLE_PREFIX}{text}")
        };

        let seq = MSG_SEQ.fetch_add(1, Ordering::Relaxed);
        let channel_msg = ChannelMessage {
            id: format!("irc_{}_{seq}", chrono::Utc::now().timestamp_millis()),
            sender: sender_nick.to_string(),
            content,
            channel: "irc".to_string(),
            context_hint: None,
            conversation_id: Some(reply_to),
            thread_id: None,
            reply_to: None,
            message_id: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            attachments: Vec::new(),
        };

        Some(ChannelEvent::Message(channel_msg))
    }

    /// Perform SASL negotiation, PASS, NICK, and USER registration, then store
    /// the write-half for later `send()` calls.
    async fn register_connection(
        &self,
        writer: &mut WriteHalf,
        current_nick: &str,
    ) -> anyhow::Result<()> {
        // ── SASL negotiation ──
        if self.sasl_password.is_some() {
            Self::send_raw(writer, "CAP REQ :sasl")
                .await
                .context("send IRC SASL capability request")?;
        }

        // ── Server password ──
        if let Some(ref pass) = self.server_password {
            Self::send_raw(writer, &format!("PASS {pass}"))
                .await
                .context("send IRC server password")?;
        }

        // ── Nick/User registration ──
        Self::send_raw(writer, &format!("NICK {current_nick}"))
            .await
            .context("send IRC NICK command")?;
        Self::send_raw(writer, &format!("USER {} 0 * :Asterel", self.username))
            .await
            .context("send IRC USER command")?;

        Ok(())
    }
}

#[cfg(test)]
mod identity_tests {
    use super::{IrcChannel, IrcChannelConfig};
    use crate::transport::channels::irc::parse::IrcMessage;
    use crate::transport::channels::traits::ChannelEvent;

    fn test_channel() -> IrcChannel {
        IrcChannel::new(IrcChannelConfig {
            server: "irc.example.test".to_string(),
            port: 6697,
            nickname: "botname".to_string(),
            username: Some("botname".to_string()),
            channels: vec!["#room".to_string()],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            allowed_users: vec!["alice".to_string()],
            verify_tls: true,
        })
    }

    #[test]
    fn privmsg_preserves_sender_and_channel_conversation() {
        let channel = test_channel();
        let irc = IrcMessage::parse(":alice!a@host PRIVMSG #room :hello").expect("irc message");
        let event = channel
            .handle_privmsg(&irc, true)
            .expect("message should pass policy");
        let ChannelEvent::Message(message) = event else {
            panic!("expected channel message");
        };

        assert_eq!(message.sender, "alice");
        assert_eq!(message.conversation_id.as_deref(), Some("#room"));
    }

    #[test]
    fn unverified_tls_rejects_credential_bearing_irc_auth() {
        for (field, config) in [
            (
                "server_password",
                IrcChannelConfig {
                    server_password: Some("server-secret".to_string()),
                    verify_tls: false,
                    ..test_config()
                },
            ),
            (
                "nickserv_password",
                IrcChannelConfig {
                    nickserv_password: Some("nickserv-secret".to_string()),
                    verify_tls: false,
                    ..test_config()
                },
            ),
            (
                "sasl_password",
                IrcChannelConfig {
                    sasl_password: Some("sasl-secret".to_string()),
                    verify_tls: false,
                    ..test_config()
                },
            ),
        ] {
            let channel = IrcChannel::new(config);
            let error = channel
                .ensure_tls_policy_allows_connection()
                .expect_err("credential-bearing IRC auth must require verified TLS");
            let message = error.to_string();
            assert!(message.contains("verify_tls=false"), "{message}");
            assert!(message.contains(field), "{message}");
            assert!(!message.contains("secret"), "{message}");
        }
    }

    #[test]
    fn unverified_tls_without_irc_credentials_remains_explicitly_allowed() {
        let channel = IrcChannel::new(IrcChannelConfig {
            verify_tls: false,
            ..test_config()
        });

        channel
            .ensure_tls_policy_allows_connection()
            .expect("credential-free unverified IRC TLS is still an explicit operator opt-in");
    }

    fn test_config() -> IrcChannelConfig {
        IrcChannelConfig {
            server: "irc.example.test".to_string(),
            port: 6697,
            nickname: "botname".to_string(),
            username: Some("botname".to_string()),
            channels: vec!["#room".to_string()],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            allowed_users: vec!["alice".to_string()],
            verify_tls: true,
        }
    }
}

impl Channel for IrcChannel {
    fn name(&self) -> &'static str {
        "irc"
    }

    fn max_message_length(&self) -> usize {
        400
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut guard = self.writer.lock().await;
            let writer = guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("IRC not connected"))?;

            // Sanitize recipient to prevent CRLF injection into the IRC protocol stream.
            let recipient = recipient.replace(['\r', '\n'], "");

            // Calculate safe payload size:
            // 512 - sender prefix (~64 bytes for :nick!user@host) - "PRIVMSG " - target - " :" - "\r\n"
            let overhead = SENDER_PREFIX_RESERVE + 10 + recipient.len() + 2;
            let max_payload = 512_usize.saturating_sub(overhead);
            let chunks = split_message(message, max_payload);

            for chunk in chunks {
                Self::send_raw(writer, &format!("PRIVMSG {recipient} :{chunk}")).await?;
            }

            Ok(())
        })
    }

    fn listen<'a>(
        &'a self,
        tx: mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut current_nick = self.nickname.clone();
            tracing::info!(
                "IRC channel connecting to {}:{} as {}...",
                self.server,
                self.port,
                current_nick
            );

            let tls = self.connect().await.context("connect to IRC server")?;
            let (reader, mut writer) = tokio::io::split(tls);

            self.register_connection(&mut writer, &current_nick).await?;

            // Store writer for send()
            {
                let mut guard = self.writer.lock().await;
                *guard = Some(writer);
            }

            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();
            let mut registered = false;
            let mut sasl_pending = self.sasl_password.is_some();

            loop {
                line.clear();
                let n = tokio::time::timeout(READ_TIMEOUT, buf_reader.read_line(&mut line))
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("IRC read timed out (no data for {READ_TIMEOUT:?})")
                    })??;
                if n == 0 {
                    anyhow::bail!("IRC connection closed by server");
                }

                let Some(msg) = IrcMessage::parse(&line) else {
                    continue;
                };

                if let Some(event) = self
                    .handle_irc_message(&msg, &mut current_nick, &mut registered, &mut sasl_pending)
                    .await?
                    && tx.send(event).await.is_err()
                {
                    return Ok(());
                }
            }
        })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            // Lightweight connectivity check: TLS connect + QUIT
            match self.connect().await {
                Ok(tls) => {
                    let (_, mut writer) = tokio::io::split(tls);
                    if let Err(error) = Self::send_raw(&mut writer, "QUIT :health check").await {
                        tracing::warn!(error = %error, "failed to send IRC QUIT during health check");
                        return false;
                    }
                    true
                }
                Err(_) => false,
            }
        })
    }
}
