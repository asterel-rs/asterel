//! Entity resolution: decides whether two extracted graph entities refer to the
//! same real-world object.
//!
//! Uses a hybrid strategy: (1) cosine similarity on entity embeddings provides
//! a fast pre-filter; (2) an LLM judge arbitrates ambiguous cases by comparing
//! canonical names, aliases, and summaries.
//!
//! References: [FELLEGI-SUNTER] Fellegi & Sunter, 1969; [ER-SURVEY]
//! Christophides et al., 2020. See the public research reference index in the
//! docs site.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::extraction::{ExtractedEntity, ExtractedRelation};
use crate::core::memory::embeddings::EmbeddingProvider;
use crate::core::memory::vector::cosine_similarity;
use crate::core::providers::Provider;
use crate::utils::text::sanitize_prompt_line;

const ENTITY_JUDGE_SYSTEM_PROMPT: &str = concat!(
    "You resolve whether two extracted GraphRAG entities refer to the same real-world ",
    "memory graph entity. Return ONLY JSON with keys `same_entity` (boolean) ",
    "and `rationale` (string). Use aliases, names, and summaries. Be conservative: ",
    "only return true when they refer to the same thing."
);

pub type JudgeFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityJudgeDecision {
    pub same_entity: bool,
    pub rationale: String,
}

pub trait EntityJudge: Send + Sync {
    fn judge<'a>(
        &'a self,
        candidate: &'a ExtractedEntity,
        canonical: &'a ExtractedEntity,
    ) -> JudgeFuture<'a, EntityJudgeDecision>;
}

#[derive(Default)]
pub struct AlwaysDifferentJudge;

impl EntityJudge for AlwaysDifferentJudge {
    fn judge<'a>(
        &'a self,
        _candidate: &'a ExtractedEntity,
        _canonical: &'a ExtractedEntity,
    ) -> JudgeFuture<'a, EntityJudgeDecision> {
        Box::pin(async {
            Ok(EntityJudgeDecision {
                same_entity: false,
                rationale: "judge disabled".to_string(),
            })
        })
    }
}

pub struct LlmEntityJudge<'a> {
    provider: &'a dyn Provider,
    model: &'a str,
    temperature: f64,
}

impl<'a> LlmEntityJudge<'a> {
    #[must_use]
    pub fn new(provider: &'a dyn Provider, model: &'a str, temperature: f64) -> Self {
        Self {
            provider,
            model,
            temperature,
        }
    }
}

