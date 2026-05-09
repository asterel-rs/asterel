//! Caption surface for the companion plugin.
//!
//! Manages speaker/assistant caption events with sequencing,
//! timeline rendering, and conversation context extraction.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Channel identifying who produced a caption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionCaptionChannel {
    /// Human speaker caption.
    Speaker,
    /// AI assistant caption.
    Assistant,
}

/// A single caption event in the companion surface timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionCaptionEvt {
    /// Unique caption identifier.
    pub caption_id: String,
    /// Speaker or assistant channel.
    pub channel: CompanionCaptionChannel,
    /// Monotonic sequence number.
    pub sequence: u64,
    /// Caption text content.
    pub text: String,
    /// RFC 3339 emission timestamp.
    pub emitted_at: String,
}

impl CompanionCaptionEvt {
    /// # Errors
    ///
    /// Returns an error when caption text is empty.
    pub fn new(
        channel: CompanionCaptionChannel,
        sequence: u64,
        text: impl Into<String>,
    ) -> Result<Self> {
        let text = text.into().trim().to_string();
        if text.is_empty() {
            anyhow::bail!("caption text must not be empty");
        }

        Ok(Self {
            caption_id: Uuid::new_v4().to_string(),
            channel,
            sequence,
            text,
            emitted_at: Utc::now().to_rfc3339(),
        })
    }
}

/// Action to perform on a companion widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAction {
    /// Create a new widget.
    Spawn,
    /// Update an existing widget's payload.
    Update,
    /// Remove a specific widget.
    Remove,
    /// Clear all active widgets.
    Clear,
    /// Open a URL in the companion surface.
    Open,
}

/// Command sent to the widget runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionWidgetCommand {
    /// Action to perform.
    pub action: CompanionAction,
    /// Target widget identifier (required for spawn/update/remove).
    #[serde(default)]
    pub widget_id: Option<String>,
    /// JSON payload for the widget.
    #[serde(default)]
    pub payload: Value,
    /// Time-to-live in seconds before auto-expiry.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
    /// URL for the open action.
    #[serde(default)]
    pub url: Option<String>,
}

/// Current state of a live companion widget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionWidgetState {
    /// Widget identifier.
    pub widget_id: String,
    /// Current JSON payload.
    pub payload: Value,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 last-update timestamp.
    pub updated_at: String,
    /// RFC 3339 expiry timestamp, if TTL was set.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Result returned after applying a widget command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompanionWidgetRuntimeResult {
    /// Action that was performed.
    pub action: CompanionAction,
    /// Widget affected by the action, if any.
    pub affected_widget_id: Option<String>,
    /// URL opened, if the action was `Open`.
    pub opened_url: Option<String>,
    /// Number of active widgets after the action.
    pub active_widgets: usize,
}

/// Runtime managing the lifecycle of companion widgets.
#[derive(Debug, Default)]
pub struct CompanionWidgetRuntime {
    widgets: HashMap<String, CompanionWidgetState>,
}

impl CompanionWidgetRuntime {
    /// Creates an empty widget runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// Returns an error when command constraints (widget id/url/payload) fail.
    pub fn apply(
        &mut self,
        command: CompanionWidgetCommand,
        now: DateTime<Utc>,
    ) -> Result<CompanionWidgetRuntimeResult> {
        match command.action {
            CompanionAction::Spawn => {
                let widget_id = require_widget_id(command.widget_id.as_deref())?;
                if !command.payload.is_object() {
                    anyhow::bail!("widget spawn payload must be a JSON object");
                }

                let expires_at = command
                    .ttl_secs
                    .map(|ttl| (now + duration_from_secs(ttl)).to_rfc3339());
                let created_at = now.to_rfc3339();
                let state = CompanionWidgetState {
                    widget_id: widget_id.clone(),
                    payload: command.payload,
                    created_at: created_at.clone(),
                    updated_at: created_at,
                    expires_at,
                };
                self.widgets.insert(widget_id.clone(), state);

                Ok(CompanionWidgetRuntimeResult {
                    action: CompanionAction::Spawn,
                    affected_widget_id: Some(widget_id),
                    opened_url: None,
                    active_widgets: self.widgets.len(),
                })
            }
            CompanionAction::Update => {
                let widget_id = require_widget_id(command.widget_id.as_deref())?;
                let Some(state) = self.widgets.get_mut(&widget_id) else {
                    anyhow::bail!("cannot update unknown widget '{widget_id}'");
                };
                if !command.payload.is_object() {
                    anyhow::bail!("widget update payload must be a JSON object");
                }
                state.payload = command.payload;
                state.updated_at = now.to_rfc3339();
                if let Some(ttl) = command.ttl_secs {
                    state.expires_at = Some((now + duration_from_secs(ttl)).to_rfc3339());
                }

                Ok(CompanionWidgetRuntimeResult {
                    action: CompanionAction::Update,
                    affected_widget_id: Some(widget_id),
                    opened_url: None,
                    active_widgets: self.widgets.len(),
                })
            }
            CompanionAction::Remove => {
                let widget_id = require_widget_id(command.widget_id.as_deref())?;
                self.widgets.remove(&widget_id);
                Ok(CompanionWidgetRuntimeResult {
                    action: CompanionAction::Remove,
                    affected_widget_id: Some(widget_id),
                    opened_url: None,
                    active_widgets: self.widgets.len(),
                })
            }
            CompanionAction::Clear => {
                self.widgets.clear();
                Ok(CompanionWidgetRuntimeResult {
                    action: CompanionAction::Clear,
                    affected_widget_id: None,
                    opened_url: None,
                    active_widgets: 0,
                })
            }
            CompanionAction::Open => {
                let Some(url) = command.url.as_deref().map(str::trim) else {
                    anyhow::bail!("widget open command requires 'url'");
                };
                validate_widget_open_url(url)?;

                Ok(CompanionWidgetRuntimeResult {
                    action: CompanionAction::Open,
                    affected_widget_id: command.widget_id,
                    opened_url: Some(url.to_string()),
                    active_widgets: self.widgets.len(),
                })
            }
        }
    }

