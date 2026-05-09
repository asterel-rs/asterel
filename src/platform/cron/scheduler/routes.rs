//! Specialized cron route handlers for data ingestion.
//!
//! Executes RSS polling, X/Twitter trend aggregation, channel
//! message dispatch, and API ingestion pipeline routes as
//! scheduled cron jobs.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::sync::{Arc, LazyLock, Mutex};

use chrono::Utc;
use serde::Deserialize;

use super::{
    INGEST_API_MIN_INTERVAL_SECONDS, INGEST_RSS_MIN_INTERVAL_SECONDS, ROUTE_MARKER_CHANNEL_SEND,
    ROUTE_MARKER_INGEST_PIPELINE, ROUTE_MARKER_INVALID_ROUTE, ROUTE_MARKER_RSS_POLL,
    ROUTE_MARKER_TREND_AGGREGATION, ROUTE_MARKER_X_POLL, TREND_AGGREGATION_LIMIT,
    TREND_AGGREGATION_TOP_ITEMS, X_RECENT_SEARCH_ENDPOINT,
};
use crate::config::Config;
use crate::contracts::ids::{ChannelId, EntityId};
use crate::contracts::strings::data_model::PREFIX_EXTERNAL;
use crate::core::memory::{
    DefaultIngestPipeline, IngestionPipeline, MemoryEventInput, MemoryEventType, MemoryLayer,
    MemoryProvenance, MemorySource, PrivacyLevel, RecallQuery, SignalEnvelope, SourceKind,
    create_memory,
};
use crate::runtime::observability::create_observer;
use crate::security::{SecurityPolicy, validate_fetch_url};

static INGEST_SOURCE_LAST_SEEN: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static CHANNEL_SEND_LAST_SEEN: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const CHANNEL_SEND_MIN_INTERVAL_SECONDS: i64 = 2;

#[derive(Debug, Deserialize)]
struct XRecentSearchResponse {
    #[serde(default)]
    data: Vec<XRecentTweet>,
}

/// A single tweet from the X/Twitter recent search API.
#[derive(Debug, Deserialize)]
pub(super) struct XRecentTweet {
    /// Tweet identifier.
    pub(super) id: String,
    /// Tweet body text.
    pub(super) text: String,
    /// ISO 639-1 language code, if available.
    #[serde(default)]
    pub(super) lang: Option<String>,
    /// Author user identifier, if available.
    #[serde(default)]
    pub(super) author_id: Option<String>,
}

/// Parsed parameters for an ingestion pipeline cron job.
#[derive(Debug, Clone)]
pub(super) struct ParsedIngestionJob {
    /// Kind of data source (API, RSS, etc.).
    pub(super) source_kind: SourceKind,
    /// Unique entity identifier for deduplication.
    pub(super) entity_id: EntityId,
    /// Source reference string (URL or key).
    pub(super) source_ref: String,
    /// Raw content to ingest.
    pub(super) content: String,
}

/// Parsed parameters for a trend aggregation cron job.
#[derive(Debug, Clone)]
pub(super) struct ParsedTrendAggregationJob {
    /// Unique entity identifier for deduplication.
    pub(super) entity_id: EntityId,
    /// Normalized topic key for grouping.
    pub(super) topic_key: String,
    /// Search query string.
    pub(super) query: String,
}

/// Parsed parameters for an X/Twitter poll cron job.
#[derive(Debug, Clone)]
pub(super) struct ParsedXPollJob {
    /// Unique entity identifier for deduplication.
    pub(super) entity_id: EntityId,
    /// Search query for recent tweets.
    pub(super) query: String,
}

/// Parsed parameters for an RSS poll cron job.
#[derive(Debug, Clone)]
pub(super) struct ParsedRssPollJob {
    /// Unique entity identifier for deduplication.
    pub(super) entity_id: EntityId,
    /// RSS feed URL.
    pub(super) url: String,
}

/// A cron command parsed into a specialized route handler.
#[derive(Debug, Clone)]
pub(super) enum ParsedRoutedJob {
    Ingestion(ParsedIngestionJob),
    TrendAggregation(ParsedTrendAggregationJob),
    XPoll(ParsedXPollJob),
    RssPoll(ParsedRssPollJob),
    ChannelSend {
        channel_name: String,
        channel_id: ChannelId,
        message: String,
    },
}

fn has_reserved_route_prefix(trimmed: &str) -> bool {
    trimmed == "channel-send"
        || trimmed.starts_with("channel-send ")
        || trimmed.starts_with("ingest:")
}

pub(super) fn malformed_routed_job_output(command: &str) -> Option<String> {
    let trimmed = command.trim();
    has_reserved_route_prefix(trimmed).then(|| {
        format!(
            "{ROUTE_MARKER_INVALID_ROUTE}\ninvalid routed cron command; malformed reserved route did not run as shell"
        )
    })
}

/// A single item extracted from an RSS feed.
#[derive(Debug, Clone)]
pub(super) struct RssPollItem {
    /// Link or GUID identifying this item.
    pub(super) source_ref: String,
    /// Aggregated title + description text.
    pub(super) content: String,
}

fn normalize_trend_topic_key(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_dot = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch.to_ascii_lowercase());
            last_dot = false;
        } else if !last_dot {
            out.push('.');
            last_dot = true;
        }
    }
    out.trim_matches('.').to_string()
}

const fn ingestion_min_interval_seconds(source_kind: SourceKind) -> i64 {
    match source_kind {
        SourceKind::News => INGEST_RSS_MIN_INTERVAL_SECONDS,
        SourceKind::Api
        | SourceKind::Conversation
        | SourceKind::Discord
        | SourceKind::Telegram
        | SourceKind::Slack
        | SourceKind::Document
        | SourceKind::Manual => INGEST_API_MIN_INTERVAL_SECONDS,
    }
}

