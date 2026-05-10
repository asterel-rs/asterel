//! LLM-driven knowledge graph extraction from unstructured text.
//!
//! Given a corpus of recall entries and an ontology definition, prompts an LLM
//! to extract typed entities (`OntologyEntityType`) and typed relations
//! (`OntologyRelationType`) with associated evidence IDs and optional
//! `valid_from`/`valid_until` validity windows.
//!
//! Entity resolution (deduplication of co-referent nodes) is delegated to
//! `entity_resolution::EntityResolver` after extraction.
//!
//! References: [REBEL] Cabot & Navigli, 2021; [TEXT2KGBENCH]
//! Mihindukulasooriya et al., 2023. See the public research reference index in
//! the docs site.

use std::collections::HashSet;
use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::entity_resolution::{EntityResolution, EntityResolver};
use super::ontology::{
    OntologyDefinition, OntologyEntityType, OntologyRelationType, companion_memory_ontology,
};
use crate::contracts::ids::EvidenceId;
use crate::core::providers::Provider;

const EXTRACTION_SYSTEM_PROMPT: &str = "You extract a constrained companion-memory graph. Return ONLY JSON with keys `entities` and `relations`. Each entity must include: `entity_key`, `canonical_name`, `entity_type`, `aliases`, `summary`, `evidence_ids`. Each relation must include: `source_entity_key`, `target_entity_key`, `relation_type`, `fact`, `confidence`, `evidence_ids`, and optional `valid_from` / `valid_until` RFC3339 timestamps when the evidence states a validity window. Use only the allowed ontology types and relation types supplied by the user. Keep names concise, preserve source wording for facts, and never invent evidence ids or time ranges.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphExtractionDocument {
    pub evidence_id: EvidenceId,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub entity_key: String,
    pub canonical_name: String,
    pub entity_type: OntologyEntityType,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedRelation {
    pub source_entity_key: String,
    pub target_entity_key: String,
    pub relation_type: OntologyRelationType,
    pub fact: String,
    pub confidence: f64,
    #[serde(default)]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphExtractionResult {
    pub ontology: OntologyDefinition,
    pub documents: Vec<GraphExtractionDocument>,
    pub entities: Vec<ExtractedEntity>,
    pub relations: Vec<ExtractedRelation>,
}

#[derive(Debug, Clone, Copy)]
pub struct GraphExtractionConfig<'a> {
    pub model: &'a str,
    pub temperature: f64,
}

#[derive(Debug, Deserialize)]
struct RawGraphExtraction {
    #[serde(default)]
    entities: Vec<ExtractedEntity>,
    #[serde(default)]
    relations: Vec<ExtractedRelation>,
}

pub struct StructuredExtractionPipeline<'a> {
    provider: &'a dyn Provider,
    config: GraphExtractionConfig<'a>,
}

