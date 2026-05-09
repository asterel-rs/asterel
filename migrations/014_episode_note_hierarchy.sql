ALTER TABLE graph_entities
    ADD COLUMN IF NOT EXISTS node_tier TEXT,
    ADD COLUMN IF NOT EXISTS parent_graph_entity_id TEXT,
    ADD COLUMN IF NOT EXISTS promoted_at TIMESTAMPTZ;

UPDATE graph_entities SET node_tier = 'note'
    WHERE graph_entity_id IN (SELECT DISTINCT parent_graph_entity_id FROM graph_entities WHERE parent_graph_entity_id IS NOT NULL);
UPDATE graph_entities SET node_tier = 'episode'
    WHERE node_tier IS NULL OR node_tier = '';

ALTER TABLE graph_entities
    ALTER COLUMN node_tier SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_graph_entities_owner_tier
    ON graph_entities (owner_entity_id, node_tier, updated_at DESC);

COMMENT ON COLUMN graph_entities.node_tier IS 'episode = raw event, note = distilled fact. No default - caller must classify.';

INSERT INTO schema_version (version) VALUES (14);
