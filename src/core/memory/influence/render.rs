//! Rendering grounding contracts and memory blocks for prompt
//! injection.
//!
//! Converts a `ContextBundle` into XML-structured blocks that the
//! agent turn loop injects into user prompts. The XML format provides
//! clear structural markers (per Anthropic Context Engineering 2025)
//! and per-item citation IDs for model-verifiable referencing.

use std::collections::HashSet;
use std::fmt::Write;
use std::sync::{Mutex, OnceLock};

use super::builder::build_context_bundle;
use super::types::{ContextBundle, GroundingEntry};
use crate::core::memory::graphrag::{
    build_companion_memory_grounding, render_companion_memory_grounding,
};
use crate::core::memory::{MemoryRecallEntry, PrivacyLevel};
use crate::security::external_content::{
    ExternalAction, decide_action, detect_injection, sanitize_marker_collision, wrap_content,
};
use crate::utils::text::truncate_ellipsis_into;

const ENTRY_VALUE_MAX_CHARS: usize = 220;
const LOW_RELEVANCE_THRESHOLD: f64 = 0.5;
const MAX_VISIBLE_FACTS: usize = 6;
const MAX_VISIBLE_HINTS: usize = 4;

/// Render a grounding contract block from a context bundle.
///
/// Output is XML-structured with citation IDs per item:
/// `<grounding>` → `<instruction>` → `<facts>` → `<hints>` → `<contradicted>` → `</grounding>`
#[must_use]
pub fn render_grounding_contract(bundle: &ContextBundle) -> String {
    render_grounding_contract_with_exposure(bundle, None)
}

fn render_grounding_contract_with_exposure(
    bundle: &ContextBundle,
    exposure: Option<&GroundingExposureProjection>,
) -> String {
    if bundle.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(1024);
    out.push_str("<grounding>\n");
    out.push_str("<instruction>Cite grounding items as [F1], [H1], [C1] when referencing them. ");
    out.push_str("Caveat claims supported only by hints. ");
    out.push_str("Say \"I don't know\" when no grounding supports a claim.</instruction>\n");
    if let Some(exposure) = exposure {
        render_exposure_rail_xml(&mut out, exposure);
    }

    if bundle.all_low_relevance(LOW_RELEVANCE_THRESHOLD) {
        out.push_str(
            "<retrieval-warning>All recalled items have low relevance scores. \
             Evidence quality is weak — prefer abstention over speculation.</retrieval-warning>\n",
        );
    }

    render_recall_projection_xml(&mut out, bundle);
    render_facts_xml(&mut out, &bundle.facts);
    render_hints_xml(&mut out, &bundle.hints);
    render_contradicted_xml(&mut out, &bundle.facts, &bundle.hints);

    out.push_str("</grounding>\n");
    out
}

fn render_exposure_rail_xml(out: &mut String, exposure: &GroundingExposureProjection) {
    let _ = writeln!(
        out,
        "<exposure-rail public_visible=\"{}\" private_internal=\"{}\" \
         secret_suppressed=\"{}\">Private grounding may inform reasoning but must not be quoted \
         as sensitive disclosure unless the user explicitly asks in a private context.</exposure-rail>",
        exposure.public_visible, exposure.private_internal, exposure.secret_suppressed
    );
}

/// Build a grounding augmentation block from raw recall items.
#[must_use]
pub fn build_grounding_augmentation_block(items: &[MemoryRecallEntry]) -> String {
    let contradicted_slots = HashSet::new();
    let bundle = build_context_bundle(items, &contradicted_slots);
    render_grounding_contract(&bundle)
}

