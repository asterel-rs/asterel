ALTER TABLE graph_entities
    ADD COLUMN IF NOT EXISTS last_accessed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS access_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS pinned BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS temporal_decay_score DOUBLE PRECISION NOT NULL DEFAULT 1.0;

UPDATE graph_entities
SET temporal_decay_score = 1.0
WHERE temporal_decay_score IS NULL;

CREATE INDEX IF NOT EXISTS idx_graph_entities_decay_priority
    ON graph_entities (owner_entity_id, pinned, temporal_decay_score DESC, importance DESC, updated_at DESC);

INSERT INTO schema_version (version) VALUES (12);