/// Maximum number of entries retained in the ingestion rate-limit tracker.
/// When the tracker exceeds this size, entries older than the largest
/// per-source interval are evicted to prevent unbounded memory growth during
/// long-running processes.
const INGEST_RATE_LIMIT_TRACKER_MAX_ENTRIES: usize = 10_000;

fn ingestion_rate_limit_key(job: &ParsedIngestionJob) -> String {
    format!("{}:{}", job.source_kind, job.source_ref)
}

fn check_and_record_ingestion_rate_limit(job: &ParsedIngestionJob) -> anyhow::Result<Option<i64>> {
    let key = ingestion_rate_limit_key(job);
    let now = Utc::now().timestamp();
    let interval = ingestion_min_interval_seconds(job.source_kind);

    let mut tracker = INGEST_SOURCE_LAST_SEEN
        .lock()
        .map_err(|e| anyhow::anyhow!("ingestion rate-limit tracker lock poisoned: {e}"))?;

    // Evict stale entries when the tracker grows beyond the cap to prevent
    // unbounded memory growth in long-running processes.
    if tracker.len() > INGEST_RATE_LIMIT_TRACKER_MAX_ENTRIES {
        let max_interval = INGEST_API_MIN_INTERVAL_SECONDS.max(INGEST_RSS_MIN_INTERVAL_SECONDS);
        tracker.retain(|_, ts| now.saturating_sub(*ts) < max_interval * 2);
    }

    let Some(previous) = tracker.get(&key).copied() else {
        tracker.insert(key, now);
        return Ok(None);
    };

    let elapsed = now.saturating_sub(previous);
    if elapsed >= interval {
        tracker.insert(key, now);
        return Ok(None);
    }

    Ok(Some(interval - elapsed))
}

fn rollback_ingestion_rate_limit(job: &ParsedIngestionJob) {
    if let Ok(mut tracker) = INGEST_SOURCE_LAST_SEEN.lock() {
        tracker.remove(&ingestion_rate_limit_key(job));
    }
}

fn rollback_channel_send_rate_limit(channel_id: &ChannelId) {
    if let Ok(mut tracker) = CHANNEL_SEND_LAST_SEEN.lock() {
        tracker.remove(channel_id.as_str());
    }
}

fn consume_security_or_output(
    security: &SecurityPolicy,
    route_marker: &str,
) -> Option<(bool, String)> {
    security
        .consume_action_cost(0)
        .err()
        .map(|policy_error| (false, format!("{route_marker}\n{policy_error}")))
}

async fn create_memory_or_output(
    config: &Config,
    route_marker: &str,
) -> Result<Box<dyn crate::core::memory::Memory>, (bool, String)> {
    create_memory(&config.memory, &config.workspace_dir, None)
        .await
        .map_err(|error| {
            (
                false,
                format!("{route_marker}\ncreate_memory failed: {error}"),
            )
        })
}

async fn create_ingestion_pipeline_or_output(
    config: &Config,
    route_marker: &str,
) -> Result<DefaultIngestPipeline, (bool, String)> {
    let memory = create_memory_or_output(config, route_marker).await?;
    let observer: Arc<dyn crate::runtime::observability::Observer> =
        Arc::from(create_observer(&config.observability));
    Ok(DefaultIngestPipeline::new_with_observer(
        Arc::from(memory),
        observer,
    ))
}

/// Parses a cron command string into a specialized route, or
/// returns `None` for plain shell commands.
pub(super) fn parse_routed_job_command(command: &str) -> Option<ParsedRoutedJob> {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix("channel-send ") {
        let mut parts = rest.splitn(3, ' ');
        let channel_name = parts.next()?.trim();
        let channel_id = parts.next()?.trim();
        let message = parts.next()?.trim();
        if channel_name.is_empty() || channel_id.is_empty() || message.is_empty() {
            return None;
        }

        return Some(ParsedRoutedJob::ChannelSend {
            channel_name: channel_name.to_string(),
            channel_id: ChannelId::new(channel_id),
            message: message.to_string(),
        });
    }

    let (source_kind, rest, source_ref_prefix) =
        if let Some(rest) = trimmed.strip_prefix("ingest:api ") {
            (Some(SourceKind::Api), rest, "")
        } else if let Some(rest) = trimmed.strip_prefix("ingest:x ") {
            (Some(SourceKind::Api), rest, "x:")
        } else if let Some(rest) = trimmed.strip_prefix("ingest:x-poll ") {
            (None, rest, "x-poll")
        } else if let Some(rest) = trimmed.strip_prefix("ingest:rss-poll ") {
            (None, rest, "rss-poll")
        } else if let Some(rest) = trimmed.strip_prefix("ingest:rss ") {
            (Some(SourceKind::News), rest, "")
        } else if let Some(rest) = trimmed.strip_prefix("ingest:trend ") {
            (None, rest, "")
        } else {
            return None;
        };

    if source_ref_prefix == "x-poll" {
        let mut parts = rest.splitn(2, ' ');
        let entity_id = parts.next()?.trim();
        let query = parts.next()?.trim();
        if entity_id.is_empty() || query.is_empty() {
            return None;
        }

        return Some(ParsedRoutedJob::XPoll(ParsedXPollJob {
            entity_id: EntityId::new(entity_id),
            query: query.to_string(),
        }));
    }

    if source_ref_prefix == "rss-poll" {
        let mut parts = rest.splitn(2, ' ');
        let entity_id = parts.next()?.trim();
        let url = parts.next()?.trim();
        if entity_id.is_empty() || url.is_empty() {
            return None;
        }

        return Some(ParsedRoutedJob::RssPoll(ParsedRssPollJob {
            entity_id: EntityId::new(entity_id),
            url: url.to_string(),
        }));
    }

    if let Some(source_kind) = source_kind {
        let mut parts = rest.splitn(3, ' ');
        let entity_id = parts.next()?.trim();
        let source_ref = parts.next()?.trim();
        let content = parts.next()?.trim();
        if entity_id.is_empty() || source_ref.is_empty() || content.is_empty() {
            return None;
        }

        return Some(ParsedRoutedJob::Ingestion(ParsedIngestionJob {
            source_kind,
            entity_id: EntityId::new(entity_id),
            source_ref: format!("{source_ref_prefix}{source_ref}"),
            content: content.to_string(),
        }));
    }

    let mut parts = rest.splitn(3, ' ');
    let entity_id = parts.next()?.trim();
    let topic_key = normalize_trend_topic_key(parts.next()?.trim());
    let query = parts.next()?.trim();
    if entity_id.is_empty() || topic_key.is_empty() || query.is_empty() {
        return None;
    }

    Some(ParsedRoutedJob::TrendAggregation(
        ParsedTrendAggregationJob {
            entity_id: EntityId::new(entity_id),
            topic_key,
            query: query.to_string(),
        },
    ))
}

