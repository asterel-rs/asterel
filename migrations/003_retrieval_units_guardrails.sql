-- Asterel PostgreSQL Memory Backend — Migration 003: retrieval unit guardrails
--
-- Adds database-level constraints for bounded scoring fields and
-- constrained enum-like status values used by recall reinforcement.
-- Existing rows are clamped before constraints are installed.

-- Phase 1: Normalize existing rows into valid score ranges.
UPDATE retrieval_units
SET recency_score = LEAST(1.0, GREATEST(0.0, recency_score)),
    importance = LEAST(1.0, GREATEST(0.0, importance)),
    reliability = LEAST(1.0, GREATEST(0.0, reliability)),
    contradiction_penalty = LEAST(1.0, GREATEST(0.0, contradiction_penalty))
WHERE recency_score < 0.0
   OR recency_score > 1.0
   OR importance < 0.0
   OR importance > 1.0
   OR reliability < 0.0
   OR reliability > 1.0
   OR contradiction_penalty < 0.0
   OR contradiction_penalty > 1.0;

-- Phase 2: Add guardrail constraints idempotently.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_retrieval_units_recency_score_range'
    ) THEN
        ALTER TABLE retrieval_units
            ADD CONSTRAINT chk_retrieval_units_recency_score_range
            CHECK (recency_score >= 0.0 AND recency_score <= 1.0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_retrieval_units_importance_range'
    ) THEN
        ALTER TABLE retrieval_units
            ADD CONSTRAINT chk_retrieval_units_importance_range
            CHECK (importance >= 0.0 AND importance <= 1.0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_retrieval_units_reliability_range'
    ) THEN
        ALTER TABLE retrieval_units
            ADD CONSTRAINT chk_retrieval_units_reliability_range
            CHECK (reliability >= 0.0 AND reliability <= 1.0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_retrieval_units_contradiction_penalty_range'
    ) THEN
        ALTER TABLE retrieval_units
            ADD CONSTRAINT chk_retrieval_units_contradiction_penalty_range
            CHECK (contradiction_penalty >= 0.0 AND contradiction_penalty <= 1.0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_retrieval_units_visibility_values'
    ) THEN
        ALTER TABLE retrieval_units
            ADD CONSTRAINT chk_retrieval_units_visibility_values
            CHECK (visibility IN ('public', 'private', 'secret'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_retrieval_units_promotion_status_values'
    ) THEN
        ALTER TABLE retrieval_units
            ADD CONSTRAINT chk_retrieval_units_promotion_status_values
            CHECK (promotion_status IN ('raw', 'candidate', 'promoted', 'demoted'));
    END IF;
END $$;

-- Phase 3: Add focused index for recall reinforcement update path.
CREATE INDEX IF NOT EXISTS idx_retrieval_units_recall_reinforcement
    ON retrieval_units (entity_id, slot_key)
    WHERE promotion_status IN ('promoted', 'candidate')
      AND visibility != 'secret';

-- Phase 4: Record migration version.
INSERT INTO schema_version (version) VALUES (3);
