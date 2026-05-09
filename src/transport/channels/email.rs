//! `Email` channel adapter: polls an `IMAP` mailbox for incoming messages
//! and sends replies via `SMTP`. Supports `TLS`, idle, and attachment handling.

use std::collections::HashSet;
use std::io::Write as IoWrite;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use mail_parser::{MessageParser, MimeHeaders};
use rustls_pki_types::ServerName;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep};
use tokio_rustls::rustls::{self, ClientConfig as TlsConfig};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::traits::{Channel, ChannelEvent, ChannelMessage};
use crate::config::schema::EmailConfig;

type ImapTlsStream = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// Escape a string for use inside `IMAP` double-quoted strings (`RFC 3501 §4.3`).
/// Backslash and double-quote are the only characters that need escaping.
fn escape_imap_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '"' => {
                out.push('\\');
                out.push(ch);
            }
            '\0' | '\r' | '\n' => {}
            _ => out.push(ch),
        }
    }
    out
}

/// `Email` channel with `IMAP` polling for inbound and `SMTP` for outbound.
pub struct EmailChannel {
    pub config: EmailConfig,
    seen_messages: Mutex<HashSet<String>>,
}

impl EmailChannel {
    #[must_use]
    pub fn new(config: EmailConfig) -> Self {
        Self {
            config,
            seen_messages: Mutex::new(HashSet::new()),
        }
    }

    /// Check if a sender email is in the allowlist.
    pub fn is_sender_allowed(&self, email: &str) -> bool {
        if self.config.allowed_senders.is_empty() {
            return false; // Empty = deny all
        }
        if self.config.allowed_senders.iter().any(|a| a == "*") {
            return true; // Wildcard = allow all
        }
        let email_lower = email.to_lowercase();
        self.config.allowed_senders.iter().any(|allowed| {
            if allowed.starts_with('@') {
                // Domain match with @ prefix: "@example.com"
                email_lower.ends_with(&allowed.to_lowercase())
            } else if allowed.contains('@') {
                // Full email address match
                allowed.eq_ignore_ascii_case(email)
            } else {
                // Domain match without @ prefix: "example.com"
                let domain = allowed.to_lowercase();
                email_lower.ends_with(&format!("@{domain}"))
            }
        })
    }