impl<'a> StructuredExtractionPipeline<'a> {
    #[must_use]
    pub fn new(provider: &'a dyn Provider, config: GraphExtractionConfig<'a>) -> Self {
        Self { provider, config }
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn extract(
        &self,
        documents: &[GraphExtractionDocument],
    ) -> Result<GraphExtractionResult> {
        self.extract_with_ontology(documents, companion_memory_ontology())
            .await
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn extract_with_ontology(
        &self,
        documents: &[GraphExtractionDocument],
        ontology: OntologyDefinition,
    ) -> Result<GraphExtractionResult> {
        if documents.is_empty() {
            bail!("Graph extraction requires at least one document");
        }

        let prompt = build_prompt(&ontology, documents);
        let response = self
            .provider
            .chat_with_system(
                Some(EXTRACTION_SYSTEM_PROMPT),
                &prompt,
                self.config.model,
                self.config.temperature,
            )
            .await
            .context("GraphRAG extraction LLM call failed")?;

        let parsed = parse_response(&response)?;
        validate_extraction(ontology, documents.to_vec(), parsed)
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn extract_and_resolve(
        &self,
        documents: &[GraphExtractionDocument],
        resolver: &EntityResolver<'_>,
    ) -> Result<GraphExtractionResult> {
        self.extract_and_resolve_with_ontology(documents, resolver, companion_memory_ontology())
            .await
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn extract_and_resolve_with_ontology(
        &self,
        documents: &[GraphExtractionDocument],
        resolver: &EntityResolver<'_>,
        ontology: OntologyDefinition,
    ) -> Result<GraphExtractionResult> {
        let extracted = self.extract_with_ontology(documents, ontology).await?;
        let EntityResolution {
            entities,
            relations,
            ..
        } = resolver
            .resolve(extracted.entities, extracted.relations)
            .await?;

        Ok(GraphExtractionResult {
            ontology: extracted.ontology,
            documents: extracted.documents,
            entities,
            relations,
        })
    }
}

fn build_prompt(ontology: &OntologyDefinition, documents: &[GraphExtractionDocument]) -> String {
    let mut prompt = format!(
        "Allowed entity types: {}\nAllowed relation types: {}\n\nDocuments:\n",
        ontology.entity_type_names().join(", "),
        ontology.relation_type_names().join(", ")
    );

    for document in documents {
        let _ = writeln!(
            prompt,
            "- evidence_id: {}\n  title: {}\n  body:\n{}\n",
            document.evidence_id, document.title, document.body
        );
    }

    prompt.push_str(
        "Return only JSON. Every relation endpoint must reference an `entity_key` from `entities`.",
    );
    prompt
}

fn parse_response(response: &str) -> Result<RawGraphExtraction> {
    let trimmed = response.trim();
    let json = trimmed
        .strip_prefix("```json")
        .map(str::trim)
        .and_then(|value| value.strip_suffix("```"))
        .map_or(trimmed, str::trim);
    serde_json::from_str(json).context("parse GraphRAG extraction JSON")
}

fn validate_extraction(
    ontology: OntologyDefinition,
    documents: Vec<GraphExtractionDocument>,
    parsed: RawGraphExtraction,
) -> Result<GraphExtractionResult> {
    let known_evidence: HashSet<EvidenceId> = documents
        .iter()
        .map(|doc| doc.evidence_id.clone())
        .collect();
    let mut entity_keys = HashSet::new();
    let mut entities = Vec::with_capacity(parsed.entities.len());

    for mut entity in parsed.entities {
        entity.entity_key = entity.entity_key.trim().to_string();
        entity.canonical_name = entity.canonical_name.trim().to_string();
        entity.summary = entity.summary.trim().to_string();

        if entity.entity_key.is_empty() {
            bail!("GraphRAG extraction returned an entity with an empty entity_key");
        }
        if !entity_keys.insert(entity.entity_key.clone()) {
            bail!(
                "GraphRAG extraction returned duplicate entity_key `{}`",
                entity.entity_key
            );
        }
        if entity.canonical_name.is_empty() {
            bail!("GraphRAG extraction returned an entity with an empty canonical_name");
        }
        if !ontology.supports_entity_type(entity.entity_type) {
            bail!(
                "GraphRAG extraction returned unsupported entity type `{}`",
                entity.entity_type.as_str()
            );
        }

        entity.aliases = dedupe_strings(entity.aliases);
        entity.evidence_ids = validate_evidence_ids(entity.evidence_ids, &known_evidence)?;
        if entity.summary.is_empty() {
            entity.summary.clone_from(&entity.canonical_name);
        }
        entities.push(entity);
    }

    let mut relations = Vec::with_capacity(parsed.relations.len());
    for mut relation in parsed.relations {
        relation.source_entity_key = relation.source_entity_key.trim().to_string();
        relation.target_entity_key = relation.target_entity_key.trim().to_string();
        relation.fact = relation.fact.trim().to_string();
        relation.confidence = relation.confidence.clamp(0.0, 1.0);

        if relation.source_entity_key.is_empty() || relation.target_entity_key.is_empty() {
            bail!("GraphRAG extraction returned a relation with an empty endpoint");
        }
        if !entity_keys.contains(&relation.source_entity_key)
            || !entity_keys.contains(&relation.target_entity_key)
        {
            bail!(
                "GraphRAG extraction relation references missing entities: {} -> {}",
                relation.source_entity_key,
                relation.target_entity_key
            );
        }
        if relation.fact.is_empty() {
            bail!("GraphRAG extraction returned a relation with an empty fact");
        }
        if let (Some(valid_from), Some(valid_until)) = (&relation.valid_from, &relation.valid_until)
            && valid_from > valid_until
        {
            bail!(
                "GraphRAG extraction returned relation with invalid validity window: {} -> {}",
                relation.source_entity_key,
                relation.target_entity_key
            );
        }
        if !ontology.supports_relation_type(relation.relation_type) {
            bail!(
                "GraphRAG extraction returned unsupported relation type `{}`",
                relation.relation_type.as_str()
            );
        }

        relation.evidence_ids = validate_evidence_ids(relation.evidence_ids, &known_evidence)?;
        relations.push(relation);
    }

    Ok(GraphExtractionResult {
        ontology,
        documents,
        entities,
        relations,
    })
}

fn validate_evidence_ids(
    evidence_ids: Vec<EvidenceId>,
    known_evidence: &HashSet<EvidenceId>,
) -> Result<Vec<EvidenceId>> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();

    for evidence_id in evidence_ids {
        if !known_evidence.contains(&evidence_id) {
            bail!("GraphRAG extraction referenced unknown evidence id `{evidence_id}`");
        }
        if seen.insert(evidence_id.clone()) {
            deduped.push(evidence_id);
        }
    }

    Ok(deduped)
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();

    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(trimmed.to_string());
        }
    }

    deduped
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use crate::core::memory::embeddings::NoopEmbedding;
    use crate::core::memory::graphrag::entity_resolution::{AlwaysDifferentJudge, EntityResolver};
    use crate::core::providers::{Provider, ProviderResult};

    use super::*;

    struct StubProvider {
        response: String,
    }

    impl Provider for StubProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok(self.response.clone()) })
        }
    }

    fn document(id: &str, title: &str, body: &str) -> GraphExtractionDocument {
        GraphExtractionDocument {
            evidence_id: EvidenceId::new(id),
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    #[tokio::test]
    async fn graphrag_extraction_defaults_to_companion_memory_ontology() {
        let provider = StubProvider {
            response: r#"{
                "entities": [
                    {
                        "entity_key": "user_haru",
                        "canonical_name": "Haru",
                        "entity_type": "user",
                        "aliases": ["operator"],
                        "summary": "The main user in this conversation.",
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "entity_key": "room_writer_lounge",
                        "canonical_name": "Writer Lounge",
                        "entity_type": "room_context",
                        "aliases": ["shared room"],
                        "summary": "Shared room for ongoing creative chats.",
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "entity_key": "topic_worldbuilding",
                        "canonical_name": "Noir worldbuilding",
                        "entity_type": "topic",
                        "aliases": ["worldbuilding"],
                        "summary": "Ongoing topic about a noir setting.",
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "entity_key": "preference_quiet_replies",
                        "canonical_name": "Quiet replies",
                        "entity_type": "preference",
                        "aliases": ["calm tone"],
                        "summary": "Haru prefers quieter reply tone in shared rooms.",
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "entity_key": "continuity_nanowrimo",
                        "canonical_name": "Nanowrimo follow-up",
                        "entity_type": "continuity",
                        "aliases": ["follow-up"],
                        "summary": "The current chat continues an earlier Nanowrimo thread.",
                        "evidence_ids": ["doc-1"]
                    }
                ],
                "relations": [
                    {
                        "source_entity_key": "user_haru",
                        "target_entity_key": "room_writer_lounge",
                        "relation_type": "participates_in",
                        "fact": "Haru participates in the Writer Lounge shared room.",
                        "confidence": 0.92,
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "source_entity_key": "user_haru",
                        "target_entity_key": "preference_quiet_replies",
                        "relation_type": "prefers",
                        "fact": "Haru prefers quiet replies in public rooms.",
                        "confidence": 0.89,
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "source_entity_key": "room_writer_lounge",
                        "target_entity_key": "topic_worldbuilding",
                        "relation_type": "discusses",
                        "fact": "The room is currently discussing noir worldbuilding.",
                        "confidence": 0.87,
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "source_entity_key": "continuity_nanowrimo",
                        "target_entity_key": "topic_worldbuilding",
                        "relation_type": "continues_from",
                        "fact": "The worldbuilding thread continues from Nanowrimo planning notes.",
                        "confidence": 0.84,
                        "evidence_ids": ["doc-1"]
                    }
                ]
            }"#
            .to_string(),
        };
        let pipeline = StructuredExtractionPipeline::new(
            &provider,
            GraphExtractionConfig {
                model: "test-model",
                temperature: 0.0,
            },
        );

        let result = pipeline
            .extract(&[document(
                "doc-1",
                "Shared room follow-up",
                "Haru returned to the Writer Lounge and asked to continue the noir worldbuilding thread with a quieter tone.",
            )])
            .await
            .expect("extract companion graphrag JSON");

        assert_eq!(result.ontology, companion_memory_ontology());
        assert!(result.entities.iter().any(|entity| {
            entity.entity_type == OntologyEntityType::User && entity.canonical_name == "Haru"
        }));
        assert!(result.relations.iter().any(|relation| {
            relation.relation_type == OntologyRelationType::Prefers
                && relation.source_entity_key == "user_haru"
                && relation.target_entity_key == "preference_quiet_replies"
        }));
    }

    #[tokio::test]
    async fn graphrag_extraction_resolves_entities_and_persists_roundtrip() {
        let provider = StubProvider {
            response: r#"{
                "entities": [
                    {
                        "entity_key": "user_haru",
                        "canonical_name": "Haru",
                        "entity_type": "user",
                        "aliases": ["operator"],
                        "summary": "The main user.",
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "entity_key": "user_haru_duplicate",
                        "canonical_name": "Haru Morita",
                        "entity_type": "user",
                        "aliases": ["Haru"],
                        "summary": "The same user mentioned by full name.",
                        "evidence_ids": ["doc-1"]
                    },
                    {
                        "entity_key": "preference_quiet_replies",
                        "canonical_name": "Quiet replies",
                        "entity_type": "preference",
                        "aliases": ["calm tone"],
                        "summary": "Haru prefers quieter replies in shared rooms.",
                        "evidence_ids": ["doc-1"]
                    }
                ],
                "relations": [
                    {
                        "source_entity_key": "user_haru_duplicate",
                        "target_entity_key": "preference_quiet_replies",
                        "relation_type": "prefers",
                        "fact": "Haru prefers quiet replies in shared rooms.",
                        "confidence": 0.91,
                        "evidence_ids": ["doc-1"]
                    }
                ]
            }"#
            .to_string(),
        };
        let pipeline = StructuredExtractionPipeline::new(
            &provider,
            GraphExtractionConfig {
                model: "test-model",
                temperature: 0.0,
            },
        );
        let resolver = EntityResolver::new(&NoopEmbedding, &AlwaysDifferentJudge);

        let result = pipeline
            .extract_and_resolve(
                &[document(
                    "doc-1",
                    "Shared-room preference",
                    "Haru Morita, also called Haru, prefers quiet replies in shared rooms.",
                )],
                &resolver,
            )
            .await
            .expect("extract and resolve GraphRAG JSON");

        assert_eq!(result.entities.len(), 2);
        let user = result
            .entities
            .iter()
            .find(|entity| entity.entity_type == OntologyEntityType::User)
            .expect("resolved user entity should remain");
        assert_eq!(user.entity_key, "user_haru");
        assert!(user.aliases.iter().any(|alias| alias == "Haru Morita"));
        assert_eq!(result.relations[0].source_entity_key, "user_haru");
        assert_eq!(
            result.relations[0].target_entity_key,
            "preference_quiet_replies"
        );

        let persisted = serde_json::to_string(&result).expect("GraphExtractionResult serializes");
        let restored: GraphExtractionResult =
            serde_json::from_str(&persisted).expect("GraphExtractionResult deserializes");
        assert_eq!(restored, result);
    }
}