/// Extracts the X/Twitter bearer token or returns an error
/// message.
pub(super) fn resolve_x_bearer_token(raw: Option<String>) -> Result<String, String> {
    match raw {
        Some(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        _ => Err(format!("{ROUTE_MARKER_X_POLL}\nmissing X_BEARER_TOKEN")),
    }
}

/// Returns the X recent search endpoint, falling back to the
/// default if none is configured.
pub(super) fn resolve_x_recent_search_endpoint(raw: Option<String>) -> String {
    match raw {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => X_RECENT_SEARCH_ENDPOINT.to_string(),
    }
}

/// Converts X/Twitter search results into memory signal
/// envelopes.
pub(super) fn build_x_poll_envelopes(
    entity_id: &str,
    query: &str,
    tweets: Vec<XRecentTweet>,
) -> Vec<SignalEnvelope> {
    tweets
        .into_iter()
        .map(|tweet| {
            let mut envelope = SignalEnvelope::new(
                SourceKind::Api,
                format!("x:{}", tweet.id),
                tweet.text,
                entity_id,
            )
            .with_privacy_level(PrivacyLevel::Private)
            .with_metadata("x_query", query.to_string());

            if let Some(author_id) = tweet.author_id {
                envelope = envelope.with_metadata("x_author_id", author_id);
            }
            if let Some(lang) = tweet.lang {
                envelope = envelope.with_language(lang);
            }
            envelope
        })
        .collect::<Vec<_>>()
}

/// Extracts up to `limit` items from raw RSS XML.
pub(super) fn parse_rss_items_from_xml(xml: &str, limit: usize) -> Vec<RssPollItem> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    if limit == 0 {
        return Vec::new();
    }

    let mut reader = Reader::from_str(xml);
    let mut items = Vec::new();

    let mut in_item = false;
    let mut current_tag = String::new();
    let mut title = String::new();
    let mut description = String::new();
    let mut guid: Option<String> = None;
    let mut link: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == b"item" {
                    in_item = true;
                    title.clear();
                    description.clear();
                    guid = None;
                    link = None;
                } else if in_item {
                    current_tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                }
            }
            Ok(Event::Text(e)) if in_item => {
                let text = String::from_utf8_lossy(e.as_ref());
                match current_tag.as_str() {
                    "title" => title.push_str(&text),
                    "description" => description.push_str(&text),
                    "guid" => guid.get_or_insert_with(String::new).push_str(&text),
                    "link" => link.get_or_insert_with(String::new).push_str(&text),
                    _ => {}
                }
            }
            Ok(Event::CData(e)) if in_item => {
                let text = String::from_utf8_lossy(e.as_ref());
                match current_tag.as_str() {
                    "title" => title.push_str(&text),
                    "description" => description.push_str(&text),
                    "guid" => guid.get_or_insert_with(String::new).push_str(&text),
                    "link" => link.get_or_insert_with(String::new).push_str(&text),
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == b"item" {
                    in_item = false;
                    current_tag.clear();

                    let id = guid
                        .take()
                        .or(link.take())
                        .unwrap_or_else(|| format!("rss-item-{}", items.len() + 1));
                    let title_trimmed = title.trim();
                    let desc_trimmed = description.trim();
                    let content = if !title_trimmed.is_empty() && !desc_trimmed.is_empty() {
                        format!("{title_trimmed} - {desc_trimmed}")
                    } else if !title_trimmed.is_empty() {
                        title_trimmed.to_string()
                    } else {
                        desc_trimmed.to_string()
                    };

                    if !content.trim().is_empty() {
                        items.push(RssPollItem {
                            source_ref: format!("rss:{id}"),
                            content,
                        });
                        if items.len() >= limit {
                            break;
                        }
                    }
                } else if in_item {
                    current_tag.clear();
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                tracing::warn!(%error, "RSS XML parse error; returning partial items");
                break;
            }
            _ => {}
        }
    }

    items
}

/// Converts parsed RSS items into memory signal envelopes.
pub(super) fn build_rss_poll_envelopes(
    entity_id: &str,
    items: Vec<RssPollItem>,
) -> Vec<SignalEnvelope> {
    items
        .into_iter()
        .map(|item| {
            SignalEnvelope::new(SourceKind::News, item.source_ref, item.content, entity_id)
                .with_privacy_level(PrivacyLevel::Private)
        })
        .collect::<Vec<_>>()
}

