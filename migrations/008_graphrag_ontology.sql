ALTER TABLE graph_entities
    ADD COLUMN IF NOT EXISTS canonical_name TEXT,
    ADD COLUMN IF NOT EXISTS ontology_type TEXT,
    ADD COLUMN IF NOT EXISTS attributes JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS embedding vector;

ALTER TABLE graph_edges
    ADD COLUMN IF NOT EXISTS fact TEXT,
    ADD COLUMN IF NOT EXISTS valid_from TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS valid_until TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS confidence DOUBLE PRECISION NOT NULL DEFAULT 0.5;

CREATE TABLE IF NOT EXISTS ontology_definitions (
    ontology_id TEXT PRIMARY KEY,
    owner_entity_id TEXT NOT NULL,
    entity_types JSONB NOT NULL DEFAULT '[]'::jsonb,
    relation_types JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS graph_entity_aliases (
    owner_entity_id TEXT NOT NULL,
    canonical_graph_entity_id TEXT NOT NULL,
    alias TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_entity_id, canonical_graph_entity_id, alias)
);

CREATE INDEX IF NOT EXISTS idx_ontology_definitions_owner
    ON ontology_definitions (owner_entity_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_graph_entity_aliases_owner_alias
    ON graph_entity_aliases (owner_entity_id, alias);

CREATE INDEX IF NOT EXISTS idx_graph_entities_ontology_type
    ON graph_entities (owner_entity_id, ontology_type, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_graph_edges_validity
    ON graph_edges (owner_entity_id, relation_type, valid_from, valid_until);

-- NOTE:
-- `graph_entities.embedding` remains a dimensionless `vector`, so pgvector
-- cannot build an HNSW index here. Keep the column available for future
-- graph-entity embedding writes without blocking runtime bootstrap.

INSERT INTO schema_version (version) VALUES (8);
