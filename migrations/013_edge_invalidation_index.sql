CREATE INDEX IF NOT EXISTS idx_graph_edges_active
    ON graph_edges (owner_entity_id, from_entity_id, to_entity_id, relation_type)
    WHERE valid_until IS NULL;

CREATE INDEX IF NOT EXISTS idx_graph_edges_history
    ON graph_edges (owner_entity_id, relation_type, valid_from, valid_until);

INSERT INTO schema_version (version) VALUES (13);
