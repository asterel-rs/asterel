//! CLI channel adapter: reads from stdin and writes to stdout.
//! Always available with zero external dependencies.
use std::future::Future;
use std::io::IsTerminal;
use std::pin::Pin;

use dialoguer::{BasicHistory, Input};
use tokio::io::{self, AsyncBufReadExt, BufReader};
use uuid::Uuid;

use super::traits::{Channel, ChannelEvent, ChannelMessage, SurfaceRealizationPolicy};

/// CLI channel — stdin/stdout, always available, zero deps
pub struct CliChannel;

#[derive(Debug, PartialEq, Eq)]
enum CliInputAction {
    Ignore,
    Message(String),
    Exit(String),
}

impl CliChannel {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CliChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel for CliChannel {
    fn name(&self) -> &'static str {
        "cli"
    }

    fn max_message_length(&self) -> usize {
        usize::MAX
    }

    fn surface_realization_policy(&self) -> SurfaceRealizationPolicy {
        SurfaceRealizationPolicy::cli()
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        _recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            println!("{message}");
            Ok(())
        })
    }

    fn listen<'a>(
        &'a self,
        tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if std::io::stdin().is_terminal() {
                listen_terminal_tty(tx).await
            } else {
                listen_non_tty(tx).await
            }
        })
    }
}

/// Forward normalized CLI lines to a plain string channel.
///
/// # Errors
///
/// Returns an error when the blocking terminal input task fails to join or
/// when stdin line reading returns an I/O error.
pub async fn listen_for_messages(tx: tokio::sync::mpsc::Sender<String>) -> anyhow::Result<()> {
    if std::io::stdin().is_terminal() {
        listen_terminal_messages_tty(tx).await
    } else {
        listen_non_tty_messages(tx).await
    }
}

