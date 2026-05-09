//! Ambient context tracking for the companion plugin.
//!
//! Manages page, video, subtitle, and vision-frame context items
//! with deduplication, TTL expiry, and content hashing.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::contracts::ids::{EventId, SessionId};

/// Kind of ambient context item tracked by the companion plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionContextKind {
    /// Web page snapshot.
    Page,
    /// Video stream context.
    Video,
    /// Subtitle text chunk from media.
    Subtitle,
    /// Vision frame captured from camera or screen.
    VisionFrame,
}

impl CompanionContextKind {
    /// Returns the `snake_case` string label for this context kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Video => "video",
            Self::Subtitle => "subtitle",
            Self::VisionFrame => "vision_frame",
        }
    }
}

/// A single ambient context event captured by the companion plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionCtxEvent {
    /// Unique identifier for this event.
    pub event_id: EventId,
    /// Session that produced the event.
    pub session_id: SessionId,
    /// Browser/application tab identifier.
    pub tab_id: String,
    /// Category of the context item.
    pub kind: CompanionContextKind,
    /// Descriptive topic label.
    pub topic: String,
    /// Origin system (e.g. `"extension"`).
    pub source: String,
    /// URL of the source page, if applicable.
    #[serde(default)]
    pub source_url: Option<String>,
    /// Media reference identifier, if applicable.
    #[serde(default)]
    pub media_ref: Option<String>,
    /// RFC 3339 timestamp of capture.
    pub captured_at: String,
    /// Arbitrary JSON payload attached to the event.
    #[serde(default)]
    pub payload: Value,
}

/// Input fields for constructing a [`CompanionCtxEvent`].
pub struct CompanionCtxInput {
    /// Session identifier.
    pub session_id: SessionId,
    /// Browser/application tab identifier.
    pub tab_id: String,
    /// Category of the context item.
    pub kind: CompanionContextKind,
    /// Descriptive topic label.
    pub topic: String,
    /// Origin system (e.g. `"extension"`).
    pub source: String,
    /// URL of the source page, if applicable.
    pub source_url: Option<String>,
    /// Media reference identifier, if applicable.
    pub media_ref: Option<String>,
    /// Arbitrary JSON payload.
    pub payload: Value,
}

impl CompanionCtxEvent {
    /// # Errors
    ///
    /// Returns an error when required context fields are missing or invalid.
    pub fn new(input: CompanionCtxInput) -> Result<Self> {
        let event = Self {
            event_id: EventId::new(Uuid::new_v4().to_string()),
            session_id: input.session_id,
            tab_id: input.tab_id,
            kind: input.kind,
            topic: input.topic,
            source: input.source,
            source_url: input.source_url,
            media_ref: input.media_ref,
            captured_at: Utc::now().to_rfc3339(),
            payload: input.payload,
        };
        event.validate_contract()?;
        Ok(event)
    }

    /// # Errors
    ///
    /// Returns an error when source attribution and kind-specific fields fail
    /// validation.
    pub fn validate_contract(&self) -> Result<()> {
        validate_segment("session_id", self.session_id.as_str())?;
        validate_segment("tab_id", &self.tab_id)?;
        validate_segment("topic", &self.topic)?;
        validate_segment("source", &self.source)?;

        if !self.payload.is_object() {
            anyhow::bail!("context payload must be a JSON object");
        }

        let source_url = normalize_optional_url(self.source_url.as_deref())?;
        let media_ref = normalize_optional_media_ref(self.media_ref.as_deref())?;

        match self.kind {
            CompanionContextKind::Page => {
                if source_url.is_none() {
                    anyhow::bail!("page context requires source_url");
                }
            }
            CompanionContextKind::Video => {
                if source_url.is_none() {
                    anyhow::bail!("video context requires source_url");
                }
                if media_ref.is_none() {
                    anyhow::bail!("video context requires media_ref");
                }
            }
            CompanionContextKind::Subtitle | CompanionContextKind::VisionFrame => {
                if media_ref.is_none() {
                    anyhow::bail!("subtitle/vision_frame context requires media_ref");
                }
            }
        }

        Ok(())
    }

    /// Computes a SHA-256 dedupe key from session, declared source, kind, topic,
    /// normalized URL, media ref, and payload.
    #[must_use]
    pub fn dedupe_key(&self) -> String {
        self.dedupe_key_for_producer(&self.source)
    }

