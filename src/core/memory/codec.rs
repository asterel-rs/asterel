//! Shared enum-to-string codec for memory backends.
//!
//! Provides conversion functions between memory domain enums and their
//! canonical string representations used in storage backends (`PostgreSQL`,
//! `Markdown`, etc.). Centralizing these mappings ensures the `PostgreSQL`
//! column values and `Markdown` file tokens stay in sync across the codebase.

use super::traits::{MemoryLayer, MemorySource, PrivacyLevel, SignalTier, SourceKind};

/// Convert a `MemorySource` to its canonical string representation.
#[must_use]
pub fn source_to_str(source: MemorySource) -> &'static str {
    match source {
        MemorySource::ExplicitUser => "explicit_user",
        MemorySource::ToolVerified => "tool_verified",
        MemorySource::System => "system",
        MemorySource::Inferred => "inferred",
        MemorySource::ExternalPrimary => "external_primary",
        MemorySource::ExternalSecondary => "external_secondary",
    }
}

/// Parse a string into a `MemorySource`, defaulting to `System`.
#[must_use]
pub fn str_to_source(source: &str) -> MemorySource {
    match source {
        "explicit_user" => MemorySource::ExplicitUser,
        "tool_verified" => MemorySource::ToolVerified,
        "inferred" => MemorySource::Inferred,
        "external_primary" => MemorySource::ExternalPrimary,
        "external_secondary" => MemorySource::ExternalSecondary,
        _ => MemorySource::System,
    }
}

/// Convert a `SignalTier` to its canonical string representation.
#[must_use]
pub fn signal_tier_to_str(tier: SignalTier) -> &'static str {
    match tier {
        SignalTier::Raw => "raw",
        SignalTier::Belief => "belief",
        SignalTier::Inferred => "inferred",
        SignalTier::Governance => "governance",
    }
}

/// Parse a string into a `SignalTier`, defaulting to `Raw`.
#[must_use]
pub fn str_to_signal_tier(s: &str) -> SignalTier {
    match s {
        "belief" => SignalTier::Belief,
        "inferred" => SignalTier::Inferred,
        "governance" => SignalTier::Governance,
        _ => SignalTier::Raw,
    }
}

/// Convert a `SourceKind` to its canonical string representation.
#[must_use]
pub fn source_kind_to_str(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Conversation => "conversation",
        SourceKind::Discord => "discord",
        SourceKind::Telegram => "telegram",
        SourceKind::Slack => "slack",
        SourceKind::Api => "api",
        SourceKind::News => "news",
        SourceKind::Document => "document",
        SourceKind::Manual => "manual",
    }
}

/// Parse a string into a `SourceKind`, returning `None` for unknown.
#[must_use]
pub fn str_to_source_kind(s: &str) -> Option<SourceKind> {
    match s {
        "conversation" => Some(SourceKind::Conversation),
        "discord" => Some(SourceKind::Discord),
        "telegram" => Some(SourceKind::Telegram),
        "slack" => Some(SourceKind::Slack),
        "api" => Some(SourceKind::Api),
        "news" => Some(SourceKind::News),
        "document" => Some(SourceKind::Document),
        "manual" => Some(SourceKind::Manual),
        _ => None,
    }
}

/// Convert a `MemoryLayer` to its canonical string representation.
#[must_use]
pub fn layer_to_str(layer: MemoryLayer) -> &'static str {
    match layer {
        MemoryLayer::Working => "working",
        MemoryLayer::Episodic => "episodic",
        MemoryLayer::Semantic => "semantic",
        MemoryLayer::Procedural => "procedural",
        MemoryLayer::Identity => "identity",
    }
}

/// Return the retention tier string for a given memory layer.
#[must_use]
pub fn retention_tier_for_layer(layer: MemoryLayer) -> &'static str {
    layer_to_str(layer)
}

/// Compute the retention expiry timestamp for a memory layer.
///
/// Returns `None` for permanent layers (semantic, procedural, identity).
#[must_use]
pub fn retention_expiry_for_layer(layer: MemoryLayer, occurred_at: &str) -> Option<String> {
    let retention_days = match layer {
        MemoryLayer::Working => Some(2),
        MemoryLayer::Episodic => Some(30),
        MemoryLayer::Semantic | MemoryLayer::Procedural | MemoryLayer::Identity => None,
    }?;

    chrono::DateTime::parse_from_rfc3339(occurred_at)
        .ok()
        .map(|ts| (ts + chrono::Duration::days(retention_days)).to_rfc3339())
}

/// Convert a `PrivacyLevel` to its canonical string representation.
#[must_use]
pub fn privacy_to_str(level: &PrivacyLevel) -> &'static str {
    match level {
        PrivacyLevel::Public => "public",
        PrivacyLevel::Private => "private",
        PrivacyLevel::Secret => "secret",
    }
}