    /// Removes expired widgets and returns their IDs.
    pub fn expire(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let mut expired_ids = Vec::new();
        self.widgets.retain(|widget_id, state| {
            let expired = state
                .expires_at
                .as_deref()
                .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
                .is_some_and(|expires_at| now > expires_at.with_timezone(&Utc));

            if expired {
                expired_ids.push(widget_id.clone());
                return false;
            }

            true
        });
        expired_ids
    }

    /// Returns a sorted snapshot of all active widget states.
    #[must_use]
    pub fn snapshot(&self) -> Vec<CompanionWidgetState> {
        let mut values = self.widgets.values().cloned().collect::<Vec<_>>();
        values.sort_by(|left, right| left.widget_id.cmp(&right.widget_id));
        values
    }
}

fn require_widget_id(widget_id: Option<&str>) -> Result<String> {
    let widget_id = widget_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("widget command requires non-empty 'widget_id'"))?;
    if !widget_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        anyhow::bail!("widget_id must use only [A-Za-z0-9._-]");
    }
    Ok(widget_id.to_string())
}

fn validate_widget_open_url(url: &str) -> Result<()> {
    if url.is_empty() {
        anyhow::bail!("widget open url must not be empty");
    }
    let parsed = url::Url::parse(url)
        .map_err(|error| anyhow::anyhow!("invalid widget open url: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("widget open url must use http or https");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!(crate::contracts::strings::verdicts::URL_USERINFO_NOT_ALLOWED);
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("widget open url must include a host"))?;
    if crate::contracts::network::is_private_host(host) || host.eq_ignore_ascii_case("localhost") {
        anyhow::bail!("widget open url must not target a local/private host");
    }
    Ok(())
}

/// State of a time-bounded confirmation request window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionRequestWindowState {
    /// Awaiting user confirmation.
    Pending,
    /// User confirmed the action.
    Confirmed,
    /// User cancelled the action.
    Cancelled,
    /// TTL elapsed without confirmation.
    Expired,
}

/// A time-bounded confirmation window for a sensitive action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionWindow {
    /// Unique window identifier.
    pub window_id: String,
    /// Description of the action awaiting confirmation.
    pub requested_action: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 expiry timestamp.
    pub expires_at: String,
    /// Current state of the window.
    pub state: CompanionRequestWindowState,
}

impl CompanionWindow {
    /// # Errors
    ///
    /// Returns an error when requested action is empty or ttl is zero.
    pub fn new(
        requested_action: impl Into<String>,
        now: DateTime<Utc>,
        ttl_secs: u64,
    ) -> Result<Self> {
        let requested_action = requested_action.into().trim().to_string();
        if requested_action.is_empty() {
            anyhow::bail!("request-window action must not be empty");
        }
        if ttl_secs == 0 {
            anyhow::bail!("request-window ttl_secs must be greater than zero");
        }

        Ok(Self {
            window_id: Uuid::new_v4().to_string(),
            requested_action,
            created_at: now.to_rfc3339(),
            expires_at: (now + duration_from_secs(ttl_secs)).to_rfc3339(),
            state: CompanionRequestWindowState::Pending,
        })
    }

