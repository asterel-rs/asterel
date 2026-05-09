//! Graph-augmented retrieval (`GraphRAG`) for the companion's memory.
//!
//! Builds and queries a typed knowledge graph alongside the vector store.
//! During writes, entities and edges are projected into `graph_entities` /
//! `graph_edges` tables. During recalls, Personalized PageRank (PPR)
//! activation spreading is used to boost structurally relevant memories.
//!
//! ## Sub-modules
//!
//! | Sub-module          | Responsibility                                      |
//! |---------------------|-----------------------------------------------------|
//! | `activation`        | CSR graph snapshot, PPR algorithm, snapshot cache   |
//! | `entity_resolution` | Dedup/alias resolution for extracted graph entities |
//! | `extraction`        | LLM-driven entity/relation extraction pipeline      |
//! | `grounding`         | Companion memory grounding                           |
//! | `ontology`          | Entity and relation type registry                   |
//! | `provenance`        | Evidence snippet set attached to graph edges        |
//! | `provenance_search` | Temporal/contradiction search over provenance       |
//!
//! References: [GRAPHRAG] Edge et al., 2024 — Graph RAG. See the public
//! research reference index in the docs site.

mod activation;
mod entity_resolution;
mod extraction;
mod grounding;
mod ontology;
mod provenance;
mod provenance_search;

use std::sync::OnceLock;

pub use activation::{GraphActivationCache, GraphSnapshot, PprQuery, PprResult};

pub use entity_resolution::{
    AlwaysDifferentJudge, EntityAliasRecord, EntityJudge, EntityJudgeDecision, EntityResolution,
    EntityResolver, LlmEntityJudge,
};
pub use extraction::{
    ExtractedEntity, ExtractedRelation, GraphExtractionConfig, GraphExtractionDocument,
    GraphExtractionResult, StructuredExtractionPipeline,
};
pub use grounding::{
    CompanionMemoryGrounding, build_companion_memory_grounding, render_companion_memory_grounding,
};
pub use ontology::{
    COMPANION_MEMORY_ENTITY_TYPES, COMPANION_MEMORY_RELATION_TYPES, OntologyDefinition,
    OntologyEntityType, OntologyRelationType, companion_memory_ontology,
};
pub use provenance::{EvidenceSnippet, EvidenceSnippetSet};
pub use provenance_search::{
    ProvenanceContradiction, ProvenanceSearchIndex, TemporalGraphRelation,
};

/// Return the process-global graph activation cache.
///
/// Initialized once on first call via [`OnceLock`]. All `Postgres` write
/// paths call `activation_cache().invalidate(entity_id)` after committing
/// so stale snapshots do not persist across writes.
pub(crate) fn activation_cache() -> &'static GraphActivationCache {
    static CACHE: OnceLock<GraphActivationCache> = OnceLock::new();
    CACHE.get_or_init(GraphActivationCache::new)
}
