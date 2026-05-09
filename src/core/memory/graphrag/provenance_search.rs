//! Temporal provenance search: indexes extracted relations by validity window
//! and enables time-point and time-range queries over the extracted graph.
//!
//! A [`TemporalGraphRelation`] wraps an [`ExtractedRelation`] with optional
//! `valid_from`/`valid_until` bounds. The [`TemporalProvenanceIndex`] supports
//! querying which relations were active at a given instant or within a range.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::entity_resolution::EntityAliasRecord;
use super::extraction::{ExtractedEntity, ExtractedRelation};
use super::ontology::OntologyRelationType;
use crate::contracts::ids::EvidenceId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphRelation {
    pub relation: ExtractedRelation,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
}

impl From<ExtractedRelation> for TemporalGraphRelation {
    fn from(relation: ExtractedRelation) -> Self {
        Self {
            valid_from: relation.valid_from,
            valid_until: relation.valid_until,
            relation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceContradiction {
    pub entity_key: String,
    pub counterpart_entity_key: String,
    pub relation: ExtractedRelation,
    pub conflicting_relation: Option<ExtractedRelation>,
    pub rationale: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceSearchIndex {
    entities_by_key: HashMap<String, ExtractedEntity>,
    relations: Vec<ExtractedRelation>,
    aliases: Vec<EntityAliasRecord>,
}

impl ProvenanceSearchIndex {
    #[must_use]
    pub fn new(
        entities: Vec<ExtractedEntity>,
        relations: Vec<ExtractedRelation>,
        aliases: Vec<EntityAliasRecord>,
    ) -> Self {
        let entities_by_key = entities
            .into_iter()
            .map(|entity| (entity.entity_key.clone(), entity))
            .collect();
        Self {
            entities_by_key,
            relations,
            aliases,
        }
    }

    #[must_use]
    pub fn find_entities_by_evidence(&self, evidence_id: &EvidenceId) -> Vec<ExtractedEntity> {
        let mut entity_keys: HashSet<String> = self
            .entities_by_key
            .values()
            .filter(|entity| entity.evidence_ids.iter().any(|id| id == evidence_id))
            .map(|entity| entity.entity_key.clone())
            .collect();

        for relation in &self.relations {
            if relation.evidence_ids.iter().any(|id| id == evidence_id) {
                entity_keys.insert(relation.source_entity_key.clone());
                entity_keys.insert(relation.target_entity_key.clone());
            }
        }

        let mut entities: Vec<_> = entity_keys
            .into_iter()
            .filter_map(|entity_key| self.entities_by_key.get(&entity_key).cloned())
            .collect();
        entities.sort_by(|lhs, rhs| lhs.entity_key.cmp(&rhs.entity_key));
        entities
    }

    #[must_use]
    pub fn find_relations_in_time_range(
        &self,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Vec<TemporalGraphRelation> {
        if from > until {
            return Vec::new();
        }

        let mut relations: Vec<_> = self
            .relations
            .iter()
            .filter(|relation| windows_overlap(relation, from, until))
            .cloned()
            .map(TemporalGraphRelation::from)
            .collect();
        relations.sort_by(|lhs, rhs| {
            relation_sort_key(&lhs.relation).cmp(&relation_sort_key(&rhs.relation))
        });
        relations
    }

    #[must_use]
    pub fn find_active_relations_in_time_range(
        &self,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Vec<TemporalGraphRelation> {
        if from > until {
            return Vec::new();
        }

        let mut relations: Vec<_> = self
            .relations
            .iter()
            .filter(|relation| {
                is_active_relation(relation) && windows_overlap(relation, from, until)
            })
            .cloned()
            .map(TemporalGraphRelation::from)
            .collect();
        relations.sort_by(|lhs, rhs| {
            relation_sort_key(&lhs.relation).cmp(&relation_sort_key(&rhs.relation))
        });
        relations
    }

    #[must_use]
    pub fn find_contradictions(&self, entity_key: &str) -> Vec<ProvenanceContradiction> {
        let resolved_keys = self.resolved_entity_keys(entity_key);
        let relevant: Vec<&ExtractedRelation> = self
            .relations
            .iter()
            .filter(|relation| {
                resolved_keys.contains(&relation.source_entity_key)
                    || resolved_keys.contains(&relation.target_entity_key)
            })
            .collect();

        let mut contradictions = Vec::new();
        let mut seen = HashSet::new();

        for relation in &relevant {
            if relation.relation_type == OntologyRelationType::Contradicts {
                let counterpart = counterpart_entity_key(relation, entity_key);
                let signature = contradiction_signature(entity_key, &counterpart, relation, None);
                if seen.insert(signature) {
                    contradictions.push(ProvenanceContradiction {
                        entity_key: entity_key.to_string(),
                        counterpart_entity_key: counterpart,
                        relation: (*relation).clone(),
                        conflicting_relation: None,
                        rationale: "explicit contradicts edge".to_string(),
                    });
                }
            }
        }

        for (index, left) in relevant.iter().enumerate() {
            for right in relevant.iter().skip(index + 1) {
                if !relations_conflict(left, right, entity_key) {
                    continue;
                }

                let counterpart = shared_counterpart(left, right, entity_key);
                let signature =
                    contradiction_signature(entity_key, &counterpart, left, Some(right));
                if seen.insert(signature) {
                    contradictions.push(ProvenanceContradiction {
                        entity_key: entity_key.to_string(),
                        counterpart_entity_key: counterpart,
                        relation: (*left).clone(),
                        conflicting_relation: Some((*right).clone()),
                        rationale: format!(
                            "conflicting {} and {} relations in overlapping validity windows",
                            left.relation_type.as_str(),
                            right.relation_type.as_str()
                        ),
                    });
                }
            }
        }

        contradictions.sort_by_key(contradiction_sort_key);
        contradictions
    }

    fn resolved_entity_keys(&self, entity_key: &str) -> HashSet<String> {
        let mut keys = HashSet::from([entity_key.to_string()]);
        for alias in &self.aliases {
            if alias.canonical_entity_key == entity_key {
                keys.insert(alias.alias.clone());
            }
        }
        keys
    }
}

pub(crate) fn active_edge_filter_sql() -> &'static str {
    "AND valid_until IS NULL"
}

fn is_active_relation(relation: &ExtractedRelation) -> bool {
    let _ = active_edge_filter_sql();
    relation.valid_until.is_none()
}

fn windows_overlap(
    relation: &ExtractedRelation,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> bool {
    relation
        .valid_from
        .is_none_or(|valid_from| valid_from <= until)
        && relation
            .valid_until
            .is_none_or(|valid_until| valid_until >= from)
}

fn relation_sort_key(
    relation: &ExtractedRelation,
) -> (Option<DateTime<Utc>>, String, String, String) {
    (
        relation.valid_from,
        relation.source_entity_key.clone(),
        relation.target_entity_key.clone(),
        relation.relation_type.as_str().to_string(),
    )
}

fn contradiction_sort_key(
    contradiction: &ProvenanceContradiction,
) -> (String, String, String, String) {
    (
        contradiction.counterpart_entity_key.clone(),
        contradiction.relation.source_entity_key.clone(),
        contradiction.relation.target_entity_key.clone(),
        contradiction.relation.relation_type.as_str().to_string(),
    )
}

fn relations_conflict(
    left: &ExtractedRelation,
    right: &ExtractedRelation,
    entity_key: &str,
) -> bool {
    if !overlapping_relation_scope(left, right, entity_key) {
        return false;
    }

    (left.relation_type == OntologyRelationType::Contradicts
        && right.relation_type != OntologyRelationType::Contradicts)
        || (right.relation_type == OntologyRelationType::Contradicts
            && left.relation_type != OntologyRelationType::Contradicts)
}

fn overlapping_relation_scope(
    left: &ExtractedRelation,
    right: &ExtractedRelation,
    entity_key: &str,
) -> bool {
    !shared_counterpart(left, right, entity_key).is_empty() && relation_windows_overlap(left, right)
}

fn relation_windows_overlap(left: &ExtractedRelation, right: &ExtractedRelation) -> bool {
    left.valid_from.is_none_or(|valid_from| {
        right
            .valid_until
            .is_none_or(|valid_until| valid_from <= valid_until)
    }) && right.valid_from.is_none_or(|valid_from| {
        left.valid_until
            .is_none_or(|valid_until| valid_from <= valid_until)
    })
}

fn shared_counterpart(
    left: &ExtractedRelation,
    right: &ExtractedRelation,
    entity_key: &str,
) -> String {
    let left_counterpart = counterpart_entity_key(left, entity_key);
    let right_counterpart = counterpart_entity_key(right, entity_key);
    if left_counterpart == right_counterpart {
        left_counterpart
    } else {
        String::new()
    }
}

fn counterpart_entity_key(relation: &ExtractedRelation, entity_key: &str) -> String {
    if relation.source_entity_key == entity_key {
        relation.target_entity_key.clone()
    } else {
        relation.source_entity_key.clone()
    }
}

fn contradiction_signature(
    entity_key: &str,
    counterpart: &str,
    left: &ExtractedRelation,
    right: Option<&ExtractedRelation>,
) -> String {
    let left_signature = relation_signature(left);
    let right_signature = right.map_or(String::new(), relation_signature);
    if left_signature <= right_signature {
        format!("{entity_key}|{counterpart}|{left_signature}|{right_signature}")
    } else {
        format!("{entity_key}|{counterpart}|{right_signature}|{left_signature}")
    }
}

fn relation_signature(relation: &ExtractedRelation) -> String {
    format!(
        "{}|{}|{}|{:?}|{:?}|{}",
        relation.source_entity_key,
        relation.target_entity_key,
        relation.relation_type.as_str(),
        relation.valid_from,
        relation.valid_until,
        relation.fact
    )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::core::memory::graphrag::ontology::{OntologyEntityType, OntologyRelationType};

    fn entity(entity_key: &str, name: &str, evidence_ids: &[&str]) -> ExtractedEntity {
        ExtractedEntity {
            entity_key: entity_key.to_string(),
            canonical_name: name.to_string(),
            entity_type: OntologyEntityType::User,
            aliases: Vec::new(),
            summary: format!("summary for {name}"),
            evidence_ids: evidence_ids.iter().map(|id| EvidenceId::new(*id)).collect(),
        }
    }

    fn relation(
        source_entity_key: &str,
        target_entity_key: &str,
        relation_type: OntologyRelationType,
        evidence_ids: &[&str],
        valid_from: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
        fact: &str,
    ) -> ExtractedRelation {
        ExtractedRelation {
            source_entity_key: source_entity_key.to_string(),
            target_entity_key: target_entity_key.to_string(),
            relation_type,
            fact: fact.to_string(),
            confidence: 0.9,
            valid_from,
            valid_until,
            evidence_ids: evidence_ids.iter().map(|id| EvidenceId::new(*id)).collect(),
        }
    }

    fn timestamp(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn provenance_search_finds_entities_by_evidence_across_entities_and_relations() {
        let index = ProvenanceSearchIndex::new(
            vec![
                entity("stakeholder_partners", "Partners", &["doc-1"]),
                entity("area_api_v2", "API v2", &[]),
                entity("risk_deadline", "Deadline risk", &[]),
            ],
            vec![relation(
                "stakeholder_partners",
                "area_api_v2",
                OntologyRelationType::Mentions,
                &["doc-1"],
                Some(timestamp(2026, 1, 1)),
                Some(timestamp(2026, 6, 30)),
                "Partners mention the migration timeline.",
            )],
            Vec::new(),
        );

        let entities = index.find_entities_by_evidence(&EvidenceId::new("doc-1"));

        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].entity_key, "area_api_v2");
        assert_eq!(entities[1].entity_key, "stakeholder_partners");
    }

    #[test]
    fn provenance_search_finds_relations_in_overlapping_time_range() {
        let index = ProvenanceSearchIndex::new(
            vec![entity("stakeholder_partners", "Partners", &[])],
            vec![
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::ParticipatesIn,
                    &["doc-1"],
                    Some(timestamp(2026, 1, 1)),
                    Some(timestamp(2026, 3, 31)),
                    "Partners participate in the first room phase.",
                ),
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::ContinuesFrom,
                    &["doc-2"],
                    Some(timestamp(2026, 7, 1)),
                    Some(timestamp(2026, 9, 30)),
                    "The thread continues from a later archive without tooling.",
                ),
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::Mentions,
                    &["doc-3"],
                    None,
                    None,
                    "Partners continuously mention migration updates.",
                ),
            ],
            Vec::new(),
        );

        let relations =
            index.find_relations_in_time_range(timestamp(2026, 2, 1), timestamp(2026, 4, 1));

        assert_eq!(relations.len(), 2);
        assert!(
            relations.iter().any(|entry| {
                entry.relation.relation_type == OntologyRelationType::ParticipatesIn
            })
        );
        assert!(
            relations
                .iter()
                .any(|entry| entry.relation.relation_type == OntologyRelationType::Mentions)
        );
    }

    #[test]
    fn provenance_search_finds_explicit_and_temporal_contradictions() {
        let index = ProvenanceSearchIndex::new(
            vec![
                entity("stakeholder_partners", "Partners", &[]),
                entity("area_api_v2", "API v2", &[]),
            ],
            vec![
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::ParticipatesIn,
                    &["doc-1"],
                    Some(timestamp(2026, 1, 1)),
                    Some(timestamp(2026, 6, 30)),
                    "Partners participate in the migration thread.",
                ),
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::Contradicts,
                    &["doc-2"],
                    Some(timestamp(2026, 3, 1)),
                    Some(timestamp(2026, 7, 31)),
                    "Partners contradict the migration thread if auth changes remain undocumented.",
                ),
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::Contradicts,
                    &["doc-3"],
                    None,
                    None,
                    "Partner readiness contradicts the current rollout plan.",
                ),
            ],
            Vec::new(),
        );