    /// Strip `HTML` tags from content (basic).
    #[must_use]
    pub fn strip_html(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;
        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => result.push(ch),
                _ => {}
            }
        }
        let mut normalized = String::with_capacity(result.len());
        for segment in result.split_whitespace() {
            if !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push_str(segment);
        }
        normalized
    }

    /// Extract the sender address from a parsed email.
    fn extract_sender(parsed: &mail_parser::Message) -> String {
        parsed
            .from()
            .and_then(|addr| addr.first())
            .and_then(|a| a.address())
            .map_or_else(|| "unknown".into(), ToString::to_string)
    }

    /// Extract readable text from a parsed email.
    fn extract_text(parsed: &mail_parser::Message) -> String {
        if let Some(text) = parsed.body_text(0) {
            return text.to_string();
        }
        if let Some(html) = parsed.body_html(0) {
            return Self::strip_html(html.as_ref());
        }
        for part in parsed.attachments() {
            let part: &mail_parser::MessagePart = part;
            if let Some(ct) = MimeHeaders::content_type(part)
                && ct.ctype() == "text"
                && let Ok(text) = std::str::from_utf8(part.contents())
            {
                let name = MimeHeaders::attachment_name(part).unwrap_or("file");
                return format!("[Attachment: {name}]\n{text}");
            }
        }
        "(no readable content)".to_string()
    }

    fn connect_imap_tls(config: &EmailConfig) -> Result<ImapTlsStream> {
        let tcp = TcpStream::connect((&*config.imap_host, config.imap_port))
            .context("connect to IMAP server")?;
        tcp.set_read_timeout(Some(Duration::from_secs(30)))
            .context("set IMAP read timeout")?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))
            .context("set IMAP write timeout")?;

        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls_config = Arc::new(
            TlsConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );
        let server_name: ServerName<'_> = ServerName::try_from(config.imap_host.clone())
            .context("parse IMAP server name for TLS")?;
        let conn = rustls::ClientConnection::new(tls_config, server_name)
            .context("create IMAP TLS connection")?;
        Ok(rustls::StreamOwned::new(conn, tcp))
    }

    fn read_imap_line(tls: &mut ImapTlsStream) -> Result<String> {
        let mut buf = Vec::new();
        loop {
            let mut byte = [0_u8; 1];
            match std::io::Read::read(tls, &mut byte) {
                Ok(0) => return Err(anyhow!("IMAP connection closed")),
                Ok(_) => {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\r\n") {
                        return Ok(String::from_utf8_lossy(&buf).to_string());
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn send_imap_command(tls: &mut ImapTlsStream, tag: &str, cmd: &str) -> Result<Vec<String>> {
        let full = format!("{tag} {cmd}\r\n");
        IoWrite::write_all(tls, full.as_bytes())?;
        IoWrite::flush(tls)?;
        let mut lines = Vec::new();
        loop {
            let line = Self::read_imap_line(tls)?;
            let done = line.starts_with(tag);
            lines.push(line);
            if done {
                break;
            }
        }
        Ok(lines)
    }

    fn login_and_select_folder(tls: &mut ImapTlsStream, config: &EmailConfig) -> Result<()> {
        let _greeting = Self::read_imap_line(tls).context("read IMAP server greeting")?;

        let login_resp = Self::send_imap_command(
            tls,
            "A1",
            &format!(
                "LOGIN \"{}\" \"{}\"",
                escape_imap_quoted(&config.username),
                escape_imap_quoted(&config.password)
            ),
        )
        .context("send IMAP login command")?;
        if !login_resp.last().is_some_and(|line| line.contains("OK")) {
            return Err(anyhow!("IMAP login failed"));
        }

        let _select = Self::send_imap_command(
            tls,
            "A2",
            &format!("SELECT \"{}\"", escape_imap_quoted(&config.imap_folder)),
        )
        .context("select IMAP folder")?;

        Ok(())
    }

    fn parse_imap_search_uids(search_resp: &[String]) -> Vec<String> {
        let mut uids = Vec::new();
        for line in search_resp {
            if line.starts_with("* SEARCH") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 2 {
                    uids.extend(parts[2..].iter().map(|uid| (*uid).to_string()));
                }
            }
        }
        uids
    }

    fn parsed_email_timestamp(parsed: &mail_parser::Message) -> u64 {
        // Cast safety: parsed email dates are expected to be Unix-epoch-era; invalid dates map to 0.
        #[allow(clippy::cast_sign_loss)]
        parsed.date().map_or_else(
            || {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0)
            },
            |date| {
                let naive = chrono::NaiveDate::from_ymd_opt(
                    i32::from(date.year),
                    u32::from(date.month),
                    u32::from(date.day),
                )
                .and_then(|day| {
                    day.and_hms_opt(
                        u32::from(date.hour),
                        u32::from(date.minute),
                        u32::from(date.second),
                    )
                });
                naive.map_or(0, |datetime| datetime.and_utc().timestamp() as u64)
            },
        )
    }

    fn fetch_imap_message(
        tls: &mut ImapTlsStream,
        uid: &str,
        fetch_tag: &str,
    ) -> Result<Option<(String, String, String, u64)>> {
        let fetch_resp = Self::send_imap_command(tls, fetch_tag, &format!("FETCH {uid} RFC822"))
            .context("fetch IMAP message body")?;
        let raw: String = fetch_resp
            .iter()
            .skip(1)
            .take(fetch_resp.len().saturating_sub(2))
            .cloned()
            .collect();

        let Some(parsed) = MessageParser::default().parse(raw.as_bytes()) else {
            return Ok(None);
        };

        let sender = Self::extract_sender(&parsed);
        let subject = parsed.subject().unwrap_or("(no subject)").to_string();
        let body = Self::extract_text(&parsed);
        let content = format!("Subject: {subject}\n\n{body}");
        let msg_id = parsed
            .message_id()
            .map_or_else(|| format!("gen-{}", Uuid::new_v4()), ToString::to_string);
        let timestamp = Self::parsed_email_timestamp(&parsed);

        Ok(Some((msg_id, sender, content, timestamp)))
    }

    /// Fetch unseen emails via `IMAP` (blocking, run in `spawn_blocking`).
    fn fetch_unseen_imap(config: &EmailConfig) -> Result<Vec<(String, String, String, u64)>> {
        let mut tls = Self::connect_imap_tls(config)?;
        Self::login_and_select_folder(&mut tls, config)?;

        let search_resp = Self::send_imap_command(&mut tls, "A3", "SEARCH UNSEEN")
            .context("search IMAP unseen messages")?;
        let uids = Self::parse_imap_search_uids(&search_resp);

        let mut results = Vec::new();
        let mut tag_counter = 4_u32;

        for uid in &uids {
            let fetch_tag = format!("A{tag_counter}");
            tag_counter += 1;
            if let Some(message) = Self::fetch_imap_message(&mut tls, uid, &fetch_tag)? {
                results.push(message);
            }

            let store_tag = format!("A{tag_counter}");
            tag_counter += 1;
            if let Err(error) = Self::send_imap_command(
                &mut tls,
                &store_tag,
                &format!("STORE {uid} +FLAGS (\\Seen)"),
            ) {
                tracing::warn!(error = %error, uid = %uid, "failed to mark IMAP message as seen");
            }
        }

        let logout_tag = format!("A{tag_counter}");
        if let Err(error) = Self::send_imap_command(&mut tls, &logout_tag, "LOGOUT") {
            tracing::warn!(error = %error, "failed to logout from IMAP session cleanly");
        }

        Ok(results)
    }

    fn create_smtp_transport(&self) -> Result<SmtpTransport> {
        let creds = Credentials::new(self.config.username.clone(), self.config.password.clone());
        let transport = if self.config.smtp_tls {
            SmtpTransport::relay(&self.config.smtp_host)
                .context("create SMTP relay connection")?
                .port(self.config.smtp_port)
                .credentials(creds)
                .build()
        } else {
            SmtpTransport::builder_dangerous(&self.config.smtp_host)
                .port(self.config.smtp_port)
                .credentials(creds)
                .build()
        };
        Ok(transport)
    }
}

impl Channel for EmailChannel {
    fn name(&self) -> &'static str {
        "email"
    }

    fn max_message_length(&self) -> usize {
        usize::MAX
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        recipient: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let (subject, body) = if message.starts_with("Subject: ") {
                if let Some(pos) = message.find('\n') {
                    (&message[9..pos], message[pos + 1..].trim())
                } else {
                    ("Asterel Message", message)
                }
            } else {
                ("Asterel Message", message)
            };

            let email = Message::builder()
                .from(
                    self.config
                        .from_address
                        .parse()
                        .context("parse email from address")?,
                )
                .to(recipient.parse().context("parse email recipient address")?)
                .subject(subject)
                .body(body.to_string())
                .context("build email message body")?;

            let transport = self.create_smtp_transport()?;
            transport.send(&email).context("send email via SMTP")?;
            info!("Email sent to {recipient}");
            Ok(())
        })
    }

    fn listen<'a>(
        &'a self,
        tx: mpsc::Sender<ChannelEvent>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            info!(
                "Email polling every {}s on {}",
                self.config.poll_interval_secs, self.config.imap_folder
            );
            let mut tick = interval(Duration::from_secs(self.config.poll_interval_secs));
            let config = Arc::new(self.config.clone());

            loop {
                tick.tick().await;
                let cfg = Arc::clone(&config);
                let imap_timeout =
                    Duration::from_secs(self.config.poll_interval_secs.saturating_mul(2).max(60));
                let fetch_future =
                    tokio::task::spawn_blocking(move || Self::fetch_unseen_imap(&cfg));
                match tokio::time::timeout(imap_timeout, fetch_future).await {
                    Err(_elapsed) => {
                        error!(
                            "Email IMAP fetch timed out after {}s",
                            imap_timeout.as_secs()
                        );
                        sleep(Duration::from_secs(10)).await;
                    }
                    Ok(Ok(Ok(messages))) => {
                        for (id, sender, content, ts) in messages {
                            if !self.is_sender_allowed(&sender) {
                                warn!("Blocked email from {sender}");
                                continue;
                            }

                            {
                                // Recover from mutex poisoning — prefer stale data over panic
                                let mut seen = self
                                    .seen_messages
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                if !seen.insert(id.clone()) {
                                    continue;
                                }
                                if seen.len() > 10_000 {
                                    seen.clear();
                                }
                            } // MutexGuard dropped before await

                            let msg = ChannelMessage {
                                id,
                                sender,
                                content,
                                channel: "email".to_string(),
                                context_hint: None,
                                conversation_id: None,
                                thread_id: None,
                                reply_to: None,
                                message_id: None,
                                timestamp: ts,
                                attachments: Vec::new(),
                            };
                            if tx.send(ChannelEvent::Message(msg)).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Ok(Ok(Err(e))) => {
                        error!("Email poll failed: {e}");
                        sleep(Duration::from_secs(10)).await;
                    }
                    Ok(Err(e)) => {
                        error!("Email poll task panicked: {e}");
                        sleep(Duration::from_secs(10)).await;
                    }
                }
            }
        })
    }

    fn health_check<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            let cfg = self.config.clone();
            tokio::task::spawn_blocking(move || {
                let tcp = TcpStream::connect((&*cfg.imap_host, cfg.imap_port));
                tcp.is_ok()
            })
            .await
            .unwrap_or_default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_email_channel(senders: Vec<String>) -> EmailChannel {
        EmailChannel::new(EmailConfig {
            allowed_senders: senders,
            ..EmailConfig::default()
        })
    }

    // ── is_sender_allowed tests ──────────────────────────────

    #[test]
    fn is_sender_allowed_denies_when_empty() {
        let ch = make_email_channel(vec![]);
        assert!(!ch.is_sender_allowed("anyone@example.com"));
    }

    #[test]
    fn is_sender_allowed_wildcard_allows_any() {
        let ch = make_email_channel(vec!["*".to_string()]);
        assert!(ch.is_sender_allowed("anyone@anywhere.net"));
    }

    #[test]
    fn is_sender_allowed_exact_match() {
        let ch = make_email_channel(vec!["alice@example.com".to_string()]);
        assert!(ch.is_sender_allowed("alice@example.com"));
        assert!(!ch.is_sender_allowed("bob@example.com"));
    }

    #[test]
    fn is_sender_allowed_domain_with_at_prefix() {
        let ch = make_email_channel(vec!["@example.com".to_string()]);
        assert!(ch.is_sender_allowed("alice@example.com"));
        assert!(!ch.is_sender_allowed("alice@other.com"));
    }

    #[test]
    fn is_sender_allowed_domain_without_at_prefix() {
        let ch = make_email_channel(vec!["example.com".to_string()]);
        assert!(ch.is_sender_allowed("alice@example.com"));
        assert!(!ch.is_sender_allowed("alice@other.com"));
    }

    #[test]
    fn is_sender_allowed_case_insensitive() {
        let ch = make_email_channel(vec!["Alice@Example.COM".to_string()]);
        assert!(ch.is_sender_allowed("alice@example.com"));
        assert!(ch.is_sender_allowed("ALICE@EXAMPLE.COM"));
    }

    // ── strip_html tests ─────────────────────────────────────

    #[test]
    fn strip_html_removes_tags() {
        assert_eq!(
            EmailChannel::strip_html("<p>Hello <b>world</b></p>"),
            "Hello world"
        );
    }

    #[test]
    fn strip_html_nested_tags() {
        assert_eq!(
            EmailChannel::strip_html("<div><span>text</span></div>"),
            "text"
        );
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        assert_eq!(
            EmailChannel::strip_html("<p>  hello   world  </p>"),
            "hello world"
        );
    }

    #[test]
    fn strip_html_empty_input() {
        assert_eq!(EmailChannel::strip_html(""), "");
    }

    #[test]
    fn strip_html_no_tags() {
        assert_eq!(EmailChannel::strip_html("plain text"), "plain text");
    }

    #[test]
    fn strip_html_preserves_entities() {
        assert_eq!(EmailChannel::strip_html("<p>&amp; &lt;</p>"), "&amp; &lt;");
    }
}