/// Build the companion-first grounding augmentation block from raw recall items.
///
/// This is the shared production path used by both transport pre-turn
/// enrichment and the `loop_` main-session augmentation path. It:
/// 1. filters out recall items below `min_confidence`
/// 2. sanitizes external replay content
/// 3. renders the XML grounding contract
/// 4. appends a compact companion-memory graph summary
#[must_use]
pub fn build_companion_grounding_augmentation_block(
    query: &str,
    items: &[MemoryRecallEntry],
    min_confidence: f64,
) -> String {
    build_companion_grounding_augmentation(query, items, min_confidence).block
}

/// Build the companion-first grounding augmentation block plus structured
/// exposure diagnostics.
#[must_use]
pub fn build_companion_grounding_augmentation(
    query: &str,
    items: &[MemoryRecallEntry],
    min_confidence: f64,
) -> CompanionGroundingAugmentation {
    let prepared = prepare_recall_items_for_grounding(items, min_confidence);
    record_grounding_exposure(prepared.exposure);
    tracing::debug!(
        public_visible = prepared.exposure.public_visible,
        private_internal = prepared.exposure.private_internal,
        secret_suppressed = prepared.exposure.secret_suppressed,
        "memory grounding exposure rail projected"
    );

    if prepared.items.is_empty() {
        return CompanionGroundingAugmentation {
            block: String::new(),
            exposure: prepared.exposure,
        };
    }

    let contradicted_slots = HashSet::new();
    let bundle = build_context_bundle(&prepared.items, &contradicted_slots);
    let mut rendered = render_grounding_contract_with_exposure(&bundle, Some(&prepared.exposure));
    let companion_grounding = build_companion_memory_grounding(query, &prepared.items);
    let companion_rendered = render_companion_memory_grounding(&companion_grounding);
    if !companion_rendered.is_empty() {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&companion_rendered);
    }

    CompanionGroundingAugmentation {
        block: rendered,
        exposure: prepared.exposure,
    }
}

fn render_recall_projection_xml(out: &mut String, bundle: &ContextBundle) {
    let visible_facts = bundle.facts.len().min(MAX_VISIBLE_FACTS);
    let visible_hints = bundle.hints.len().min(MAX_VISIBLE_HINTS);
    let omitted_facts = bundle.facts.len().saturating_sub(visible_facts);
    let omitted_hints = bundle.hints.len().saturating_sub(visible_hints);

    let _ = writeln!(
        out,
        "<recall-projection facts_visible=\"{visible_facts}\" facts_omitted=\"{omitted_facts}\" \
         hints_visible=\"{visible_hints}\" hints_omitted=\"{omitted_hints}\" noise_omitted=\"{}\">\
         Use visible facts first. Treat hints as provisional. Omitted items are not evidence.</recall-projection>",
        bundle.noise.len()
    );
}