        let contradictions = index.find_contradictions("stakeholder_partners");

        assert!(contradictions.len() >= 2);
        assert!(
            contradictions
                .iter()
                .any(|entry| entry.rationale == "explicit contradicts edge")
        );
        assert!(
            contradictions
                .iter()
                .any(|entry| entry.conflicting_relation.is_some())
        );
    }

    #[test]
    fn default_provenance_search_excludes_invalidated_edges() {
        let index = ProvenanceSearchIndex::new(
            vec![
                entity("stakeholder_partners", "Partners", &[]),
                entity("area_api_v2", "API v2", &[]),
            ],
            vec![
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::ParticipatesIn,
                    &["doc-1"],
                    Some(timestamp(2026, 1, 1)),
                    None,
                    "Partners participate in the current rollout.",
                ),
                relation(
                    "stakeholder_partners",
                    "area_api_v2",
                    OntologyRelationType::Contradicts,
                    &["doc-2"],
                    Some(timestamp(2026, 1, 1)),
                    Some(timestamp(2026, 2, 1)),
                    "Partners previously contradicted the rollout summary.",
                ),
            ],
            Vec::new(),
        );

        let contradictions =
            index.find_active_relations_in_time_range(timestamp(2026, 1, 1), timestamp(2026, 3, 1));

        assert_eq!(contradictions.len(), 1);
        assert!(contradictions[0].valid_until.is_none());
        assert_eq!(active_edge_filter_sql(), "AND valid_until IS NULL");
    }

    #[test]
    fn history_search_includes_invalidated_edges() {
        let index = ProvenanceSearchIndex::new(
            vec![entity("stakeholder_partners", "Partners", &[])],
            vec![relation(
                "stakeholder_partners",
                "area_api_v2",
                OntologyRelationType::ParticipatesIn,
                &["doc-1"],
                Some(timestamp(2026, 1, 1)),
                Some(timestamp(2026, 1, 15)),
                "Partners participated in the early rollout window.",
            )],
            Vec::new(),
        );

        let relations =
            index.find_relations_in_time_range(timestamp(2026, 1, 10), timestamp(2026, 1, 20));

        assert_eq!(relations.len(), 1);
        assert!(relations[0].valid_until.is_some());
    }
}