/// Runs an ingestion pipeline cron job.
pub(super) async fn run_ingestion_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: ParsedIngestionJob,
) -> (bool, String) {
    match check_and_record_ingestion_rate_limit(&job) {
        Ok(Some(wait_seconds)) => {
            return (
                false,
                format!(
                    "{ROUTE_MARKER_INGEST_PIPELINE}\naccepted=false\nreason=rate_limited\nwait_seconds={wait_seconds}"
                ),
            );
        }
        Ok(None) => {}
        Err(error) => {
            return (
                false,
                format!("{ROUTE_MARKER_INGEST_PIPELINE}\nrate limiter failed: {error}"),
            );
        }
    }

    if let Some(output) = consume_security_or_output(security, ROUTE_MARKER_INGEST_PIPELINE) {
        rollback_ingestion_rate_limit(&job);
        return output;
    }

    let pipeline =
        match create_ingestion_pipeline_or_output(config, ROUTE_MARKER_INGEST_PIPELINE).await {
            Ok(pipeline) => pipeline,
            Err(output) => {
                rollback_ingestion_rate_limit(&job);
                return output;
            }
        };
    let envelope = SignalEnvelope::new(
        job.source_kind,
        &job.source_ref,
        &job.content,
        job.entity_id.as_str(),
    )
    .with_privacy_level(PrivacyLevel::Private);

    match pipeline.ingest(envelope).await {
        Ok(result) => (
            result.accepted,
            format!(
                "{ROUTE_MARKER_INGEST_PIPELINE}\naccepted={}\nslot_key={}\nreason={}",
                result.accepted,
                result.slot_key,
                result.reason.unwrap_or_else(|| "none".to_string())
            ),
        ),
        Err(error) => {
            rollback_ingestion_rate_limit(&job);
            (
                false,
                format!("{ROUTE_MARKER_INGEST_PIPELINE}\ningestion failed: {error}"),
            )
        }
    }
}

/// Runs a scheduled channel-send cron job.
pub(super) async fn run_channel_send_job_command(
    config: &Config,
    security: &SecurityPolicy,
    command: &str,
) -> (bool, String) {
    let parsed = match parse_routed_job_command(command) {
        Some(ParsedRoutedJob::ChannelSend {
            channel_name,
            channel_id,
            message,
        }) => (channel_name, channel_id, message),
        _ => {
            return (
                false,
                format!(
                    "{ROUTE_MARKER_CHANNEL_SEND}\ninvalid channel-send format: expected 'channel-send <channel_name> <channel_id> <message>'"
                ),
            );
        }
    };

    let (channel_name, channel_id, message) = parsed;
    if !channel_name.eq_ignore_ascii_case("discord") {
        return (
            false,
            format!("{ROUTE_MARKER_CHANNEL_SEND}\nunsupported channel-send target: {channel_name}"),
        );
    }
    let now = Utc::now().timestamp();

    // Atomically check rate limit AND record the timestamp under a single
    // lock acquisition to prevent TOCTOU where two concurrent sends both
    // pass the check before either updates the tracker.
    {
        let mut tracker = match CHANNEL_SEND_LAST_SEEN.lock() {
            Ok(tracker) => tracker,
            Err(error) => {
                return (
                    false,
                    format!("{ROUTE_MARKER_CHANNEL_SEND}\nrate limiter failed: {error}"),
                );
            }
        };

        if let Some(previous) = tracker.get(channel_id.as_str()).copied() {
            let elapsed = now.saturating_sub(previous);
            if elapsed < CHANNEL_SEND_MIN_INTERVAL_SECONDS {
                let wait_seconds = CHANNEL_SEND_MIN_INTERVAL_SECONDS - elapsed;
                return (
                    false,
                    format!(
                        "{ROUTE_MARKER_CHANNEL_SEND}\naccepted=false\nreason=rate_limited\nwait_seconds={wait_seconds}"
                    ),
                );
            }
        }
        // Record the send timestamp while still holding the lock.
        tracker.insert(channel_id.to_string(), now);
    }

    if let Some(output) = consume_security_or_output(security, ROUTE_MARKER_CHANNEL_SEND) {
        rollback_channel_send_rate_limit(&channel_id);
        return output;
    }

    #[cfg(feature = "discord")]
    {
        let token = match config.channels_config.discord.as_ref() {
            Some(discord) if !discord.bot_token.trim().is_empty() => discord.bot_token.as_str(),
            _ => {
                rollback_channel_send_rate_limit(&channel_id);
                return (
                    false,
                    format!("{ROUTE_MARKER_CHANNEL_SEND}\nmissing discord bot token"),
                );
            }
        };

        let client =
            crate::transport::channels::discord::http_client::DiscordHttpClient::new(token);
        if let Err(error) = client.send_message(channel_id.as_str(), &message).await {
            rollback_channel_send_rate_limit(&channel_id);
            return (
                false,
                format!("{ROUTE_MARKER_CHANNEL_SEND}\ndiscord send failed: {error}"),
            );
        }
        (
            true,
            format!("{ROUTE_MARKER_CHANNEL_SEND}\nmessage sent to {channel_name}:{channel_id}"),
        )
    }

    #[cfg(not(feature = "discord"))]
    {
        rollback_channel_send_rate_limit(&channel_id);
        let _ = (config, channel_name, message);
        (
            false,
            format!("{ROUTE_MARKER_CHANNEL_SEND}\ndiscord feature is disabled"),
        )
    }
}