async fn listen_terminal_tty(tx: tokio::sync::mpsc::Sender<ChannelEvent>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut history = BasicHistory::new().max_entries(256).no_duplicates(true);
        loop {
            let raw = Input::<String>::new()
                .with_prompt(">")
                .allow_empty(true)
                .history_with(&mut history)
                .interact_text()?;
            match classify_cli_line(&raw) {
                CliInputAction::Ignore => {}
                CliInputAction::Message(line) => {
                    if tx
                        .blocking_send(ChannelEvent::Message(build_cli_message(line)))
                        .is_err()
                    {
                        break;
                    }
                }
                CliInputAction::Exit(line) => {
                    let _ = tx.blocking_send(ChannelEvent::Message(build_cli_message(line)));
                    break;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| anyhow::anyhow!("cli input task join failure: {error}"))?
}

async fn listen_terminal_messages_tty(tx: tokio::sync::mpsc::Sender<String>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut history = BasicHistory::new().max_entries(256).no_duplicates(true);
        loop {
            let raw = Input::<String>::new()
                .with_prompt(">")
                .allow_empty(true)
                .history_with(&mut history)
                .interact_text()?;
            match classify_cli_line(&raw) {
                CliInputAction::Ignore => {}
                CliInputAction::Message(line) => {
                    if tx.blocking_send(line).is_err() {
                        break;
                    }
                }
                CliInputAction::Exit(line) => {
                    let _ = tx.blocking_send(line);
                    break;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| anyhow::anyhow!("cli input task join failure: {error}"))?
}

async fn listen_non_tty(tx: tokio::sync::mpsc::Sender<ChannelEvent>) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(raw)) = lines.next_line().await {
        match classify_cli_line(&raw) {
            CliInputAction::Ignore => {}
            CliInputAction::Message(line) => {
                if tx
                    .send(ChannelEvent::Message(build_cli_message(line)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            CliInputAction::Exit(line) => {
                let _ = tx
                    .send(ChannelEvent::Message(build_cli_message(line)))
                    .await;
                break;
            }
        }
    }
    Ok(())
}

async fn listen_non_tty_messages(tx: tokio::sync::mpsc::Sender<String>) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(raw)) = lines.next_line().await {
        match classify_cli_line(&raw) {
            CliInputAction::Ignore => {}
            CliInputAction::Message(line) => {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
            CliInputAction::Exit(line) => {
                let _ = tx.send(line).await;
                break;
            }
        }
    }
    Ok(())
}

fn normalize_cli_line(raw: &str) -> Option<String> {
    let line = raw.trim().to_string();
    if line.is_empty() { None } else { Some(line) }
}

fn is_exit_command(line: &str) -> bool {
    line == "/quit" || line == "/exit"
}

fn classify_cli_line(raw: &str) -> CliInputAction {
    let Some(line) = normalize_cli_line(raw) else {
        return CliInputAction::Ignore;
    };

    if is_exit_command(&line) {
        CliInputAction::Exit(line)
    } else {
        CliInputAction::Message(line)
    }
}

fn build_cli_message(content: String) -> ChannelMessage {
    ChannelMessage {
        id: Uuid::new_v4().to_string(),
        sender: "user".to_string(),
        content,
        channel: "cli".to_string(),
        context_hint: None,
        conversation_id: None,
        thread_id: None,
        reply_to: None,
        message_id: None,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        attachments: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::CliInputAction;
    use super::*;

    #[test]
    fn cli_channel_name() {
        assert_eq!(CliChannel::new().name(), "cli");
    }

    #[tokio::test]
    async fn cli_channel_send_does_not_panic() {
        let ch = CliChannel::new();
        let result = ch.send("hello", "user").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cli_channel_send_empty_message() {
        let ch = CliChannel::new();
        let result = ch.send("", "").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cli_channel_health_check() {
        let ch = CliChannel::new();
        assert!(ch.health_check().await);
    }

    #[test]
    fn channel_message_struct() {
        let msg = ChannelMessage {
            id: "test-id".into(),
            sender: "user".into(),
            content: "hello".into(),
            channel: "cli".into(),
            context_hint: None,
            conversation_id: None,
            thread_id: None,
            reply_to: None,
            message_id: None,
            timestamp: 1_234_567_890,
            attachments: Vec::new(),
        };
        assert_eq!(msg.id, "test-id");
        assert_eq!(msg.sender, "user");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "cli");
        assert_eq!(msg.timestamp, 1_234_567_890);
    }

    #[test]
    fn channel_message_clone() {
        let msg = ChannelMessage {
            id: "id".into(),
            sender: "s".into(),
            content: "c".into(),
            channel: "ch".into(),
            context_hint: None,
            conversation_id: None,
            thread_id: None,
            reply_to: None,
            message_id: None,
            timestamp: 0,
            attachments: Vec::new(),
        };
        let cloned = msg.clone();
        assert_eq!(cloned.id, msg.id);
        assert_eq!(cloned.content, msg.content);
    }

    #[test]
    fn normalize_cli_line_rejects_empty_lines() {
        assert_eq!(normalize_cli_line(""), None);
        assert_eq!(normalize_cli_line("   "), None);
        assert_eq!(normalize_cli_line("\n\t"), None);
    }

    #[test]
    fn normalize_cli_line_trims_and_keeps_content() {
        assert_eq!(
            normalize_cli_line("  hello world  "),
            Some("hello world".into())
        );
    }

    #[test]
    fn exit_command_detection_matches_supported_commands() {
        assert!(is_exit_command("/quit"));
        assert!(is_exit_command("/exit"));
        assert!(!is_exit_command("/plan"));
    }

    #[test]
    fn classify_cli_line_marks_exit_commands_for_downstream_handling() {
        assert_eq!(
            classify_cli_line("/quit"),
            CliInputAction::Exit("/quit".to_string())
        );
        assert_eq!(
            classify_cli_line(" /exit "),
            CliInputAction::Exit("/exit".to_string())
        );
    }

    #[test]
    fn classify_cli_line_marks_regular_messages() {
        assert_eq!(
            classify_cli_line(" hello "),
            CliInputAction::Message("hello".to_string())
        );
    }
}