fn sanitize_external_replay_value_for_grounding(slot_key: &str, value: &str) -> String {
    if !slot_key.starts_with(crate::contracts::strings::data_model::PREFIX_EXTERNAL) {
        return value.to_string();
    }
    if !crate::security::external_content::PersistedExternalSummary::is_memory_summary_value(value)
    {
        return "[external payload omitted by replay-ban policy]".to_string();
    }

    let signals = detect_injection(value);
    let action = decide_action(&signals);
    match action {
        ExternalAction::Allow => wrap_content(slot_key, value),
        ExternalAction::Sanitize => {
            let sanitized = sanitize_marker_collision(value);
            wrap_content(slot_key, &sanitized)
        }
        ExternalAction::Block => {
            "[external summary blocked by policy during context replay]".to_string()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompanionGroundingAugmentation {
    /// Rendered grounding block for prompt injection.
    pub block: String,
    /// Structured exposure counts for diagnostics.
    pub exposure: GroundingExposureProjection,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroundingExposureProjection {
    /// Public recall items visible in grounding.
    pub public_visible: usize,
    /// Private recall items available as internal grounding, not direct disclosure.
    pub private_internal: usize,
    /// Secret recall items suppressed before prompt grounding.
    pub secret_suppressed: usize,
}

impl GroundingExposureProjection {
    #[must_use]
    pub const fn total_considered(self) -> usize {
        self.public_visible + self.private_internal + self.secret_suppressed
    }

    #[must_use]
    pub const fn has_suppression(self) -> bool {
        self.secret_suppressed > 0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.total_considered() == 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroundingExposureMonitorSnapshot {
    /// Number of grounding augmentation builds observed in this process.
    pub observed_builds: u64,
    /// Total public recall items surfaced to grounding.
    pub public_visible_total: u64,
    /// Total private recall items used as internal grounding.
    pub private_internal_total: u64,
    /// Total secret recall items suppressed before grounding.
    pub secret_suppressed_total: u64,
    /// Last observed per-turn exposure projection.
    pub last_projection: GroundingExposureProjection,
}

#[derive(Debug, Default)]
struct GroundingExposureMonitor {
    snapshot: GroundingExposureMonitorSnapshot,
}

impl GroundingExposureMonitor {
    fn record(&mut self, projection: GroundingExposureProjection) {
        self.snapshot.observed_builds = self.snapshot.observed_builds.saturating_add(1);
        self.snapshot.public_visible_total = self
            .snapshot
            .public_visible_total
            .saturating_add(usize_to_u64(projection.public_visible));
        self.snapshot.private_internal_total = self
            .snapshot
            .private_internal_total
            .saturating_add(usize_to_u64(projection.private_internal));
        self.snapshot.secret_suppressed_total = self
            .snapshot
            .secret_suppressed_total
            .saturating_add(usize_to_u64(projection.secret_suppressed));
        self.snapshot.last_projection = projection;
    }
}

static GROUNDING_EXPOSURE_MONITOR: OnceLock<Mutex<GroundingExposureMonitor>> = OnceLock::new();

fn exposure_monitor() -> &'static Mutex<GroundingExposureMonitor> {
    GROUNDING_EXPOSURE_MONITOR.get_or_init(|| Mutex::new(GroundingExposureMonitor::default()))
}

fn record_grounding_exposure(projection: GroundingExposureProjection) {
    let mut guard = exposure_monitor()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.record(projection);
}

#[must_use]
pub fn grounding_exposure_monitor_snapshot() -> GroundingExposureMonitorSnapshot {
    exposure_monitor()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot
        .clone()
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Default)]
struct PreparedGroundingItems {
    items: Vec<MemoryRecallEntry>,
    exposure: GroundingExposureProjection,
}

fn prepare_recall_items_for_grounding(
    items: &[MemoryRecallEntry],
    min_confidence: f64,
) -> PreparedGroundingItems {
    let mut prepared = PreparedGroundingItems::default();
    for item in items {
        if item.confidence.get() < min_confidence {
            continue;
        }
        match &item.privacy_level {
            PrivacyLevel::Public => prepared.exposure.public_visible += 1,
            PrivacyLevel::Private => prepared.exposure.private_internal += 1,
            PrivacyLevel::Secret => {
                prepared.exposure.secret_suppressed += 1;
                continue;
            }
        }

        let value =
            sanitize_external_replay_value_for_grounding(item.slot_key.as_str(), &item.value);
        prepared.items.push(MemoryRecallEntry {
            value,
            ..item.clone()
        });
    }
    prepared
}

fn render_facts_xml(out: &mut String, items: &[GroundingEntry]) {
    if items.iter().all(|item| item.is_contradicted) {
        return;
    }
    out.push_str("<facts>\n");
    let mut i = 0usize;
    for item in items
        .iter()
        .filter(|it| !it.is_contradicted)
        .take(MAX_VISIBLE_FACTS)
    {
        i += 1;
        render_item_xml(out, item, 'F', i);
    }
    out.push_str("</facts>\n");
}

fn render_hints_xml(out: &mut String, items: &[GroundingEntry]) {
    if items.iter().all(|item| item.is_contradicted) {
        return;
    }
    out.push_str("<hints note=\"treat as uncertain\">\n");
    let mut i = 0usize;
    for item in items
        .iter()
        .filter(|it| !it.is_contradicted)
        .take(MAX_VISIBLE_HINTS)
    {
        i += 1;
        render_item_xml(out, item, 'H', i);
    }
    out.push_str("</hints>\n");
}

fn render_contradicted_xml(out: &mut String, facts: &[GroundingEntry], hints: &[GroundingEntry]) {
    let has_contradicted = facts
        .iter()
        .chain(hints.iter())
        .any(|item| item.is_contradicted);
    if !has_contradicted {
        return;
    }
    out.push_str("<contradicted note=\"verify before using\">\n");
    let mut i = 0usize;
    for item in facts
        .iter()
        .chain(hints.iter())
        .filter(|it| it.is_contradicted)
    {
        i += 1;
        render_item_xml_contradicted(out, item, 'C', i);
    }
    out.push_str("</contradicted>\n");
}

fn render_item_xml(out: &mut String, item: &GroundingEntry, prefix: char, index: usize) {
    let _ = write!(
        out,
        "<item id=\"{prefix}{index}\" confidence=\"{:.2}\" source=\"",
        item.confidence
    );
    xml_escape_into(out, &format!("{:?}", item.source));
    out.push_str("\" slot=\"");
    xml_escape_into(out, item.slot_key.as_str());
    out.push_str("\">");
    // Truncate + escape the value directly into `out` with no intermediate
    // String allocations.
    write_xml_escaped_truncated(out, &item.value, ENTRY_VALUE_MAX_CHARS);
    out.push_str("</item>\n");
}

fn render_item_xml_contradicted(
    out: &mut String,
    item: &GroundingEntry,
    prefix: char,
    index: usize,
) {
    let _ = write!(
        out,
        "<item id=\"{prefix}{index}\" confidence=\"{:.2}\" source=\"",
        item.confidence
    );
    xml_escape_into(out, &format!("{:?}", item.source));
    out.push_str("\" slot=\"");
    xml_escape_into(out, item.slot_key.as_str());
    out.push_str("\" tier=\"");
    xml_escape_into(out, &format!("{:?}", item.tier));
    out.push_str("\" contradicted=\"true\">");
    write_xml_escaped_truncated(out, &item.value, ENTRY_VALUE_MAX_CHARS);
    out.push_str("</item>\n");
}

/// Truncate `value` to `max_chars` and write the result XML-escaped into `out`.
/// Single pass: no intermediate String, no repeated allocations from the
/// chained `.replace()` pattern in the old `xml_escape` helper.
fn write_xml_escaped_truncated(out: &mut String, value: &str, max_chars: usize) {
    // Stage 1: copy the truncated slice into a scratch buffer we can escape.
    // We avoid the classic 4-chain `.replace()` pattern by walking the bytes
    // once and substituting the 4 special characters into `out` directly.
    //
    // The truncated view is computed in place on a scratch `String` because
    // truncation is character-oriented (we need char boundaries) whereas
    // escaping is byte-oriented. A single small allocation here is cheaper
    // than the 4 reallocs the old code did per item.
    let mut scratch = String::with_capacity(value.len().min(max_chars + 3));
    truncate_ellipsis_into(&mut scratch, value, max_chars);
    xml_escape_into(out, &scratch);
}

/// Append an XML-escaped copy of `s` to `out` in a single pass.
fn xml_escape_into(out: &mut String, s: &str) {
    let bytes = s.as_bytes();
    let mut last = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        let replacement: &str = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            _ => continue,
        };
        // SAFETY: the special chars are single-byte ASCII, so the byte offsets
        // are valid UTF-8 char boundaries.
        out.push_str(&s[last..i]);
        out.push_str(replacement);
        last = i + 1;
    }
    out.push_str(&s[last..]);
}

#[cfg(test)]
mod tests {
    use super::{
        build_companion_grounding_augmentation, build_companion_grounding_augmentation_block,
        grounding_exposure_monitor_snapshot, render_grounding_contract,
    };
    use crate::contracts::ids::EntityId;
    use crate::core::memory::MemorySource;
    use crate::core::memory::influence::{ContextBundle, GroundingEntry, GroundingTier};

    fn item(
        slot_key: &str,
        value: &str,
        tier: GroundingTier,
        contradicted: bool,
    ) -> GroundingEntry {
        GroundingEntry {
            slot_key: slot_key.into(),
            value: value.to_string(),
            tier,
            confidence: 0.9,
            source: MemorySource::ExplicitUser,
            is_contradicted: contradicted,
            recall_score: 0.8,
        }
    }

    #[test]
    fn render_grounding_contract_empty_bundle_is_empty() {
        assert!(render_grounding_contract(&ContextBundle::default()).is_empty());
    }

    #[test]
    fn render_xml_structure_with_facts_and_hints() {
        let mut bundle = ContextBundle::default();
        bundle
            .facts
            .push(item("profile.name", "Aster", GroundingTier::Fact, false));
        bundle.hints.push(item(
            "preference.locale",
            "ja-JP",
            GroundingTier::Hint,
            false,
        ));

        let rendered = render_grounding_contract(&bundle);

        assert!(rendered.contains("<grounding>"));
        assert!(rendered.contains("</grounding>"));
        assert!(rendered.contains("<instruction>"));
        assert!(rendered.contains("<facts>"));
        assert!(rendered.contains("<hints note=\"treat as uncertain\">"));
        assert!(rendered.contains("id=\"F1\""));
        assert!(rendered.contains("id=\"H1\""));
        assert!(rendered.contains("slot=\"profile.name\""));
        assert!(rendered.contains(">Aster</item>"));
        assert!(rendered.contains(">ja-JP</item>"));
    }

    #[test]
    fn render_recall_projection_limits_visible_items() {
        let mut bundle = ContextBundle::default();
        for i in 0..8 {
            bundle.facts.push(item(
                &format!("profile.fact.{i}"),
                &format!("fact {i}"),
                GroundingTier::Fact,
                false,
            ));
        }
        for i in 0..5 {
            bundle.hints.push(item(
                &format!("profile.hint.{i}"),
                &format!("hint {i}"),
                GroundingTier::Hint,
                false,
            ));
        }
        bundle
            .noise
            .push(item("profile.noise", "noise", GroundingTier::Noise, false));

        let rendered = render_grounding_contract(&bundle);

        assert!(rendered.contains("<recall-projection"));
        assert!(rendered.contains("facts_visible=\"6\""));
        assert!(rendered.contains("facts_omitted=\"2\""));
        assert!(rendered.contains("hints_visible=\"4\""));
        assert!(rendered.contains("hints_omitted=\"1\""));
        assert!(rendered.contains("noise_omitted=\"1\""));
        assert!(rendered.contains("slot=\"profile.fact.5\""));
        assert!(!rendered.contains("slot=\"profile.fact.6\""));
        assert!(rendered.contains("slot=\"profile.hint.3\""));
        assert!(!rendered.contains("slot=\"profile.hint.4\""));
    }

    #[test]
    fn render_contradicted_item_has_xml_attributes() {
        let mut bundle = ContextBundle::default();
        bundle
            .facts
            .push(item("profile.name", "Aster", GroundingTier::Fact, true));

        let rendered = render_grounding_contract(&bundle);
        assert!(rendered.contains("<contradicted note=\"verify before using\">"));
        assert!(rendered.contains("id=\"C1\""));
        assert!(rendered.contains("contradicted=\"true\""));
    }

    #[test]
    fn render_citation_ids_are_sequential() {
        let mut bundle = ContextBundle::default();
        bundle
            .facts
            .push(item("profile.name", "Aster", GroundingTier::Fact, false));
        bundle
            .facts
            .push(item("profile.age", "25", GroundingTier::Fact, false));

        let rendered = render_grounding_contract(&bundle);
        assert!(rendered.contains("id=\"F1\""));
        assert!(rendered.contains("id=\"F2\""));
    }

    #[test]
    fn render_xml_escapes_special_characters() {
        let mut bundle = ContextBundle::default();
        bundle
            .facts
            .push(item("note", "x < y & z > w", GroundingTier::Fact, false));

        let rendered = render_grounding_contract(&bundle);
        assert!(rendered.contains("x &lt; y &amp; z &gt; w"));
        assert!(!rendered.contains("x < y"));
    }

    #[test]
    fn render_xml_escapes_slot_attributes() {
        let mut bundle = ContextBundle::default();
        bundle.facts.push(item(
            "profile.\"bad\"&<slot>",
            "safe value",
            GroundingTier::Fact,
            false,
        ));

        let rendered = render_grounding_contract(&bundle);

        assert!(rendered.contains("slot=\"profile.&quot;bad&quot;&amp;&lt;slot&gt;\""));
        assert!(!rendered.contains("slot=\"profile.\"bad\""));
    }

    #[test]
    fn render_low_relevance_warning() {
        let mut bundle = ContextBundle::default();
        bundle.facts.push(GroundingEntry {
            recall_score: 0.3,
            ..item("profile.name", "Aster", GroundingTier::Fact, false)
        });
        bundle.hints.push(GroundingEntry {
            recall_score: 0.2,
            ..item("misc.note", "something", GroundingTier::Hint, false)
        });

        let rendered = render_grounding_contract(&bundle);
        assert!(rendered.contains("<retrieval-warning>"));
    }

    #[test]
    fn render_no_warning_when_scores_high() {
        let mut bundle = ContextBundle::default();
        bundle
            .facts
            .push(item("profile.name", "Aster", GroundingTier::Fact, false));

        let rendered = render_grounding_contract(&bundle);
        assert!(!rendered.contains("<retrieval-warning>"));
    }

    /// End-to-end test: MemoryRecallEntry → build_context_bundle → render_grounding_contract.
    /// Mirrors the production path in pipeline.rs::build_grounding_block().
    #[test]
    fn end_to_end_recall_to_xml_grounding() {
        use crate::contracts::ids::SlotKey;
        use crate::core::memory::{MemoryRecallEntry, PrivacyLevel};

        let recall_items = vec![
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.name"),
                value: "Haru".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.95.into(),
                importance: 0.8.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.9,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("interest.hobby"),
                value: "Rust programming".to_string(),
                source: MemorySource::Inferred,
                confidence: 0.55.into(),
                importance: 0.4.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.7,
                occurred_at: "2026-03-15T00:00:00Z".to_string(),
            },
        ];

        let rendered = super::build_grounding_augmentation_block(&recall_items);

        // Verify XML structure
        assert!(rendered.starts_with("<grounding>\n"));
        assert!(rendered.ends_with("</grounding>\n"));
        assert!(rendered.contains("<instruction>"));
        assert!(rendered.contains("<facts>"));
        assert!(rendered.contains("<hints note=\"treat as uncertain\">"));

        // Verify citation IDs
        assert!(rendered.contains("id=\"F1\""));
        assert!(rendered.contains("id=\"H1\""));

        // Verify content
        assert!(rendered.contains(">Haru</item>"));
        assert!(rendered.contains(">Rust programming</item>"));

        // Verify metadata attributes
        assert!(rendered.contains("confidence=\"0.95\""));
        assert!(rendered.contains("source=\"ExplicitUser\""));
        assert!(rendered.contains("slot=\"profile.name\""));
        assert!(rendered.contains("confidence=\"0.55\""));
        assert!(rendered.contains("source=\"Inferred\""));

        // No retrieval warning (scores are 0.9 and 0.7, both > 0.5)
        assert!(!rendered.contains("<retrieval-warning>"));
    }

    #[test]
    fn companion_grounding_block_appends_memory_graph_summary() {
        use crate::contracts::ids::SlotKey;
        use crate::core::memory::{MemoryRecallEntry, PrivacyLevel};

        let recall_items = vec![
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.name"),
                value: "Haru prefers quiet replies".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.95.into(),
                importance: 0.8.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.9,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("continuity.thread"),
                value: "Follow up from last week's noir planning thread".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.9.into(),
                importance: 0.8.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.88,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("external.web.summary"),
                value: "ATTACK_PAYLOAD_ALPHA".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.9.into(),
                importance: 0.6.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.4,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
        ];

        let rendered = build_companion_grounding_augmentation_block(
            "continue our noir thread",
            &recall_items,
            0.3,
        );

        assert!(rendered.contains("<grounding>"));
        assert!(rendered.contains("[Companion Memory Graph]"));
        assert!(rendered.contains("User focus:"));
        assert!(rendered.contains("Continuity:"));
        assert!(rendered.contains("[external payload omitted by replay-ban policy]"));
    }

    #[test]
    fn companion_grounding_block_suppresses_secret_items_and_reports_exposure() {
        use crate::contracts::ids::SlotKey;
        use crate::core::memory::{MemoryRecallEntry, PrivacyLevel};

        let recall_items = vec![
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.public_name"),
                value: "Haru".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.95.into(),
                importance: 0.8.into(),
                privacy_level: PrivacyLevel::Public,
                score: 0.9,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.private_note"),
                value: "prefers quiet replies".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.9.into(),
                importance: 0.8.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.88,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.secret_token"),
                value: "do-not-render-secret".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.99.into(),
                importance: 1.0.into(),
                privacy_level: PrivacyLevel::Secret,
                score: 0.99,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
        ];

        let rendered = build_companion_grounding_augmentation_block("profile", &recall_items, 0.3);

        assert!(rendered.contains("<exposure-rail"));
        assert!(rendered.contains("public_visible=\"1\""));
        assert!(rendered.contains("private_internal=\"1\""));
        assert!(rendered.contains("secret_suppressed=\"1\""));
        assert!(rendered.contains(">Haru</item>"));
        assert!(rendered.contains(">prefers quiet replies</item>"));
        assert!(!rendered.contains("do-not-render-secret"));
        assert!(!rendered.contains("profile.secret_token"));
    }

    #[test]
    fn companion_grounding_updates_exposure_monitor_snapshot() {
        use crate::contracts::ids::SlotKey;
        use crate::core::memory::{MemoryRecallEntry, PrivacyLevel};

        let before = grounding_exposure_monitor_snapshot();
        let recall_items = vec![
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.public_name"),
                value: "Haru".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.95.into(),
                importance: 0.8.into(),
                privacy_level: PrivacyLevel::Public,
                score: 0.9,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
            MemoryRecallEntry {
                entity_id: EntityId::new("user-1"),
                slot_key: SlotKey::new("profile.secret_token"),
                value: "do-not-render-secret".to_string(),
                source: MemorySource::ExplicitUser,
                confidence: 0.99.into(),
                importance: 1.0.into(),
                privacy_level: PrivacyLevel::Secret,
                score: 0.99,
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
            },
        ];

        let augmentation = build_companion_grounding_augmentation("profile", &recall_items, 0.3);
        let snapshot = grounding_exposure_monitor_snapshot();

        assert_eq!(augmentation.exposure.public_visible, 1);
        assert_eq!(augmentation.exposure.secret_suppressed, 1);
        assert!(snapshot.observed_builds >= before.observed_builds + 1);
        assert!(snapshot.public_visible_total >= before.public_visible_total + 1);
        assert!(snapshot.secret_suppressed_total >= before.secret_suppressed_total + 1);
    }
}
