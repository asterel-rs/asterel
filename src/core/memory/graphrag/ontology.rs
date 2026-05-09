//! Domain ontology definitions for `GraphRAG` extraction.
//!
//! An [`OntologyDefinition`] specifies the allowed entity types
//! ([`OntologyEntityType`]) and relation types ([`OntologyRelationType`]) for
//! a given extraction domain. The default is the `companion_memory_ontology`,
//! covering user / room-context / identity / session-working / topic /
//! continuity nouns for the companion runtime.
//!
//! References: [DL-HANDBOOK] Baader et al., 2007; [OWL] Horrocks et al., 2003.
//! See the public research reference index in the docs site.

use serde::{Deserialize, Serialize};

/// Constrained graph entity types for companion memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologyEntityType {
    User,
    RoomContext,
    Identity,
    SessionWorking,
    Topic,
    Continuity,
    Preference,
    Relationship,
}

impl OntologyEntityType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::RoomContext => "room_context",
            Self::Identity => "identity",
            Self::SessionWorking => "session_working",
            Self::Topic => "topic",
            Self::Continuity => "continuity",
            Self::Preference => "preference",
            Self::Relationship => "relationship",
        }
    }
}

/// Allowed relation types for companion memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologyRelationType {
    ParticipatesIn,
    Prefers,
    Discusses,
    Reflects,
    ContinuesFrom,
    OccursIn,
    Mentions,
    Contradicts,
}

impl OntologyRelationType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParticipatesIn => "participates_in",
            Self::Prefers => "prefers",
            Self::Discusses => "discusses",
            Self::Reflects => "reflects",
            Self::ContinuesFrom => "continues_from",
            Self::OccursIn => "occurs_in",
            Self::Mentions => "mentions",
            Self::Contradicts => "contradicts",
        }
    }
}

pub const COMPANION_MEMORY_ENTITY_TYPES: [OntologyEntityType; 8] = [
    OntologyEntityType::User,
    OntologyEntityType::RoomContext,
    OntologyEntityType::Identity,
    OntologyEntityType::SessionWorking,
    OntologyEntityType::Topic,
    OntologyEntityType::Continuity,
    OntologyEntityType::Preference,
    OntologyEntityType::Relationship,
];

pub const COMPANION_MEMORY_RELATION_TYPES: [OntologyRelationType; 8] = [
    OntologyRelationType::ParticipatesIn,
    OntologyRelationType::Prefers,
    OntologyRelationType::Discusses,
    OntologyRelationType::Reflects,
    OntologyRelationType::ContinuesFrom,
    OntologyRelationType::OccursIn,
    OntologyRelationType::Mentions,
    OntologyRelationType::Contradicts,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyDefinition {
    pub entity_types: Vec<OntologyEntityType>,
    pub relation_types: Vec<OntologyRelationType>,
}

impl OntologyDefinition {
    #[must_use]
    pub fn supports_entity_type(&self, entity_type: OntologyEntityType) -> bool {
        self.entity_types.contains(&entity_type)
    }

    #[must_use]
    pub fn supports_relation_type(&self, relation_type: OntologyRelationType) -> bool {
        self.relation_types.contains(&relation_type)
    }

    #[must_use]
    pub fn entity_type_names(&self) -> Vec<&'static str> {
        self.entity_types
            .iter()
            .map(|entity_type| entity_type.as_str())
            .collect()
    }

    #[must_use]
    pub fn relation_type_names(&self) -> Vec<&'static str> {
        self.relation_types
            .iter()
            .map(|relation_type| relation_type.as_str())
            .collect()
    }
}

/// Return the companion-first ontology used by default in the main runtime.
#[must_use]
pub fn companion_memory_ontology() -> OntologyDefinition {
    OntologyDefinition {
        entity_types: COMPANION_MEMORY_ENTITY_TYPES.to_vec(),
        relation_types: COMPANION_MEMORY_RELATION_TYPES.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_memory_ontology_is_constrained() {
        let ontology = companion_memory_ontology();
        assert!(ontology.supports_entity_type(OntologyEntityType::User));
        assert!(ontology.supports_relation_type(OntologyRelationType::ParticipatesIn));
        assert_eq!(ontology.entity_types.len(), 8);
    }

    #[test]
    fn companion_memory_type_names_match_cutover_rules() {
        let ontology = companion_memory_ontology();
        assert_eq!(
            ontology.entity_type_names(),
            vec![
                "user",
                "room_context",
                "identity",
                "session_working",
                "topic",
                "continuity",
                "preference",
                "relationship",
            ]
        );
        assert_eq!(
            ontology.relation_type_names(),
            vec![
                "participates_in",
                "prefers",
                "discusses",
                "reflects",
                "continues_from",
                "occurs_in",
                "mentions",
                "contradicts",
            ]
        );
    }
}