impl EntityJudge for LlmEntityJudge<'_> {
    fn judge<'a>(
        &'a self,
        candidate: &'a ExtractedEntity,
        canonical: &'a ExtractedEntity,
    ) -> JudgeFuture<'a, EntityJudgeDecision> {
        Box::pin(async move {
            let candidate_name = sanitize_prompt_line(&candidate.canonical_name);
            let candidate_summary = sanitize_prompt_line(&candidate.summary);
            let canonical_name = sanitize_prompt_line(&canonical.canonical_name);
            let canonical_summary = sanitize_prompt_line(&canonical.summary);
            let mut prompt = String::with_capacity(
                64 + candidate_name.len()
                    + candidate.aliases.iter().map(|a| a.len() + 2).sum::<usize>()
                    + candidate_summary.len()
                    + canonical_name.len()
                    + canonical.aliases.iter().map(|a| a.len() + 2).sum::<usize>()
                    + canonical_summary.len(),
            );
            let _ = write!(prompt, "Candidate: {candidate_name} | aliases: ");
            let mut first = true;
            for a in &candidate.aliases {
                if !first {
                    prompt.push_str(", ");
                }
                prompt.push_str(&sanitize_prompt_line(a));
                first = false;
            }
            let _ = write!(
                prompt,
                " | summary: {candidate_summary}\nCanonical: {canonical_name} | aliases: "
            );
            first = true;
            for a in &canonical.aliases {
                if !first {
                    prompt.push_str(", ");
                }
                prompt.push_str(&sanitize_prompt_line(a));
                first = false;
            }
            let _ = write!(prompt, " | summary: {canonical_summary}");
            let response = self
                .provider
                .chat_with_system(
                    Some(ENTITY_JUDGE_SYSTEM_PROMPT),
                    &prompt,
                    self.model,
                    self.temperature,
                )
                .await
                .context("GraphRAG entity judge LLM call failed")?;

            let trimmed = response.trim();
            let json = trimmed
                .strip_prefix("```json")
                .map(str::trim)
                .and_then(|value| value.strip_suffix("```"))
                .map_or(trimmed, str::trim);
            serde_json::from_str(json).context("parse GraphRAG entity judge JSON")
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityAliasRecord {
    pub alias: String,
    pub canonical_entity_key: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityResolution {
    pub entities: Vec<ExtractedEntity>,
    pub relations: Vec<ExtractedRelation>,
    pub aliases: Vec<EntityAliasRecord>,
}

pub struct EntityResolver<'a> {
    embedder: &'a dyn EmbeddingProvider,
    judge: &'a dyn EntityJudge,
    lexical_merge_threshold: f64,
    embedding_merge_threshold: f32,
    judge_threshold: f64,
}

impl<'a> EntityResolver<'a> {
    #[must_use]
    pub fn new(embedder: &'a dyn EmbeddingProvider, judge: &'a dyn EntityJudge) -> Self {
        Self {
            embedder,
            judge,
            lexical_merge_threshold: 0.9,
            embedding_merge_threshold: 0.92,
            judge_threshold: 0.45,
        }
    }

    /// Resolve extracted entities into canonical entities and remap their relations.
    ///
    /// # Errors
    ///
    /// Returns an error when the entity judge or embedding provider fails.
    ///
    /// # Panics
    ///
    /// Panics if the internal pending-entity state is unexpectedly missing while merging.
    pub async fn resolve(
        &self,
        entities: Vec<ExtractedEntity>,
        relations: Vec<ExtractedRelation>,
    ) -> Result<EntityResolution> {
        let mut canonicals: Vec<ExtractedEntity> = Vec::new();
        let mut aliases = Vec::new();
        let mut remap = HashMap::new();

        for entity in entities {
            let mut pending_entity = Some(entity);
            let mut merged = false;
            for canonical in &mut canonicals {
                let entity = pending_entity
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("entity resolution invariant violated: pending_entity was consumed before the merge loop completed — this is a bug in the resolution pipeline"))?;
                let score = lexical_score(&entity, canonical);
                let embedding_similarity = self.embedding_similarity(&entity, canonical).await;
                let exact_alias_match = exact_alias_match(&entity, canonical);

                let should_merge = exact_alias_match
                    || score >= self.lexical_merge_threshold
                    || embedding_similarity >= self.embedding_merge_threshold
                    || ((score >= self.judge_threshold
                        || f64::from(embedding_similarity) >= self.judge_threshold)
                        && self.judge.judge(&entity, canonical).await?.same_entity);

                if should_merge {
                    let alias_confidence = f64::from(embedding_similarity).max(score).max(0.6);
                    let alias_values = alias_values(&entity);
                    let entity_key = entity.entity_key.clone();
                    let entity = pending_entity
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("entity resolution invariant violated: pending_entity was None at merge point for entity_key='{entity_key}' — this is a bug in the resolution pipeline"))?;
                    remap.insert(entity_key, canonical.entity_key.clone());
                    merge_entities(canonical, entity);
                    aliases.extend(alias_values.into_iter().map(|alias| EntityAliasRecord {
                        alias,
                        canonical_entity_key: canonical.entity_key.clone(),
                        confidence: alias_confidence,
                    }));
                    merged = true;
                    break;
                }
            }

            if !merged {
                let entity = pending_entity
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("entity resolution invariant violated: pending_entity was None for an unmerged entity — this is a bug in the resolution pipeline"))?;
                remap.insert(entity.entity_key.clone(), entity.entity_key.clone());
                canonicals.push(entity);
            }
        }

        let relations = relations
            .into_iter()
            .map(|mut relation| {
                if let Some(source) = remap.get(&relation.source_entity_key) {
                    relation.source_entity_key = source.clone();
                }
                if let Some(target) = remap.get(&relation.target_entity_key) {
                    relation.target_entity_key = target.clone();
                }
                relation
            })
            .collect();

        Ok(EntityResolution {
            entities: canonicals,
            relations,
            aliases: dedupe_aliases(aliases),
        })
    }

    async fn embedding_similarity(&self, lhs: &ExtractedEntity, rhs: &ExtractedEntity) -> f32 {
        if self.embedder.dimensions() == 0 {
            return 0.0;
        }

        let lhs_text = entity_text(lhs);
        let rhs_text = entity_text(rhs);
        let lhs_embedding = match self.embedder.embed_one_document(&lhs_text).await {
            Ok(embedding) if !embedding.is_empty() => embedding,
            _ => return 0.0,
        };
        let rhs_embedding = match self.embedder.embed_one_document(&rhs_text).await {
            Ok(embedding) if !embedding.is_empty() => embedding,
            _ => return 0.0,
        };
        cosine_similarity(&lhs_embedding, &rhs_embedding)
    }
}