/// Runs a trend aggregation cron job.
pub(super) async fn run_trend_aggregation_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: ParsedTrendAggregationJob,
) -> (bool, String) {
    if let Some(output) = consume_security_or_output(security, ROUTE_MARKER_TREND_AGGREGATION) {
        return output;
    }

    let memory = match create_memory_or_output(config, ROUTE_MARKER_TREND_AGGREGATION).await {
        Ok(memory) => memory,
        Err(output) => return output,
    };

    let recalled = match memory
        .recall_scoped(RecallQuery::new(
            &job.entity_id,
            &job.query,
            TREND_AGGREGATION_LIMIT,
        ))
        .await
    {
        Ok(items) => items,
        Err(error) => {
            return (
                false,
                format!("{ROUTE_MARKER_TREND_AGGREGATION}\nrecall_scoped failed: {error}"),
            );
        }
    };

    let mut candidates = recalled
        .into_iter()
        .filter(|item| item.slot_key.as_str().starts_with(PREFIX_EXTERNAL))
        .collect::<Vec<_>>();
    candidates.truncate(TREND_AGGREGATION_TOP_ITEMS);

    if candidates.is_empty() {
        return (
            true,
            format!(
                "{ROUTE_MARKER_TREND_AGGREGATION}\naccepted=false\nreason=no_external_candidates"
            ),
        );
    }

    let slot_key = format!("trend.snapshot.{}", job.topic_key);
    let mut summary = String::new();
    for item in &candidates {
        if !summary.is_empty() {
            summary.push_str(" | ");
        }
        let _ = write!(
            summary,
            "{}({:.2}):{}",
            item.slot_key,
            item.score,
            item.value.replace('\n', " ")
        );
    }
    let payload = format!(
        "trend topic={} query='{}' candidates={} top={}",
        job.topic_key,
        job.query,
        candidates.len(),
        summary
    );

    let input = MemoryEventInput::new(
        &job.entity_id,
        &slot_key,
        MemoryEventType::SummaryCompacted,
        payload,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Working)
    .with_importance(0.6)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        format!("ingestion:trend:{}", job.topic_key),
    ));

    match memory.append_event(input).await {
        Ok(_) => (
            true,
            format!(
                "{ROUTE_MARKER_TREND_AGGREGATION}\naccepted=true\nslot_key={slot_key}\nsource_count={}\nquery={}",
                candidates.len(),
                job.query
            ),
        ),
        Err(error) => (
            false,
            format!("{ROUTE_MARKER_TREND_AGGREGATION}\nappend_event failed: {error}"),
        ),
    }
}

/// Runs an RSS poll cron job, fetching and ingesting feed items.
pub(super) async fn run_rss_poll_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: ParsedRssPollJob,
) -> (bool, String) {
    if let Some(output) = consume_security_or_output(security, ROUTE_MARKER_RSS_POLL) {
        return output;
    }

    if let Err(error) = validate_fetch_url(&job.url, false).await {
        return (
            false,
            format!("{ROUTE_MARKER_RSS_POLL}\ninvalid rss url: {error}"),
        );
    }

    let rss_client = crate::utils::http::build_http_client_with(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)),
    );
    let response = match rss_client.get(&job.url).send().await {
        Ok(resp) => resp,
        Err(error) => {
            return (
                false,
                format!("{ROUTE_MARKER_RSS_POLL}\nrequest failed: {error}"),
            );
        }
    };
    if !response.status().is_success() {
        return (
            false,
            format!(
                "{ROUTE_MARKER_RSS_POLL}\nrss fetch non-success status={}",
                response.status()
            ),
        );
    }

    let xml = match response.text().await {
        Ok(body) => body,
        Err(error) => {
            return (
                false,
                format!("{ROUTE_MARKER_RSS_POLL}\nresponse decode failed: {error}"),
            );
        }
    };

    let items = parse_rss_items_from_xml(&xml, 10);
    if items.is_empty() {
        return (
            true,
            format!("{ROUTE_MARKER_RSS_POLL}\naccepted=false\nreason=no_items"),
        );
    }

    let pipeline = match create_ingestion_pipeline_or_output(config, ROUTE_MARKER_RSS_POLL).await {
        Ok(pipeline) => pipeline,
        Err(output) => return output,
    };

    let envelopes = build_rss_poll_envelopes(job.entity_id.as_str(), items);
    match pipeline.ingest_batch(envelopes).await {
        Ok(results) => {
            let accepted_count = results.iter().filter(|item| item.accepted).count();
            (
                true,
                format!(
                    "{ROUTE_MARKER_RSS_POLL}\naccepted=true\naccepted_count={accepted_count}\ntotal={}\nurl={}",
                    results.len(),
                    job.url
                ),
            )
        }
        Err(error) => (
            false,
            format!("{ROUTE_MARKER_RSS_POLL}\ningestion batch failed: {error}"),
        ),
    }
}

/// Runs an X/Twitter poll cron job, searching and ingesting
/// tweets.
pub(super) async fn run_x_poll_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: ParsedXPollJob,
) -> (bool, String) {
    if let Some(output) = consume_security_or_output(security, ROUTE_MARKER_X_POLL) {
        return output;
    }

    let token = match resolve_x_bearer_token(std::env::var("X_BEARER_TOKEN").ok()) {
        Ok(token) => token,
        Err(output) => return (false, output),
    };

    let endpoint = match resolve_and_validate_x_endpoint().await {
        Ok(endpoint) => endpoint,
        Err(output) => return (false, output),
    };

    let tweets = match fetch_x_recent_tweets(&endpoint, &token, &job.query).await {
        Ok(tweets) => tweets,
        Err(output) => return output,
    };

    if tweets.is_empty() {
        return (
            true,
            format!("{ROUTE_MARKER_X_POLL}\naccepted=false\nreason=no_tweets"),
        );
    }

    let pipeline = match create_ingestion_pipeline_or_output(config, ROUTE_MARKER_X_POLL).await {
        Ok(pipeline) => pipeline,
        Err(output) => return output,
    };

    let envelopes = build_x_poll_envelopes(job.entity_id.as_str(), &job.query, tweets);
    ingest_x_poll_batch(&pipeline, envelopes, &job.query).await
}

