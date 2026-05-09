-- Asterel PostgreSQL Memory Backend — Migration 005: pinned flag and temporal decay
--
-- Adds a `pinned` boolean column to retrieval_units so that important
-- memories can be exempted from temporal decay scoring.
-- Existing rows default to false (not pinned).

ALTER TABLE retrieval_units
    ADD COLUMN IF NOT EXISTS pinned BOOLEAN NOT NULL DEFAULT FALSE;

-- Partial index for efficient queries filtering pinned memories.
CREATE INDEX IF NOT EXISTS idx_retrieval_units_pinned
    ON retrieval_units (entity_id)
    WHERE pinned = TRUE;

-- Record migration version.
INSERT INTO schema_version (version) VALUES (5);