fn merge_entities(canonical: &mut ExtractedEntity, candidate: ExtractedEntity) {
    canonical.canonical_name = preferred_name(&canonical.canonical_name, &candidate.canonical_name);
    canonical.summary = preferred_summary(&canonical.summary, &candidate.summary);
    canonical.aliases = dedupe_strings(
        alias_values(canonical)
            .into_iter()
            .chain(alias_values(&candidate))
            .collect(),
    );
    canonical.evidence_ids = dedupe_evidence(
        canonical
            .evidence_ids
            .iter()
            .cloned()
            .chain(candidate.evidence_ids)
            .collect(),
    );
}

fn preferred_name(current: &str, candidate: &str) -> String {
    if candidate.len() > current.len() {
        candidate.to_string()
    } else {
        current.to_string()
    }
}

fn preferred_summary(current: &str, candidate: &str) -> String {
    if candidate.len() > current.len() {
        candidate.to_string()
    } else {
        current.to_string()
    }
}

fn alias_values(entity: &ExtractedEntity) -> Vec<String> {
    let mut values = vec![entity.canonical_name.clone()];
    values.extend(entity.aliases.iter().cloned());
    dedupe_strings(values)
}

fn exact_alias_match(lhs: &ExtractedEntity, rhs: &ExtractedEntity) -> bool {
    if lhs.entity_type != rhs.entity_type {
        return false;
    }

    let lhs_values: HashSet<String> = alias_values(lhs)
        .into_iter()
        .map(|value| normalize(&value))
        .collect();
    let rhs_values: HashSet<String> = alias_values(rhs)
        .into_iter()
        .map(|value| normalize(&value))
        .collect();
    !lhs_values.is_disjoint(&rhs_values)
}

fn lexical_score(lhs: &ExtractedEntity, rhs: &ExtractedEntity) -> f64 {
    if lhs.entity_type != rhs.entity_type {
        return 0.0;
    }

    let lhs_values = alias_values(lhs);
    let rhs_values = alias_values(rhs);
    lhs_values
        .iter()
        .flat_map(|lhs_value| {
            rhs_values
                .iter()
                .map(move |rhs_value| pairwise_score(lhs_value, rhs_value))
        })
        .fold(0.0, f64::max)
}

fn pairwise_score(lhs: &str, rhs: &str) -> f64 {
    let lhs_normalized = normalize(lhs);
    let rhs_normalized = normalize(rhs);
    if lhs_normalized == rhs_normalized {
        return 1.0;
    }
    if lhs_normalized.contains(&rhs_normalized) || rhs_normalized.contains(&lhs_normalized) {
        return 0.95;
    }

    let lhs_tokens: HashSet<&str> = lhs_normalized.split_whitespace().collect();
    let rhs_tokens: HashSet<&str> = rhs_normalized.split_whitespace().collect();
    let union = lhs_tokens.union(&rhs_tokens).count();
    if union == 0 {
        return 0.0;
    }
    let intersection = lhs_tokens.intersection(&rhs_tokens).count();
    let intersection = u32::try_from(intersection).unwrap_or(u32::MAX);
    let union = u32::try_from(union).unwrap_or(u32::MAX);
    f64::from(intersection) / f64::from(union)
}

fn normalize(value: &str) -> String {
    // Single pass: collect alphanumeric chars (lowercased), separating runs
    // of whitespace/non-alnum with a single space. No intermediate allocations.
    let mut result = String::with_capacity(value.len());
    let mut needs_space = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if needs_space && !result.is_empty() {
                result.push(' ');
            }
            result.push(ch.to_ascii_lowercase());
            needs_space = false;
        } else if !result.is_empty() {
            needs_space = true;
        }
    }
    result
}