    /// # Errors
    ///
    /// Returns an error when window is not pending or has already expired.
    pub fn confirm(&mut self, now: DateTime<Utc>) -> Result<()> {
        self.refresh_expiry(now)?;
        if self.state != CompanionRequestWindowState::Pending {
            anyhow::bail!("cannot confirm request-window in state {:?}", self.state);
        }
        self.state = CompanionRequestWindowState::Confirmed;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when window is not pending.
    pub fn cancel(&mut self) -> Result<()> {
        if self.state != CompanionRequestWindowState::Pending {
            anyhow::bail!("cannot cancel request-window in state {:?}", self.state);
        }
        self.state = CompanionRequestWindowState::Cancelled;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when expiry timestamp cannot be parsed.
    pub fn refresh_expiry(&mut self, now: DateTime<Utc>) -> Result<()> {
        if self.state != CompanionRequestWindowState::Pending {
            return Ok(());
        }

        let expires_at = DateTime::parse_from_rfc3339(&self.expires_at).map_err(|error| {
            anyhow::anyhow!("request-window expiry timestamp parse failed: {error}")
        })?;
        if now > expires_at.with_timezone(&Utc) {
            self.state = CompanionRequestWindowState::Expired;
        }
        Ok(())
    }
}

fn duration_from_secs(seconds: u64) -> Duration {
    Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use serde_json::json;

    use super::{
        CompanionAction, CompanionCaptionChannel, CompanionCaptionEvt, CompanionRequestWindowState,
        CompanionWidgetCommand, CompanionWidgetRuntime, CompanionWindow,
    };

    #[test]
    fn caption_event_rejects_empty_text() {
        let error = CompanionCaptionEvt::new(CompanionCaptionChannel::Assistant, 1, "   ")
            .expect_err("empty caption text must fail");
        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn widget_runtime_spawn_update_and_expire() {
        let now = Utc::now();
        let mut runtime = CompanionWidgetRuntime::new();

        runtime
            .apply(
                CompanionWidgetCommand {
                    action: CompanionAction::Spawn,
                    widget_id: Some("weather.panel".to_string()),
                    payload: json!({"title":"Weather"}),
                    ttl_secs: Some(2),
                    url: None,
                },
                now,
            )
            .unwrap();

        runtime
            .apply(
                CompanionWidgetCommand {
                    action: CompanionAction::Update,
                    widget_id: Some("weather.panel".to_string()),
                    payload: json!({"title":"Weather","state":"rain"}),
                    ttl_secs: Some(2),
                    url: None,
                },
                now + Duration::seconds(1),
            )
            .unwrap();
        assert_eq!(runtime.snapshot().len(), 1);

        let expired = runtime.expire(now + Duration::seconds(10));
        assert_eq!(expired, vec!["weather.panel".to_string()]);
        assert_eq!(runtime.snapshot().len(), 0);
    }

    #[test]
    fn widget_runtime_rejects_invalid_open_url() {
        let now = Utc::now();
        let mut runtime = CompanionWidgetRuntime::new();

        let error = runtime
            .apply(
                CompanionWidgetCommand {
                    action: CompanionAction::Open,
                    widget_id: None,
                    payload: json!({}),
                    ttl_secs: None,
                    url: Some("file:///tmp/secret".to_string()),
                },
                now,
            )
            .expect_err("open should reject non-http url");
        assert!(error.to_string().contains("must use http or https"));
    }

    #[test]
    fn widget_runtime_rejects_private_open_url() {
        let now = Utc::now();
        let mut runtime = CompanionWidgetRuntime::new();

        let error = runtime
            .apply(
                CompanionWidgetCommand {
                    action: CompanionAction::Open,
                    widget_id: None,
                    payload: json!({}),
                    ttl_secs: None,
                    url: Some("https://[fc00::1]/admin".to_string()),
                },
                now,
            )
            .expect_err("open should reject private IPv6 url");
        assert!(error.to_string().contains("local/private"));
    }

    #[test]
    fn request_window_confirms_within_ttl() {
        let now = Utc::now();
        let mut window = CompanionWindow::new("dangerous_action", now, 5).unwrap();
        window.confirm(now + Duration::seconds(2)).unwrap();
        assert_eq!(window.state, CompanionRequestWindowState::Confirmed);
    }

    #[test]
    fn request_window_expires_after_ttl() {
        let now = Utc::now();
        let mut window = CompanionWindow::new("dangerous_action", now, 2).unwrap();
        window.refresh_expiry(now + Duration::seconds(5)).unwrap();
        assert_eq!(window.state, CompanionRequestWindowState::Expired);
    }
}