    /// Computes a SHA-256 dedupe key from session, verified producer identity,
    /// kind, topic, normalized URL, media ref, and payload.
    #[must_use]
    pub fn dedupe_key_for_producer(&self, producer_identity: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.session_id.as_str().as_bytes());
        hasher.update(b"::");
        hasher.update(producer_identity.as_bytes());
        hasher.update(b"::");
        hasher.update(self.kind.as_str().as_bytes());
        hasher.update(b"::");
        hasher.update(self.topic.as_bytes());
        hasher.update(b"::");
        if let Ok(Some(source_url)) = normalize_optional_url(self.source_url.as_deref()) {
            hasher.update(source_url.as_bytes());
        }
        hasher.update(b"::");
        if let Ok(Some(media_ref)) = normalize_optional_media_ref(self.media_ref.as_deref()) {
            hasher.update(media_ref.as_bytes());
        }
        hasher.update(b"::");
        if let Ok(payload) = serde_json::to_vec(&self.payload) {
            hasher.update(payload.as_slice());
        }
        hex::encode(hasher.finalize())
    }
}

fn validate_segment(field: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        anyhow::bail!("{field} must use only [A-Za-z0-9._-]");
    }
    Ok(())
}

fn normalize_optional_url(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let mut parsed =
        url::Url::parse(raw).map_err(|error| anyhow::anyhow!("invalid source_url: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("source_url must use http or https");
    }
    parsed.set_fragment(None);
    Ok(Some(parsed.into()))
}

fn normalize_optional_media_ref(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if !raw
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/'))
    {
        anyhow::bail!("media_ref must use only [A-Za-z0-9._-:/]");
    }
    Ok(Some(raw.to_string()))
}

/// Reason the ingress gate accepted or suppressed a context event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionContextIngressReason {
    /// Event was accepted into the context store.
    Accepted,
    /// Event was suppressed as a duplicate within the window.
    DuplicateSuppressed,
}

/// Result of an ingress gate evaluation for a context event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionContextIngressDecision {
    /// Whether the event was accepted.
    pub accepted: bool,
    /// Reason for the decision.
    pub reason: CompanionContextIngressReason,
    /// Content-based dedupe key used for duplicate detection.
    pub dedupe_key: String,
}

/// Policy controlling duplicate detection window and capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionContextDedupePolicy {
    /// Seconds within which identical events are suppressed.
    pub duplicate_window_secs: u64,
    /// Maximum number of tracked dedupe entries.
    pub max_entries: usize,
}

impl Default for CompanionContextDedupePolicy {
    fn default() -> Self {
        Self {
            duplicate_window_secs: 15,
            max_entries: 4096,
        }
    }
}

/// Stateful ingress gate that deduplicates context events.
#[derive(Debug)]
pub struct CompanionContextIngressGate {
    policy: CompanionContextDedupePolicy,
    seen_at: HashMap<String, DateTime<Utc>>,
}

impl CompanionContextIngressGate {
    /// Creates a new ingress gate with the given deduplication policy.
    #[must_use]
    pub fn new(policy: CompanionContextDedupePolicy) -> Self {
        Self {
            policy: CompanionContextDedupePolicy {
                duplicate_window_secs: policy.duplicate_window_secs.max(1),
                max_entries: policy.max_entries.max(1),
            },
            seen_at: HashMap::new(),
        }
    }

    /// # Errors
    ///
    /// Returns an error when the context event contract is invalid.
    pub fn ingest(
        &mut self,
        event: &CompanionCtxEvent,
        now: DateTime<Utc>,
    ) -> Result<CompanionContextIngressDecision> {
        self.ingest_for_producer(event, &event.source, now)
    }

    /// # Errors
    ///
    /// Returns an error when the context event contract is invalid.
    pub fn ingest_for_producer(
        &mut self,
        event: &CompanionCtxEvent,
        producer_identity: &str,
        now: DateTime<Utc>,
    ) -> Result<CompanionContextIngressDecision> {
        event.validate_contract()?;
        self.prune_expired(now);

        let dedupe_key = event.dedupe_key_for_producer(producer_identity);
        let duplicate_window = duration_from_secs(self.policy.duplicate_window_secs);

        if let Some(last_seen_at) = self.seen_at.get(&dedupe_key) {
            let elapsed = now.signed_duration_since(*last_seen_at);
            if elapsed < Duration::zero() || elapsed <= duplicate_window {
                self.record_seen(dedupe_key.clone(), now);
                return Ok(CompanionContextIngressDecision {
                    accepted: false,
                    reason: CompanionContextIngressReason::DuplicateSuppressed,
                    dedupe_key,
                });
            }
        }

        self.record_seen(dedupe_key.clone(), now);
        self.prune_capacity();

        Ok(CompanionContextIngressDecision {
            accepted: true,
            reason: CompanionContextIngressReason::Accepted,
            dedupe_key,
        })
    }

