ALTER TABLE graph_edges
    ADD COLUMN IF NOT EXISTS valid_from TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS valid_until TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS confidence DOUBLE PRECISION NOT NULL DEFAULT 0.5;

CREATE TABLE IF NOT EXISTS graph_entity_aliases (
    owner_entity_id TEXT NOT NULL,
    canonical_graph_entity_id TEXT NOT NULL,
    alias TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_entity_id, canonical_graph_entity_id, alias)
);

CREATE INDEX IF NOT EXISTS idx_graph_edges_validity_confidence
    ON graph_edges (owner_entity_id, relation_type, valid_from, valid_until, confidence);

CREATE INDEX IF NOT EXISTS idx_graph_entity_aliases_owner_alias
    ON graph_entity_aliases (owner_entity_id, alias);

INSERT INTO schema_version (version) VALUES (9);