async fn resolve_and_validate_x_endpoint() -> Result<url::Url, String> {
    let raw =
        resolve_x_recent_search_endpoint(std::env::var("ASTEREL_X_RECENT_SEARCH_ENDPOINT").ok());
    validate_fetch_url(&raw, true)
        .await
        .map_err(|error| format!("{ROUTE_MARKER_X_POLL}\ninvalid x endpoint: {error}"))
}

async fn fetch_x_recent_tweets(
    endpoint: &url::Url,
    token: &str,
    query: &str,
) -> Result<Vec<XRecentTweet>, (bool, String)> {
    let client = crate::utils::http::build_http_client_with(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)),
    );

    let response = client
        .get(endpoint.as_str())
        .bearer_auth(token)
        .query(&[
            ("query", query),
            ("max_results", "10"),
            ("tweet.fields", "created_at,lang,author_id"),
        ])
        .send()
        .await
        .map_err(|error| {
            (
                false,
                format!("{ROUTE_MARKER_X_POLL}\nrequest failed: {error}"),
            )
        })?;

    if !response.status().is_success() {
        return Err((
            false,
            format!(
                "{ROUTE_MARKER_X_POLL}\nx api non-success status={}",
                response.status()
            ),
        ));
    }

    let parsed: XRecentSearchResponse = response.json().await.map_err(|error| {
        (
            false,
            format!("{ROUTE_MARKER_X_POLL}\nresponse decode failed: {error}"),
        )
    })?;

    Ok(parsed.data)
}

async fn ingest_x_poll_batch(
    pipeline: &DefaultIngestPipeline,
    envelopes: Vec<SignalEnvelope>,
    query: &str,
) -> (bool, String) {
    match pipeline.ingest_batch(envelopes).await {
        Ok(results) => {
            let accepted_count = results.iter().filter(|item| item.accepted).count();
            (
                true,
                format!(
                    "{ROUTE_MARKER_X_POLL}\naccepted=true\naccepted_count={accepted_count}\ntotal={}\nquery={query}",
                    results.len(),
                ),
            )
        }
        Err(error) => (
            false,
            format!("{ROUTE_MARKER_X_POLL}\ningestion batch failed: {error}"),
        ),
    }
}

