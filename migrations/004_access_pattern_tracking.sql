-- Asterel PostgreSQL Memory Backend — Migration 004: access pattern tracking
--
-- Adds access_count and accessed_at columns to retrieval_units so that
-- recall frequency can be tracked and used for scoring.
-- Existing rows default to 0 accesses / never accessed.

ALTER TABLE retrieval_units
    ADD COLUMN IF NOT EXISTS access_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS accessed_at  TIMESTAMPTZ;

-- Partial index for efficient queries on frequently-accessed memories.
CREATE INDEX IF NOT EXISTS idx_retrieval_units_access_count
    ON retrieval_units (entity_id, access_count DESC)
    WHERE access_count > 0;

-- Record migration version.
INSERT INTO schema_version (version) VALUES (4);
