CREATE TABLE IF NOT EXISTS operator_trust_state (
    operator_id TEXT PRIMARY KEY,
    decision_count BIGINT NOT NULL DEFAULT 0,
    approve_count BIGINT NOT NULL DEFAULT 0,
    revise_count BIGINT NOT NULL DEFAULT 0,
    reject_count BIGINT NOT NULL DEFAULT 0,
    fast_approve_ewma DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    rubber_stamp_score DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    over_reject_score DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    confidence DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    intervention_level TEXT NOT NULL DEFAULT 'observe',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO schema_version (version) VALUES (15);