    /// Returns the number of currently tracked dedupe entries.
    #[must_use]
    pub fn tracked_entries(&self) -> usize {
        self.seen_at.len()
    }

    /// Forget a previously recorded dedupe key.
    ///
    /// This is used as a rollback hook when downstream durable ingestion fails.
    pub fn forget(&mut self, dedupe_key: &str) {
        self.seen_at.remove(dedupe_key);
    }

    fn prune_expired(&mut self, now: DateTime<Utc>) {
        let duplicate_window = duration_from_secs(self.policy.duplicate_window_secs);
        self.seen_at
            .retain(|_, last_seen_at| now.signed_duration_since(*last_seen_at) <= duplicate_window);
    }

    fn prune_capacity(&mut self) {
        if self.seen_at.len() <= self.policy.max_entries {
            return;
        }

        let overflow = self.seen_at.len().saturating_sub(self.policy.max_entries);
        let mut by_age = self
            .seen_at
            .iter()
            .map(|(key, seen_at)| (key.clone(), *seen_at))
            .collect::<Vec<_>>();
        by_age.sort_by_key(|(_, seen_at)| *seen_at);

        for (old_key, _) in by_age.into_iter().take(overflow) {
            self.seen_at.remove(&old_key);
        }
    }

    fn record_seen(&mut self, dedupe_key: String, now: DateTime<Utc>) {
        self.seen_at.insert(dedupe_key, now);
    }
}

impl Default for CompanionContextIngressGate {
    fn default() -> Self {
        Self::new(CompanionContextDedupePolicy::default())
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
        CompanionContextDedupePolicy, CompanionContextIngressGate, CompanionContextIngressReason,
        CompanionContextKind, CompanionCtxEvent, CompanionCtxInput,
    };
    use crate::contracts::ids::SessionId;