#[cfg(test)]
pub(super) fn set_channel_send_last_seen_for_test(channel_id: &str, seen_at: i64) {
    if let Ok(mut tracker) = CHANNEL_SEND_LAST_SEEN.lock() {
        tracker.insert(channel_id.to_string(), seen_at);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHANNEL_SEND_LAST_SEEN, CHANNEL_SEND_MIN_INTERVAL_SECONDS, ParsedIngestionJob,
        ParsedRoutedJob, ROUTE_MARKER_CHANNEL_SEND, ROUTE_MARKER_INGEST_PIPELINE, RssPollItem,
        SourceKind, X_RECENT_SEARCH_ENDPOINT, XRecentTweet, build_rss_poll_envelopes,
        build_x_poll_envelopes, check_and_record_ingestion_rate_limit, malformed_routed_job_output,
        parse_routed_job_command, parse_rss_items_from_xml, resolve_x_bearer_token,
        resolve_x_recent_search_endpoint, run_channel_send_job_command, run_ingestion_job_command,
        set_channel_send_last_seen_for_test,
    };
    use crate::config::{Config, MemoryBackend, MemoryConfig};
    use crate::contracts::ids::EntityId;
    use crate::security::SecurityPolicy;
    use chrono::Utc;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        Config {
            workspace_dir,
            memory: MemoryConfig {
                backend: MemoryBackend::Markdown,
                ..MemoryConfig::default()
            },
            ..Config::default()
        }
    }

    fn permissive_security(config: &Config) -> SecurityPolicy {
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir)
    }

    #[test]
    fn test_helper_sets_channel_send_timestamp() {
        let channel_id = "test-channel-send";
        set_channel_send_last_seen_for_test(channel_id, 1234);
        let tracker = CHANNEL_SEND_LAST_SEEN.lock().expect("tracker mutex");
        assert_eq!(tracker.get(channel_id).copied(), Some(1234));
    }

    #[test]
    fn ingestion_rate_limit_tracker_blocks_immediate_repeat() {
        let job = ParsedIngestionJob {
            source_kind: SourceKind::Api,
            entity_id: EntityId::new("person:test"),
            source_ref: format!(
                "api-rate-limit-test-{}",
                Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            content: "hello".to_string(),
        };

        assert_eq!(check_and_record_ingestion_rate_limit(&job).unwrap(), None);
        assert!(
            check_and_record_ingestion_rate_limit(&job)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn parse_routed_job_command_covers_supported_variants() {
        match parse_routed_job_command("channel-send discord 12345 hello there") {
            Some(ParsedRoutedJob::ChannelSend {
                channel_name,
                channel_id,
                message,
            }) => {
                assert_eq!(channel_name, "discord");
                assert_eq!(channel_id.as_str(), "12345");
                assert_eq!(message, "hello there");
            }
            other => panic!("unexpected channel-send parse result: {other:?}"),
        }

        match parse_routed_job_command("ingest:api person:test api-source hello") {
            Some(ParsedRoutedJob::Ingestion(job)) => {
                assert_eq!(job.source_kind, SourceKind::Api);
                assert_eq!(job.entity_id.as_str(), "person:test");
                assert_eq!(job.source_ref, "api-source");
                assert_eq!(job.content, "hello");
            }
            other => panic!("unexpected ingest:api parse result: {other:?}"),
        }

        match parse_routed_job_command("ingest:x person:test tweet-1 hello x") {
            Some(ParsedRoutedJob::Ingestion(job)) => {
                assert_eq!(job.source_kind, SourceKind::Api);
                assert_eq!(job.source_ref, "x:tweet-1");
                assert_eq!(job.content, "hello x");
            }
            other => panic!("unexpected ingest:x parse result: {other:?}"),
        }

        match parse_routed_job_command("ingest:x-poll person:test rustlang from:rustlang") {
            Some(ParsedRoutedJob::XPoll(job)) => {
                assert_eq!(job.entity_id.as_str(), "person:test");
                assert_eq!(job.query, "rustlang from:rustlang");
            }
            other => panic!("unexpected ingest:x-poll parse result: {other:?}"),
        }

        match parse_routed_job_command("ingest:rss-poll person:test https://example.com/feed.xml") {
            Some(ParsedRoutedJob::RssPoll(job)) => {
                assert_eq!(job.entity_id.as_str(), "person:test");
                assert_eq!(job.url, "https://example.com/feed.xml");
            }
            other => panic!("unexpected ingest:rss-poll parse result: {other:?}"),
        }

        match parse_routed_job_command("ingest:rss person:test source-rss rss body") {
            Some(ParsedRoutedJob::Ingestion(job)) => {
                assert_eq!(job.source_kind, SourceKind::News);
                assert_eq!(job.source_ref, "source-rss");
                assert_eq!(job.content, "rss body");
            }
            other => panic!("unexpected ingest:rss parse result: {other:?}"),
        }

        match parse_routed_job_command("ingest:trend person:test Release/Notes release pulse") {
            Some(ParsedRoutedJob::TrendAggregation(job)) => {
                assert_eq!(job.entity_id.as_str(), "person:test");
                assert_eq!(job.topic_key, "release.notes");
                assert_eq!(job.query, "release pulse");
            }
            other => panic!("unexpected ingest:trend parse result: {other:?}"),
        }

        assert!(parse_routed_job_command("ingest:api person:test source-only").is_none());
        assert!(parse_routed_job_command("channel-send discord 12345").is_none());
        assert!(parse_routed_job_command("echo hello").is_none());
    }

    #[test]
    fn malformed_reserved_routes_do_not_fall_back_to_shell() {
        let invalid_ingest = malformed_routed_job_output("ingest:api person:test source-only")
            .expect("reserved ingest prefix should be rejected");
        assert!(invalid_ingest.contains(super::ROUTE_MARKER_INVALID_ROUTE));
        assert!(invalid_ingest.contains("malformed reserved route"));

        let invalid_channel = malformed_routed_job_output("channel-send discord 12345")
            .expect("reserved channel prefix should be rejected");
        assert!(invalid_channel.contains(super::ROUTE_MARKER_INVALID_ROUTE));

        assert!(malformed_routed_job_output("echo ingest:api is just text").is_none());
    }

    #[test]
    fn resolve_x_helpers_trim_non_empty_values_and_fallback() {
        assert_eq!(
            resolve_x_bearer_token(Some("  bearer-token  ".to_string())).expect("token"),
            "bearer-token"
        );
        assert!(resolve_x_bearer_token(Some("   ".to_string())).is_err());

        assert_eq!(
            resolve_x_recent_search_endpoint(Some("  https://example.test/search  ".to_string())),
            "https://example.test/search"
        );
        assert_eq!(
            resolve_x_recent_search_endpoint(Some("   ".to_string())),
            X_RECENT_SEARCH_ENDPOINT
        );
    }

    #[test]
    fn build_x_poll_envelopes_preserves_metadata_and_language() {
        let envelopes = build_x_poll_envelopes(
            "person:test",
            "rustlang",
            vec![XRecentTweet {
                id: "tweet-1".to_string(),
                text: "hello timeline".to_string(),
                lang: Some("en".to_string()),
                author_id: Some("author-1".to_string()),
            }],
        );

        assert_eq!(envelopes.len(), 1);
        let envelope = &envelopes[0];
        assert_eq!(envelope.source_ref, "x:tweet-1");
        assert_eq!(envelope.content, "hello timeline");
        assert_eq!(envelope.language.as_deref(), Some("en"));
        assert_eq!(
            envelope.metadata.get("x_query").map(String::as_str),
            Some("rustlang")
        );
        assert_eq!(
            envelope.metadata.get("x_author_id").map(String::as_str),
            Some("author-1")
        );
    }

    #[test]
    fn parse_rss_items_from_xml_handles_limit_and_partial_failures() {
        let xml = r#"
            <rss><channel>
              <item><guid>first</guid><title>First</title><description>Body</description></item>
              <item><link>https://example.com/two</link><title>Second only</title></item>
              <item><description>Desc only</description></item>
            </channel></rss>
        "#;
        let items = parse_rss_items_from_xml(xml, 3);

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].source_ref, "rss:first");
        assert_eq!(items[0].content, "First - Body");
        assert_eq!(items[1].source_ref, "rss:https://example.com/two");
        assert_eq!(items[1].content, "Second only");
        assert_eq!(items[2].source_ref, "rss:rss-item-3");
        assert_eq!(items[2].content, "Desc only");

        assert!(parse_rss_items_from_xml(xml, 0).is_empty());

        let malformed = r#"
            <rss><channel>
              <item><guid>good</guid><title>Good</title><description>Kept</description></item>
              <item><title>Broken
            </channel></rss>
        "#;
        let partial = parse_rss_items_from_xml(malformed, 10);
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].source_ref, "rss:good");
        assert_eq!(partial[0].content, "Good - Kept");
    }

    #[test]
    fn build_rss_poll_envelopes_marks_private_news_signals() {
        let envelopes = build_rss_poll_envelopes(
            "person:test",
            vec![RssPollItem {
                source_ref: "rss:item-1".to_string(),
                content: "Item content".to_string(),
            }],
        );

        assert_eq!(envelopes.len(), 1);
        let envelope = &envelopes[0];
        assert_eq!(envelope.source_ref, "rss:item-1");
        assert_eq!(envelope.content, "Item content");
        assert_eq!(envelope.source_kind, SourceKind::News);
        assert_eq!(
            envelope.privacy_level,
            crate::core::memory::PrivacyLevel::Private
        );
    }

    #[tokio::test]
    async fn channel_send_invalid_format_reports_user_route_marker() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);
        let security = permissive_security(&config);

        let (success, output) =
            run_channel_send_job_command(&config, &security, "channel-send discord").await;

        assert!(!success, "{output}");
        assert!(output.contains(ROUTE_MARKER_CHANNEL_SEND), "{output}");
        assert!(output.contains("invalid channel-send format"), "{output}");
    }

    #[tokio::test]
    async fn channel_send_rate_limit_reports_wait_seconds() {
        let channel_id = format!(
            "channel-rate-limit-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        set_channel_send_last_seen_for_test(&channel_id, Utc::now().timestamp());

        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);
        let security = permissive_security(&config);

        let (success, output) = run_channel_send_job_command(
            &config,
            &security,
            &format!("channel-send discord {channel_id} hello"),
        )
        .await;

        assert!(!success, "{output}");
        assert!(output.contains("reason=rate_limited"), "{output}");
        let wait_seconds = output
            .lines()
            .find_map(|line| line.strip_prefix("wait_seconds="))
            .and_then(|value| value.parse::<i64>().ok())
            .expect("wait_seconds line");
        assert!((1..=CHANNEL_SEND_MIN_INTERVAL_SECONDS).contains(&wait_seconds));
    }

    #[tokio::test]
    async fn channel_send_rejects_unsupported_channel_names_before_dispatch() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);
        let security = permissive_security(&config);

        let (success, output) =
            run_channel_send_job_command(&config, &security, "channel-send slack abc hello").await;

        assert!(!success, "{output}");
        assert!(output.contains(ROUTE_MARKER_CHANNEL_SEND), "{output}");
        assert!(
            output.contains("unsupported channel-send target: slack"),
            "{output}"
        );
    }

    #[tokio::test]
    async fn channel_send_failures_do_not_consume_rate_limit_budget() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);
        let security = permissive_security(&config);
        let command = format!(
            "channel-send discord retry-channel-{} hello",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );

        let (first_success, first_output) =
            run_channel_send_job_command(&config, &security, &command).await;
        assert!(!first_success, "{first_output}");
        assert!(
            first_output.contains("missing discord bot token"),
            "{first_output}"
        );

        let (second_success, second_output) =
            run_channel_send_job_command(&config, &security, &command).await;
        assert!(!second_success, "{second_output}");
        assert!(
            second_output.contains("missing discord bot token"),
            "{second_output}"
        );
        assert!(
            !second_output.contains("reason=rate_limited"),
            "{second_output}"
        );
    }

    #[tokio::test]
    async fn channel_send_security_block_does_not_consume_rate_limit_budget() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.autonomy.max_actions_per_hour = 0;
        let security = permissive_security(&config);
        let command = format!(
            "channel-send discord blocked-channel-{} hello",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );

        let (first_success, first_output) =
            run_channel_send_job_command(&config, &security, &command).await;
        assert!(!first_success, "{first_output}");
        assert!(
            first_output.contains(ROUTE_MARKER_CHANNEL_SEND),
            "{first_output}"
        );
        assert!(
            first_output.contains("blocked by security policy"),
            "{first_output}"
        );

        let (second_success, second_output) =
            run_channel_send_job_command(&config, &security, &command).await;
        assert!(!second_success, "{second_output}");
        assert!(
            second_output.contains("blocked by security policy"),
            "{second_output}"
        );
        assert!(
            !second_output.contains("reason=rate_limited"),
            "{second_output}"
        );
    }

    #[tokio::test]
    async fn ingestion_security_block_does_not_consume_rate_limit_budget() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.autonomy.max_actions_per_hour = 0;
        let security = permissive_security(&config);
        let job = ParsedIngestionJob {
            source_kind: SourceKind::Api,
            entity_id: EntityId::new("person:ingest.retry"),
            source_ref: format!(
                "api-retry-{}",
                Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            content: "hello".to_string(),
        };

        let (first_success, first_output) =
            run_ingestion_job_command(&config, &security, job.clone()).await;
        assert!(!first_success, "{first_output}");
        assert!(
            first_output.contains(ROUTE_MARKER_INGEST_PIPELINE),
            "{first_output}"
        );
        assert!(
            first_output.contains("blocked by security policy"),
            "{first_output}"
        );

        let (second_success, second_output) =
            run_ingestion_job_command(&config, &security, job).await;
        assert!(!second_success, "{second_output}");
        assert!(
            second_output.contains("blocked by security policy"),
            "{second_output}"
        );
        assert!(
            !second_output.contains("reason=rate_limited"),
            "{second_output}"
        );
    }
}
