//! Signal envelope: the ingestion-layer input record carrying source
//! metadata, content, privacy level, and signal tier.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::contracts::ids::EntityId;
use crate::core::memory::ingestion::IngestionError;
use crate::core::memory::ingestion::IngestionPipelineResult;
use crate::core::memory::memory_types::{PrivacyLevel, SignalTier, SourceKind};

/// Ingestion-layer input record carrying source metadata and content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEnvelope {
    /// Kind of source this signal originated from.
    pub source_kind: SourceKind,
    /// Unique reference within the source (e.g. message ID).
    pub source_ref: String,
    /// Textual content of the signal.
    pub content: String,
    /// Entity this signal is associated with.
    pub entity_id: EntityId,
    /// Signal tier (raw, belief, inferred, governance).
    pub signal_tier: SignalTier,
    /// Privacy classification for the content.
    pub privacy_level: PrivacyLevel,
    /// Optional ISO language code.
    pub language: Option<String>,
    /// Arbitrary key-value metadata pairs.
    pub metadata: HashMap<String, String>,
    /// RFC 3339 timestamp of when this envelope was created.
    pub ingested_at: String,
}

impl SignalEnvelope {
    /// Create a new envelope with default tier, privacy, and
    /// timestamp.
    pub fn new(
        source_kind: SourceKind,
        source_ref: impl Into<String>,
        content: impl Into<String>,
        entity_id: impl AsRef<str>,
    ) -> Self {
        Self {
            source_kind,
            source_ref: source_ref.into(),
            content: content.into(),
            entity_id: EntityId::new(entity_id.as_ref()),
            signal_tier: SignalTier::Raw,
            privacy_level: PrivacyLevel::Public,
            language: None,
            metadata: HashMap::new(),
            ingested_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Set the signal tier.
    #[must_use]
    pub fn with_signal_tier(mut self, tier: SignalTier) -> Self {
        self.signal_tier = tier;
        self
    }

    /// Set the privacy level.
    #[must_use]
    pub fn with_privacy_level(mut self, level: PrivacyLevel) -> Self {
        self.privacy_level = level;
        self
    }

    /// Set the language code.
    #[must_use]
    pub fn with_language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }

    /// Add a metadata key-value pair.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Return the source kind as a string label.
    #[must_use]
    pub fn source_kind_str(&self) -> String {
        self.source_kind.to_string()
    }

    /// # Errors
    ///
    /// Returns an error when source reference, content, entity id, or language
    /// fields cannot be normalized.
    pub fn normalize(mut self) -> IngestionPipelineResult<Self> {
        self.source_ref = normalize_source_ref(&self.source_ref)?;
        self.content = normalize_content(&self.content)?;
        self.entity_id = EntityId::new(normalize_entity_id(self.entity_id.as_str())?);
        self.language = self
            .language
            .as_deref()
            .map(normalize_language)
            .transpose()?;
        self.ingested_at = normalize_ingested_at(&self.ingested_at);
        self.apply_rule_based_classification();
        Ok(self)
    }

    fn apply_rule_based_classification(&mut self) {
        let content_lower = self.content.to_ascii_lowercase();

        let mut risk_flags = Vec::new();
        if contains_any(
            &content_lower,
            &["rumor", "unverified", "allegedly", "未確認", "噂"],
        ) {
            risk_flags.push("rumor");
            risk_flags.push("unverified");
        }
        if contains_any(
            &content_lower,
            &[
                "password",
                "api key",
                "token",
                "secret",
                "個人情報",
                "住所",
                "電話番号",
            ],
        ) {
            risk_flags.push("sensitive");
        }
        if contains_any(
            &content_lower,
            &[
                "policy",
                "ban",
                "compliance",
                "regulation",
                "利用規約",
                "コンプライアンス",
            ],
        ) {
            risk_flags.push("policy_risky");
        }
        if !risk_flags.is_empty() {
            risk_flags.sort_unstable();
            risk_flags.dedup();
            self.metadata
                .insert("risk_flags".to_string(), risk_flags.join("|"));
        }

        let topic = infer_topic(&content_lower, self.source_kind);
        self.metadata
            .entry("topic".to_string())
            .or_insert(topic.to_string());

        let entity = infer_entity_hint(self.entity_id.as_str());
        self.metadata
            .entry("entity_hint".to_string())
            .or_insert(entity);
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn infer_topic(content_lower: &str, source_kind: SourceKind) -> &'static str {
    if contains_any(
        content_lower,
        &[
            "security",
            "vulnerability",
            "exploit",
            "脆弱性",
            "セキュリティ",
        ],
    ) {
        return "security";
    }
    if contains_any(
        content_lower,
        &["release", "version", "deploy", "リリース", "デプロイ"],
    ) {
        return "technical_change";
    }
    if contains_any(content_lower, &["price", "market", "stocks", "株", "相場"]) {
        return "market";
    }

    match source_kind {
        SourceKind::News => "news",
        SourceKind::Document => "document",
        SourceKind::Conversation => "conversation",
        SourceKind::Discord | SourceKind::Telegram | SourceKind::Slack => "community",
        SourceKind::Api => "api",
        SourceKind::Manual => "manual",
    }
}

fn infer_entity_hint(entity_id: &str) -> String {
    if let Some((prefix, rest)) = entity_id.split_once(':')
        && !rest.is_empty()
    {
        return format!("{prefix}:{rest}");
    }
    entity_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_topic_uses_neutral_change_label_for_release_language() {
        assert_eq!(
            infer_topic("release checklist and deploy notes", SourceKind::Document),
            "technical_change"
        );
    }
}

fn normalize_source_ref(raw: &str) -> IngestionPipelineResult<String> {
    let normalized = normalize_identifier(raw, true);
    if normalized.is_empty() {
        return Err(IngestionError::validation(
            "signal_envelope.source_ref must not be empty",
        ));
    }
    if normalized.len() > 256 {
        return Err(IngestionError::validation(
            "signal_envelope.source_ref must be <= 256 chars",
        ));
    }
    Ok(normalized)
}

fn normalize_content(raw: &str) -> IngestionPipelineResult<String> {
    let mut normalized = String::with_capacity(raw.len());
    for word in raw.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    if normalized.is_empty() {
        return Err(IngestionError::validation(
            "signal_envelope.content must not be empty",
        ));
    }
    Ok(normalized)
}

fn normalize_entity_id(raw: &str) -> IngestionPipelineResult<String> {
    let normalized = normalize_identifier(raw, false);
    if normalized.is_empty() {
        return Err(IngestionError::validation(
            "signal_envelope.entity_id must not be empty",
        ));
    }
    if normalized.len() > 128 {
        return Err(IngestionError::validation(
            "signal_envelope.entity_id must be <= 128 chars",
        ));
    }
    Ok(normalized)
}

fn normalize_language(raw: &str) -> IngestionPipelineResult<String> {
    let candidate = raw.trim().to_ascii_lowercase();
    let normalized = candidate
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '-')
        .collect::<String>();
    if normalized.is_empty() {
        return Err(IngestionError::validation(
            "signal_envelope.language must contain at least one valid character",
        ));
    }
    if normalized.len() > 16 {
        return Err(IngestionError::validation(
            "signal_envelope.language must be <= 16 chars",
        ));
    }
    Ok(normalized)
}

fn normalize_ingested_at(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map_or_else(|_| chrono::Utc::now().to_rfc3339(), |dt| dt.to_rfc3339())
}

use crate::core::memory::identifier::normalize_identifier;
