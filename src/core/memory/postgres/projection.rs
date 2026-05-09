//! Knowledge graph projection for `PostgreSQL` memory events.
//!
//! Every `remember` call that writes or replaces a slot also projects the
//! event into the `graph_entities` / `graph_relations` tables. The projection
//! creates or refreshes companion-memory graph nodes:
//!
//! - **person node** — one per `entity_id`; always upserted.
//! - **slot node** — one per `(entity_id, slot_key)` for replace-mode writes.
//! - **event node** — one per memory event, linked to the person node.
//! - **companion anchor node** — optional per owner + companion category
//!   (`user`, `room_context`, `identity`, `session_working`, `topic`,
//!   `continuity`, `preference`, `relationship`).
//!
//! Generic edges (`has_slot`, `recorded_event`, `supersedes`) are preserved,
//! but default companion-memory slots also receive category anchors and
//! category relations so recall and PPR can spread over companion-continuity
//! semantics instead of a purely generic person/slot/event graph.

use sqlx_core::query::query;

use super::error::{PostgresMemoryResult, PostgresMemoryResultExt};
use crate::contracts::memory::MemoryLayer;
use crate::core::memory::codec;
use crate::core::memory::graphrag::{OntologyEntityType, OntologyRelationType};
use crate::core::memory::traits::{MemoryEventInput, MemoryEventType, NodeTier, SignalTier};

type PgTx<'a> = sqlx_core::transaction::Transaction<'a, sqlx_postgres::Postgres>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionProjectionKind {
    User,
    RoomContext,
    Identity,
    SessionWorking,
    Topic,
    Continuity,
    Preference,
    Relationship,
}

impl CompanionProjectionKind {
    const fn ontology_type(self) -> OntologyEntityType {
        match self {
            Self::User => OntologyEntityType::User,
            Self::RoomContext => OntologyEntityType::RoomContext,
            Self::Identity => OntologyEntityType::Identity,
            Self::SessionWorking => OntologyEntityType::SessionWorking,
            Self::Topic => OntologyEntityType::Topic,
            Self::Continuity => OntologyEntityType::Continuity,
            Self::Preference => OntologyEntityType::Preference,
            Self::Relationship => OntologyEntityType::Relationship,
        }
    }

    const fn relation_type(self) -> OntologyRelationType {
        match self {
            Self::RoomContext => OntologyRelationType::ParticipatesIn,
            Self::Preference => OntologyRelationType::Prefers,
            Self::Topic => OntologyRelationType::Discusses,
            Self::Identity => OntologyRelationType::Reflects,
            Self::Continuity => OntologyRelationType::ContinuesFrom,
            Self::SessionWorking => OntologyRelationType::OccursIn,
            Self::User | Self::Relationship => OntologyRelationType::Mentions,
        }
    }

    const fn anchor_label(self) -> &'static str {
        match self {
            Self::User => "User focus",
            Self::RoomContext => "Room context",
            Self::Identity => "Identity",
            Self::SessionWorking => "Session working",
            Self::Topic => "Topic",
            Self::Continuity => "Continuity",
            Self::Preference => "Preference",
            Self::Relationship => "Relationship",
        }
    }

    const fn edge_weight(self) -> f64 {
        match self {
            Self::Continuity => 1.0,
            Self::Identity | Self::Preference => 0.9,
            Self::Topic | Self::Relationship => 0.85,
            Self::RoomContext | Self::SessionWorking | Self::User => 0.8,
        }
    }

    fn anchor_graph_id(self, owner_entity_id: &str) -> String {
        format!(
            "companion_anchor::{owner_entity_id}::{}",
            self.ontology_type().as_str()
        )
    }
}

/// Upsert graph entities and edges for a memory event.
pub(super) async fn upsert_graph_projection(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    event_id: &str,
    now: &str,
    should_replace: bool,
    supersedes_event_id: Option<&str>,
) -> PostgresMemoryResult<()> {
    let source_str = codec::source_to_str(input.source);
    let privacy_str = codec::privacy_to_str(&input.privacy_level);
    let node_tier = node_tier_for_projection(input);

    let person_graph_id = format!("person::{}", input.entity_id);
    upsert_person_graph_entity(
        tx,
        input,
        now,
        source_str,
        privacy_str,
        node_tier,
        &person_graph_id,
    )
    .await?;

    if should_replace {
        let slot_graph_id = format!("slot::{}::{}", input.entity_id, input.slot_key);
        upsert_slot_projection(
            tx,
            input,
            now,
            source_str,
            privacy_str,
            node_tier,
            &person_graph_id,
            &slot_graph_id,
        )
        .await?;
    }

    let event_graph_id = format!("event::{event_id}");
    upsert_event_graph_entity(
        tx,
        input,
        now,
        source_str,
        privacy_str,
        node_tier,
        &event_graph_id,
    )
    .await?;

    let slot_graph_id = format!("slot::{}::{}", input.entity_id, input.slot_key);
    upsert_slot_event_edge(tx, input, event_id, now, &slot_graph_id, &event_graph_id).await?;

    if let Some(prev_event_id) = supersedes_event_id {
        upsert_supersedes_projection(tx, input, event_id, prev_event_id, now, &event_graph_id)
            .await?;
    }

    Ok(())
}

