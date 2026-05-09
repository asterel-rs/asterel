-- Asterel PostgreSQL Memory Backend — Migration 002: halfvec
--
-- Converts retrieval_units.embedding from vector (float32) to halfvec
-- (float16) for 50% storage reduction with near-identical recall.
--
-- Prerequisites: pgvector >= 0.7.0 (halfvec support)
-- Idempotent: checks column type before ALTER.
--
-- Note: HNSW index recreation is delegated to the application-level
-- create_hnsw_index_if_absent() because CREATE INDEX CONCURRENTLY
-- cannot run inside a transaction block.

-- Phase 1: Drop existing HNSW index (must drop before type change)
DROP INDEX IF EXISTS idx_retrieval_units_embedding_hnsw;

-- Phase 2: Convert vector → halfvec (blocking ALTER)
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'retrieval_units'
          AND column_name = 'embedding'
          AND data_type = 'USER-DEFINED'
          AND udt_name = 'vector'
    ) THEN
        ALTER TABLE retrieval_units
            ALTER COLUMN embedding TYPE halfvec
            USING embedding::halfvec;
    END IF;
END $$;

-- Phase 3: Record migration version
INSERT INTO schema_version (version) VALUES (2);