fn entity_text(entity: &ExtractedEntity) -> String {
    let alias_len: usize = entity.aliases.iter().map(|a| a.len() + 1).sum();
    let mut out =
        String::with_capacity(entity.canonical_name.len() + 1 + alias_len + entity.summary.len());
    out.push_str(&entity.canonical_name);
    for alias in &entity.aliases {
        out.push(' ');
        out.push_str(alias);
    }
    out.push(' ');
    out.push_str(&entity.summary);
    out
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = normalize(trimmed);
        if seen.insert(key) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

fn dedupe_evidence(
    values: Vec<crate::contracts::ids::EvidenceId>,
) -> Vec<crate::contracts::ids::EvidenceId> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        if seen.insert(value.clone()) {
            deduped.push(value);
        }
    }
    deduped
}

fn dedupe_aliases(values: Vec<EntityAliasRecord>) -> Vec<EntityAliasRecord> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let key = (normalize(&value.alias), value.canonical_entity_key.clone());
        if seen.insert(key) {
            deduped.push(value);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{EntityJudge, EntityJudgeDecision, EntityResolver, JudgeFuture, normalize};
    use crate::contracts::ids::EvidenceId;
    use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
    use crate::core::memory::graphrag::extraction::{ExtractedEntity, ExtractedRelation};
    use crate::core::memory::graphrag::ontology::OntologyEntityType;

    struct TestEmbedding {
        vectors: HashMap<String, Vec<f32>>,
    }

    impl EmbeddingProvider for TestEmbedding {
        fn name(&self) -> &'static str {
            "test"
        }

        fn dimensions(&self) -> usize {
            3
        }

        fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
            Box::pin(async move {
                Ok(texts
                    .iter()
                    .map(|text| {
                        self.vectors
                            .get(&normalize(text))
                            .cloned()
                            .unwrap_or_else(|| vec![0.0, 0.0, 1.0])
                    })
                    .collect())
            })
        }
    }

    struct AbbreviationJudge;

    impl EntityJudge for AbbreviationJudge {
        fn judge<'a>(
            &'a self,
            candidate: &'a ExtractedEntity,
            canonical: &'a ExtractedEntity,
        ) -> JudgeFuture<'a, EntityJudgeDecision> {
            Box::pin(async move {
                let names = [
                    normalize(&candidate.canonical_name),
                    normalize(&canonical.canonical_name),
                ];
                let same = names.iter().any(|name| name == "customer success")
                    && names.iter().any(|name| name == "cs");
                Ok(EntityJudgeDecision {
                    same_entity: same,
                    rationale: "abbreviation judge".to_string(),
                })
            })
        }
    }

    fn entity(key: &str, name: &str, aliases: &[&str]) -> ExtractedEntity {
        ExtractedEntity {
            entity_key: key.to_string(),
            canonical_name: name.to_string(),
            entity_type: OntologyEntityType::Topic,
            aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
            summary: format!("{name} topic"),
            evidence_ids: vec![EvidenceId::new(key)],
        }
    }

    #[tokio::test]
    async fn graphrag_entity_resolution_merges_same_topic_aliases() {
        let embedder = TestEmbedding {
            vectors: HashMap::from([
                (
                    normalize("Customer Success CS customer success stakeholder"),
                    vec![1.0, 0.0, 0.0],
                ),
                (normalize("CS cs stakeholder"), vec![1.0, 0.0, 0.0]),
            ]),
        };
        let resolver = EntityResolver::new(&embedder, &AbbreviationJudge);

        let resolution = resolver
            .resolve(
                vec![
                    entity("topic_customer_success", "Customer Success", &["CS"]),
                    entity("topic_cs", "CS", &[]),
                ],
                vec![ExtractedRelation {
                    source_entity_key: "topic_cs".to_string(),
                    target_entity_key: "topic_customer_success".to_string(),
                    relation_type:
                        crate::core::memory::graphrag::ontology::OntologyRelationType::Mentions,
                    fact: "CS is mentioned as the same topic.".to_string(),
                    confidence: 0.7,
                    valid_from: None,
                    valid_until: None,
                    evidence_ids: vec![EvidenceId::new("doc-1")],
                }],
            )
            .await
            .expect("resolve aliases");

        assert_eq!(resolution.entities.len(), 1);
        assert_eq!(resolution.entities[0].canonical_name, "Customer Success");
        assert!(
            resolution.entities[0]
                .aliases
                .iter()
                .any(|alias| alias == "CS")
        );
        assert_eq!(
            resolution.relations[0].source_entity_key,
            "topic_customer_success"
        );
        assert_eq!(
            resolution.relations[0].target_entity_key,
            "topic_customer_success"
        );
        assert_eq!(resolution.aliases.len(), 1);
    }
}