    #[test]
    fn page_context_requires_source_url() {
        let error = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_1"),
            tab_id: "tab_1".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            source_url: None,
            media_ref: None,
            payload: json!({"title":"Example"}),
        })
        .expect_err("page context without source_url must fail");
        assert!(error.to_string().contains("requires source_url"));
    }

    #[test]
    fn context_rejects_invalid_source_url_scheme() {
        let error = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_1"),
            tab_id: "tab_1".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            source_url: Some("file:///tmp/secret".to_string()),
            media_ref: None,
            payload: json!({"title":"Secret"}),
        })
        .expect_err("non-http source url must fail");
        assert!(error.to_string().contains("must use http or https"));
    }

    #[test]
    fn dedupe_suppresses_duplicate_context_across_tabs() {
        let now = Utc::now();
        let mut gate = CompanionContextIngressGate::default();

        let first = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_1"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            source_url: Some("https://example.com/news".to_string()),
            media_ref: None,
            payload: json!({"title":"News"}),
        })
        .unwrap();

        let second = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_1"),
            tab_id: "tab_b".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            source_url: Some("https://example.com/news".to_string()),
            media_ref: None,
            payload: json!({"title":"News"}),
        })
        .unwrap();

        let accepted = gate.ingest(&first, now).unwrap();
        assert!(accepted.accepted);
        assert_eq!(accepted.reason, CompanionContextIngressReason::Accepted);

        let duplicate = gate.ingest(&second, now + Duration::seconds(1)).unwrap();
        assert!(!duplicate.accepted);
        assert_eq!(
            duplicate.reason,
            CompanionContextIngressReason::DuplicateSuppressed
        );
        assert_eq!(gate.tracked_entries(), 1);
    }

    #[test]
    fn dedupe_partitions_same_context_by_source() {
        let now = Utc::now();
        let mut gate = CompanionContextIngressGate::default();

        let first = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_source"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "producer_a".to_string(),
            source_url: Some("https://example.com/news".to_string()),
            media_ref: None,
            payload: json!({"title":"News"}),
        })
        .unwrap();

        let second = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_source"),
            tab_id: "tab_b".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "producer_b".to_string(),
            source_url: Some("https://example.com/news".to_string()),
            media_ref: None,
            payload: json!({"title":"News"}),
        })
        .unwrap();

        assert!(gate.ingest(&first, now).unwrap().accepted);
        assert!(
            gate.ingest(&second, now + Duration::seconds(1))
                .unwrap()
                .accepted
        );
        assert_eq!(gate.tracked_entries(), 2);
    }

    #[test]
    fn dedupe_allows_same_context_after_window() {
        let now = Utc::now();
        let mut gate = CompanionContextIngressGate::new(CompanionContextDedupePolicy {
            duplicate_window_secs: 2,
            max_entries: 64,
        });

        let event = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_2"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Subtitle,
            topic: "subtitle_chunk".to_string(),
            source: "extension".to_string(),
            source_url: None,
            media_ref: Some("video:clip-1".to_string()),
            payload: json!({"text":"hello"}),
        })
        .unwrap();

        let accepted = gate.ingest(&event, now).unwrap();
        assert!(accepted.accepted);

        let accepted_again = gate.ingest(&event, now + Duration::seconds(5)).unwrap();
        assert!(accepted_again.accepted);
        assert_eq!(
            accepted_again.reason,
            CompanionContextIngressReason::Accepted
        );
    }

    #[test]
    fn dedupe_capacity_evicts_oldest_key() {
        let now = Utc::now();
        let mut gate = CompanionContextIngressGate::new(CompanionContextDedupePolicy {
            duplicate_window_secs: 60,
            max_entries: 1,
        });

        let first = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_3"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Subtitle,
            topic: "subtitle_chunk".to_string(),
            source: "extension".to_string(),
            source_url: None,
            media_ref: Some("video:clip-1".to_string()),
            payload: json!({"text":"first"}),
        })
        .unwrap();

        let second = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_3"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Subtitle,
            topic: "subtitle_chunk".to_string(),
            source: "extension".to_string(),
            source_url: None,
            media_ref: Some("video:clip-2".to_string()),
            payload: json!({"text":"second"}),
        })
        .unwrap();

        assert!(gate.ingest(&first, now).unwrap().accepted);
        assert!(
            gate.ingest(&second, now + Duration::seconds(1))
                .unwrap()
                .accepted
        );
        assert_eq!(gate.tracked_entries(), 1);

        let reaccepted = gate.ingest(&first, now + Duration::seconds(2)).unwrap();
        assert!(reaccepted.accepted);
    }

    #[test]
    fn dedupe_uses_normalized_source_url() {
        let now = Utc::now();
        let mut gate = CompanionContextIngressGate::default();

        let first = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_4"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            source_url: Some("https://Example.com/news#top".to_string()),
            media_ref: None,
            payload: json!({"title":"News"}),
        })
        .unwrap();

        let second = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_4"),
            tab_id: "tab_b".to_string(),
            kind: CompanionContextKind::Page,
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            source_url: Some(" https://example.com/news ".to_string()),
            media_ref: None,
            payload: json!({"title":"News"}),
        })
        .unwrap();

        assert!(gate.ingest(&first, now).unwrap().accepted);
        let duplicate = gate.ingest(&second, now + Duration::seconds(1)).unwrap();
        assert!(!duplicate.accepted);
        assert_eq!(
            duplicate.reason,
            CompanionContextIngressReason::DuplicateSuppressed
        );
    }

    #[test]
    fn forget_allows_retry_within_duplicate_window() {
        let now = Utc::now();
        let mut gate = CompanionContextIngressGate::default();
        let event = CompanionCtxEvent::new(CompanionCtxInput {
            session_id: SessionId::new("session_5"),
            tab_id: "tab_a".to_string(),
            kind: CompanionContextKind::Subtitle,
            topic: "subtitle_chunk".to_string(),
            source: "extension".to_string(),
            source_url: None,
            media_ref: Some("video:clip-5".to_string()),
            payload: json!({"text":"hello"}),
        })
        .unwrap();

        let accepted = gate.ingest(&event, now).unwrap();
        assert!(accepted.accepted);
        gate.forget(&accepted.dedupe_key);
        let retried = gate.ingest(&event, now + Duration::seconds(1)).unwrap();
        assert!(retried.accepted);
    }
}