async fn upsert_person_graph_entity(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    now: &str,
    source_str: &str,
    privacy_str: &str,
    node_tier: NodeTier,
    person_graph_id: &str,
) -> PostgresMemoryResult<()> {
    query(
        "INSERT INTO graph_entities ( \
            graph_entity_id, owner_entity_id, entity_type, label, value, \
            source, confidence, importance, node_tier, privacy_level, updated_at \
         ) VALUES ($1, $2, 'person', $3, $4, $5, $6, $7, $8, $9, $10::timestamptz) \
         ON CONFLICT (graph_entity_id) DO UPDATE SET \
             value = EXCLUDED.value, \
             source = EXCLUDED.source, \
             confidence = EXCLUDED.confidence, \
             importance = EXCLUDED.importance, \
             node_tier = EXCLUDED.node_tier, \
             privacy_level = EXCLUDED.privacy_level, \
             updated_at = EXCLUDED.updated_at",
    )
    .bind(person_graph_id)
    .bind(input.entity_id.as_str())
    .bind(input.entity_id.as_str())
    .bind(input.entity_id.as_str())
    .bind(source_str)
    .bind(input.confidence.get())
    .bind(input.importance.get())
    .bind(node_tier_sql(node_tier))
    .bind(privacy_str)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert person graph entity")?;

    Ok(())
}