/// Parse a string into a `PrivacyLevel`, defaulting to `Private`.
#[must_use]
pub fn str_to_privacy(level: &str) -> PrivacyLevel {
    match level {
        "public" => PrivacyLevel::Public,
        "secret" => PrivacyLevel::Secret,
        _ => PrivacyLevel::Private,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_layers() -> [MemoryLayer; 5] {
        [
            MemoryLayer::Working,
            MemoryLayer::Episodic,
            MemoryLayer::Semantic,
            MemoryLayer::Procedural,
            MemoryLayer::Identity,
        ]
    }

    #[test]
    fn source_round_trip_for_all_variants() {
        let sources = [
            MemorySource::ExplicitUser,
            MemorySource::ToolVerified,
            MemorySource::System,
            MemorySource::Inferred,
            MemorySource::ExternalPrimary,
            MemorySource::ExternalSecondary,
        ];

        for source in sources {
            let encoded = source_to_str(source);
            let decoded = str_to_source(encoded);
            assert_eq!(decoded, source);
        }
    }

    #[test]
    fn layer_to_str_maps_all_variants() {
        assert_eq!(layer_to_str(MemoryLayer::Working), "working");
        assert_eq!(layer_to_str(MemoryLayer::Episodic), "episodic");
        assert_eq!(layer_to_str(MemoryLayer::Semantic), "semantic");
        assert_eq!(layer_to_str(MemoryLayer::Procedural), "procedural");
        assert_eq!(layer_to_str(MemoryLayer::Identity), "identity");
    }

    #[test]
    fn retention_tier_for_layer_maps_all_variants() {
        assert_eq!(retention_tier_for_layer(MemoryLayer::Working), "working");
        assert_eq!(retention_tier_for_layer(MemoryLayer::Episodic), "episodic");
        assert_eq!(retention_tier_for_layer(MemoryLayer::Semantic), "semantic");
        assert_eq!(
            retention_tier_for_layer(MemoryLayer::Procedural),
            "procedural"
        );
        assert_eq!(retention_tier_for_layer(MemoryLayer::Identity), "identity");
    }

    #[test]
    fn retention_expiry_for_layer_maps_expected_windows() {
        let occurred_at = "2026-01-01T00:00:00+00:00";
        let occurred = chrono::DateTime::parse_from_rfc3339(occurred_at).unwrap();

        let working_expiry = retention_expiry_for_layer(MemoryLayer::Working, occurred_at)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok());
        assert_eq!(working_expiry, Some(occurred + chrono::Duration::days(2)));

        let episodic_expiry = retention_expiry_for_layer(MemoryLayer::Episodic, occurred_at)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok());
        assert_eq!(episodic_expiry, Some(occurred + chrono::Duration::days(30)));

        for layer in [
            MemoryLayer::Semantic,
            MemoryLayer::Procedural,
            MemoryLayer::Identity,
        ] {
            assert!(retention_expiry_for_layer(layer, occurred_at).is_none());
        }
    }

    #[test]
    fn privacy_round_trip() {
        let levels = [
            PrivacyLevel::Public,
            PrivacyLevel::Private,
            PrivacyLevel::Secret,
        ];

        for level in levels {
            let encoded = privacy_to_str(&level);
            let decoded = str_to_privacy(encoded);
            assert_eq!(decoded, level);
        }
    }

    #[test]
    fn str_to_source_unknown_defaults_to_system() {
        assert_eq!(str_to_source("unknown-source"), MemorySource::System);
    }

    #[test]
    fn signal_tier_round_trip_and_default() {
        let tiers = [
            SignalTier::Raw,
            SignalTier::Belief,
            SignalTier::Inferred,
            SignalTier::Governance,
        ];

        for tier in tiers {
            let encoded = signal_tier_to_str(tier);
            let decoded = str_to_signal_tier(encoded);
            assert_eq!(decoded, tier);
        }

        assert_eq!(str_to_signal_tier("unknown-tier"), SignalTier::Raw);
    }

    #[test]
    fn source_kind_round_trip_and_unknown() {
        let kinds = [
            SourceKind::Conversation,
            SourceKind::Discord,
            SourceKind::Telegram,
            SourceKind::Slack,
            SourceKind::Api,
            SourceKind::News,
            SourceKind::Document,
            SourceKind::Manual,
        ];

        for kind in kinds {
            let encoded = source_kind_to_str(kind);
            let decoded = str_to_source_kind(encoded);
            assert_eq!(decoded, Some(kind));
        }

        assert_eq!(str_to_source_kind("unknown-kind"), None);
    }

    #[test]
    fn str_to_privacy_unknown_defaults_to_private() {
        assert_eq!(str_to_privacy("unknown-privacy"), PrivacyLevel::Private);
    }

    #[test]
    fn retention_and_layer_strings_stay_in_sync() {
        for layer in all_layers() {
            assert_eq!(retention_tier_for_layer(layer), layer_to_str(layer));
        }
    }
}