async fn upsert_slot_projection(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    now: &str,
    source_str: &str,
    privacy_str: &str,
    node_tier: NodeTier,
    person_graph_id: &str,
    slot_graph_id: &str,
) -> PostgresMemoryResult<()> {
    let companion_kind = classify_companion_slot(input.slot_key.as_str());
    let companion_ontology_type = companion_kind.map(|kind| kind.ontology_type().as_str());

    query(
        "INSERT INTO graph_entities ( \
            graph_entity_id, owner_entity_id, entity_type, label, value, canonical_name, \
            ontology_type, source, confidence, importance, node_tier, privacy_level, updated_at \
         ) VALUES ($1, $2, 'slot', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::timestamptz) \
         ON CONFLICT (graph_entity_id) DO UPDATE SET \
              value = EXCLUDED.value, \
              canonical_name = EXCLUDED.canonical_name, \
              ontology_type = EXCLUDED.ontology_type, \
              source = EXCLUDED.source, \
              confidence = EXCLUDED.confidence, \
              importance = EXCLUDED.importance, \
             node_tier = EXCLUDED.node_tier, \
             privacy_level = EXCLUDED.privacy_level, \
             updated_at = EXCLUDED.updated_at",
    )
    .bind(slot_graph_id)
    .bind(input.entity_id.as_str())
    .bind(input.slot_key.as_str())
    .bind(&input.value)
    .bind(input.slot_key.as_str())
    .bind(companion_ontology_type)
    .bind(source_str)
    .bind(input.confidence.get())
    .bind(input.importance.get())
    .bind(node_tier_sql(node_tier))
    .bind(privacy_str)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert slot graph entity")?;

    let has_slot_edge_id = format!("edge::has_slot::{}::{}", input.entity_id, input.slot_key);
    query(
        "INSERT INTO graph_edges ( \
            graph_edge_id, owner_entity_id, from_entity_id, to_entity_id, \
            relation_type, weight, created_at \
         ) VALUES ($1, $2, $3, $4, 'has_slot', 1.0, $5::timestamptz) \
         ON CONFLICT (owner_entity_id, from_entity_id, to_entity_id, relation_type) \
         DO UPDATE SET weight = EXCLUDED.weight",
    )
    .bind(&has_slot_edge_id)
    .bind(input.entity_id.as_str())
    .bind(person_graph_id)
    .bind(slot_graph_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert has_slot edge")?;

    if let Some(kind) = companion_kind {
        upsert_companion_anchor_projection(
            tx,
            input,
            now,
            source_str,
            privacy_str,
            node_tier,
            slot_graph_id,
            kind,
        )
        .await?;
    }

    Ok(())
}

async fn upsert_companion_anchor_projection(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    now: &str,
    source_str: &str,
    privacy_str: &str,
    node_tier: NodeTier,
    slot_graph_id: &str,
    kind: CompanionProjectionKind,
) -> PostgresMemoryResult<()> {
    let anchor_graph_id = kind.anchor_graph_id(input.entity_id.as_str());
    let anchor_label = kind.anchor_label();
    let ontology_type = kind.ontology_type().as_str();

    query(
        "INSERT INTO graph_entities ( \
            graph_entity_id, owner_entity_id, entity_type, label, value, canonical_name, \
            ontology_type, source, confidence, importance, node_tier, privacy_level, updated_at \
         ) VALUES ($1, $2, 'companion_anchor', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::timestamptz) \
         ON CONFLICT (graph_entity_id) DO UPDATE SET \
             label = EXCLUDED.label, \
             value = EXCLUDED.value, \
             canonical_name = EXCLUDED.canonical_name, \
             ontology_type = EXCLUDED.ontology_type, \
             source = EXCLUDED.source, \
             confidence = EXCLUDED.confidence, \
             importance = EXCLUDED.importance, \
             node_tier = EXCLUDED.node_tier, \
             privacy_level = EXCLUDED.privacy_level, \
             updated_at = EXCLUDED.updated_at",
    )
    .bind(&anchor_graph_id)
    .bind(input.entity_id.as_str())
    .bind(anchor_label)
    .bind(anchor_label)
    .bind(anchor_label)
    .bind(ontology_type)
    .bind(source_str)
    .bind(input.confidence.get())
    .bind(input.importance.get())
    .bind(node_tier_sql(node_tier))
    .bind(privacy_str)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert companion anchor graph entity")?;

    upsert_companion_anchor_edge(tx, input, now, &anchor_graph_id, slot_graph_id, kind, true)
        .await?;
    upsert_companion_anchor_edge(tx, input, now, slot_graph_id, &anchor_graph_id, kind, false)
        .await?;

    Ok(())
}

async fn upsert_companion_anchor_edge(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    now: &str,
    from_entity_id: &str,
    to_entity_id: &str,
    kind: CompanionProjectionKind,
    towards_slot: bool,
) -> PostgresMemoryResult<()> {
    let relation_type = kind.relation_type().as_str();
    let direction = if towards_slot {
        "anchor_to_slot"
    } else {
        "slot_to_anchor"
    };
    let edge_id = format!(
        "edge::{relation_type}::{direction}::{}::{}",
        input.entity_id, input.slot_key
    );

    query(
        "INSERT INTO graph_edges ( \
            graph_edge_id, owner_entity_id, from_entity_id, to_entity_id, \
            relation_type, weight, confidence, created_at \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz) \
         ON CONFLICT (owner_entity_id, from_entity_id, to_entity_id, relation_type) \
         DO UPDATE SET weight = EXCLUDED.weight, confidence = EXCLUDED.confidence",
    )
    .bind(&edge_id)
    .bind(input.entity_id.as_str())
    .bind(from_entity_id)
    .bind(to_entity_id)
    .bind(relation_type)
    .bind(kind.edge_weight())
    .bind(input.confidence.get())
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert companion anchor edge")?;

    Ok(())
}

async fn upsert_event_graph_entity(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    now: &str,
    source_str: &str,
    privacy_str: &str,
    node_tier: NodeTier,
    event_graph_id: &str,
) -> PostgresMemoryResult<()> {
    query(
        "INSERT INTO graph_entities ( \
            graph_entity_id, owner_entity_id, entity_type, label, value, \
            source, confidence, importance, node_tier, privacy_level, updated_at \
         ) VALUES ($1, $2, 'event', $3, $4, $5, $6, $7, $8, $9, $10::timestamptz) \
         ON CONFLICT (graph_entity_id) DO UPDATE SET \
             value = EXCLUDED.value, \
             source = EXCLUDED.source, \
             confidence = EXCLUDED.confidence, \
             importance = EXCLUDED.importance, \
             node_tier = EXCLUDED.node_tier, \
             privacy_level = EXCLUDED.privacy_level, \
             updated_at = EXCLUDED.updated_at",
    )
    .bind(event_graph_id)
    .bind(input.entity_id.as_str())
    .bind(input.event_type.to_string())
    .bind(&input.value)
    .bind(source_str)
    .bind(input.confidence.get())
    .bind(input.importance.get())
    .bind(node_tier_sql(node_tier))
    .bind(privacy_str)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert event graph entity")?;

    Ok(())
}

async fn upsert_slot_event_edge(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    event_id: &str,
    now: &str,
    slot_graph_id: &str,
    event_graph_id: &str,
) -> PostgresMemoryResult<()> {
    let relation_type = slot_to_event_relation(&input.event_type);
    let edge_id = format!("edge::{relation_type}::{event_id}");
    let weight = input.importance.get();

    query(
        "INSERT INTO graph_edges ( \
            graph_edge_id, owner_entity_id, from_entity_id, to_entity_id, \
            relation_type, weight, event_id, created_at \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz) \
         ON CONFLICT (owner_entity_id, from_entity_id, to_entity_id, relation_type) \
         DO UPDATE SET weight = EXCLUDED.weight, event_id = EXCLUDED.event_id",
    )
    .bind(&edge_id)
    .bind(input.entity_id.as_str())
    .bind(slot_graph_id)
    .bind(event_graph_id)
    .bind(relation_type)
    .bind(weight)
    .bind(event_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert slot-event edge")?;

    Ok(())
}

async fn upsert_supersedes_projection(
    tx: &mut PgTx<'_>,
    input: &MemoryEventInput,
    event_id: &str,
    prev_event_id: &str,
    now: &str,
    event_graph_id: &str,
) -> PostgresMemoryResult<()> {
    let prev_event_graph_id = format!("event::{prev_event_id}");
    query(
        "INSERT INTO graph_entities ( \
            graph_entity_id, owner_entity_id, entity_type, label, value, \
            source, confidence, importance, node_tier, privacy_level, updated_at \
         ) VALUES ($1, $2, 'event', 'superseded', '', 'system', 0.0, 0.0, 'episode', 'public', $3::timestamptz) \
         ON CONFLICT (graph_entity_id) DO NOTHING",
    )
    .bind(&prev_event_graph_id)
    .bind(input.entity_id.as_str())
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("ensure previous event graph entity")?;

    let supersedes_edge_id = format!("edge::supersedes::{event_id}::{prev_event_id}");
    query(
        "INSERT INTO graph_edges ( \
            graph_edge_id, owner_entity_id, from_entity_id, to_entity_id, \
            relation_type, weight, event_id, created_at \
         ) VALUES ($1, $2, $3, $4, 'supersedes', 1.0, $5, $6::timestamptz) \
         ON CONFLICT (owner_entity_id, from_entity_id, to_entity_id, relation_type) \
         DO NOTHING",
    )
    .bind(&supersedes_edge_id)
    .bind(input.entity_id.as_str())
    .bind(event_graph_id)
    .bind(&prev_event_graph_id)
    .bind(event_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .pg_projection("upsert supersedes edge")?;

    Ok(())
}

fn slot_to_event_relation(
    _event_type: &crate::core::memory::traits::MemoryEventType,
) -> &'static str {
    // All event types map to recorded_event for graph edge relations
    "recorded_event"
}

fn node_tier_for_projection(input: &MemoryEventInput) -> NodeTier {
    if matches!(input.event_type, MemoryEventType::SummaryCompacted) {
        return NodeTier::Note;
    }

    if matches!(input.signal_tier, Some(SignalTier::Raw)) || input.source_kind.is_some() {
        return NodeTier::Episode;
    }

    match input.layer {
        MemoryLayer::Semantic if input.source_ref.is_none() => NodeTier::Note,
        _ => NodeTier::Episode,
    }
}

fn node_tier_sql(node_tier: NodeTier) -> &'static str {
    match node_tier {
        NodeTier::Episode => "episode",
        NodeTier::Note => "note",
    }
}

fn classify_companion_slot(slot_key: &str) -> Option<CompanionProjectionKind> {
    let lower = slot_key.to_ascii_lowercase();

    if lower.contains("relationship") || lower.contains("rapport") || lower.contains("trust") {
        return Some(CompanionProjectionKind::Relationship);
    }
    if lower.contains("continuity")
        || lower.contains("narrative")
        || lower.contains("milestone")
        || lower.contains("follow_up")
        || lower.contains("conversation.state")
        || lower.contains("conversation.ledger")
        || lower.starts_with("experience.")
    {
        return Some(CompanionProjectionKind::Continuity);
    }
    if lower.contains("session")
        || lower.contains("conversation.user_msg")
        || lower.contains("conversation.assistant_resp")
        || lower.starts_with("turn_outcome.")
        || lower.starts_with("outcome.")
        || lower.starts_with("trajectory.")
        || lower.contains("rollback")
    {
        return Some(CompanionProjectionKind::SessionWorking);
    }
    if lower.contains("channel")
        || lower.contains("room")
        || lower.contains("guild")
        || lower.contains("server")
        || lower.contains("dm")
    {
        return Some(CompanionProjectionKind::RoomContext);
    }
    if lower.contains("response_style")
        || lower.contains("style_pref")
        || lower.contains("style_profile")
        || lower.contains("language")
        || lower.contains("locale")
        || lower.contains("preference")
        || lower.contains("taste")
    {
        return Some(CompanionProjectionKind::Preference);
    }
    if lower.contains("state_header")
        || lower.contains("self_contract")
        || lower.contains("self_model")
        || lower.contains("big_five")
        || lower.contains("persona.affect")
        || lower.contains("embodied_state")
        || lower.contains("metacognition")
        || lower.starts_with("principle.")
    {
        return Some(CompanionProjectionKind::Identity);
    }
    if lower.contains("topic")
        || lower.contains("project")
        || lower.contains("interest")
        || lower.contains("hobby")
        || lower.contains("world_model")
        || lower.contains("user_knowledge")
    {
        return Some(CompanionProjectionKind::Topic);
    }
    if lower.contains("/user_facts/name")
        || lower.starts_with("user.")
        || lower.contains("profile.name")
    {
        return Some(CompanionProjectionKind::User);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{CompanionProjectionKind, classify_companion_slot, node_tier_for_projection};
    use crate::contracts::memory::MemoryLayer;
    use crate::core::memory::{
        MemoryEventInput, MemoryEventType, MemorySource, NodeTier, PrivacyLevel, SignalTier,
        SourceKind,
    };

    #[test]
    fn projection_marks_summary_compaction_as_note() {
        let input = MemoryEventInput::new(
            "person:alice",
            "semantic.summary",
            MemoryEventType::SummaryCompacted,
            "Alice prefers Rust.",
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Semantic);

        assert_eq!(node_tier_for_projection(&input), NodeTier::Note);
    }

    #[test]
    fn projection_keeps_raw_slot_events_as_episodes() {
        let input = MemoryEventInput::new(
            "person:alice",
            "preference.language",
            MemoryEventType::FactAdded,
            "Rust",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Working)
        .with_signal_tier(SignalTier::Raw)
        .with_source_kind(SourceKind::Conversation);

        assert_eq!(node_tier_for_projection(&input), NodeTier::Episode);
    }

    #[test]
    fn classifies_core_companion_slot_categories() {
        assert_eq!(
            classify_companion_slot("persona/alice/user_facts/name"),
            Some(CompanionProjectionKind::User)
        );
        assert_eq!(
            classify_companion_slot("channel.context"),
            Some(CompanionProjectionKind::RoomContext)
        );
        assert_eq!(
            classify_companion_slot("persona/alice/state_header/v1"),
            Some(CompanionProjectionKind::Identity)
        );
        assert_eq!(
            classify_companion_slot("persona/alice/big_five/v1"),
            Some(CompanionProjectionKind::Identity)
        );
        assert_eq!(
            classify_companion_slot("session.current"),
            Some(CompanionProjectionKind::SessionWorking)
        );
        assert_eq!(
            classify_companion_slot("persona/alice/user_facts/ongoing_projects"),
            Some(CompanionProjectionKind::Topic)
        );
        assert_eq!(
            classify_companion_slot("continuity.thread"),
            Some(CompanionProjectionKind::Continuity)
        );
        assert_eq!(
            classify_companion_slot("persona/alice/user_facts/response_style"),
            Some(CompanionProjectionKind::Preference)
        );
        assert_eq!(
            classify_companion_slot("persona/alice/relationship/v1"),
            Some(CompanionProjectionKind::Relationship)
        );
    }

    #[test]
    fn companion_projection_kind_maps_to_companion_ontology_defaults() {
        let continuity = CompanionProjectionKind::Continuity;
        assert_eq!(continuity.ontology_type().as_str(), "continuity");
        assert_eq!(continuity.relation_type().as_str(), "continues_from");

        let preference = CompanionProjectionKind::Preference;
        assert_eq!(preference.ontology_type().as_str(), "preference");
        assert_eq!(preference.relation_type().as_str(), "prefers");
    }
}
