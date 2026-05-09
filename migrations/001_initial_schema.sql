-- Asterel PostgreSQL Memory Backend — Initial Schema
-- Requires: pgvector, pg_trgm extensions

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Schema version tracking (replaces PRAGMA user_version)
CREATE TABLE IF NOT EXISTS schema_version (
    version   INTEGER NOT NULL,
    migrated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO schema_version (version) VALUES (1);

-- Embedding cache (LRU)
CREATE TABLE IF NOT EXISTS embedding_cache (
    content_hash TEXT PRIMARY KEY,
    embedding    bytea       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    accessed_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_cache_accessed ON embedding_cache (accessed_at);

-- Core event log with integrity hash chain
CREATE TABLE IF NOT EXISTS memory_events (
    seq_id                    BIGSERIAL   NOT NULL UNIQUE,
    event_id                  TEXT        PRIMARY KEY,
    entity_id                 TEXT        NOT NULL,
    slot_key                  TEXT        NOT NULL,
    layer                     TEXT        NOT NULL DEFAULT 'working',
    event_type                TEXT        NOT NULL,
    value                     TEXT        NOT NULL,
    source                    TEXT        NOT NULL,
    confidence                DOUBLE PRECISION NOT NULL,
    importance                DOUBLE PRECISION NOT NULL,
    provenance_source_class   TEXT,
    provenance_reference      TEXT,
    provenance_evidence_uri   TEXT,
    retention_tier            TEXT        NOT NULL DEFAULT 'working',
    retention_expires_at      TIMESTAMPTZ,
    signal_tier               TEXT        NOT NULL DEFAULT 'raw',
    source_kind               TEXT,
    privacy_level             TEXT        NOT NULL,
    occurred_at               TIMESTAMPTZ NOT NULL,
    ingested_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    supersedes_event_id       TEXT,
    integrity_prev_hash       TEXT        NOT NULL DEFAULT '',
    integrity_hash            TEXT        NOT NULL DEFAULT ''
);
CREATE INDEX idx_memory_events_entity_slot
    ON memory_events (entity_id, slot_key, occurred_at DESC);
CREATE INDEX idx_memory_events_entity_layer
    ON memory_events (entity_id, layer, occurred_at DESC);
CREATE INDEX idx_memory_events_retention_expires
    ON memory_events (retention_expires_at)
    WHERE retention_expires_at IS NOT NULL;
CREATE INDEX idx_memory_events_seq
    ON memory_events (seq_id);

-- Materialized belief state (conflict-resolved winner per slot)
CREATE TABLE IF NOT EXISTS belief_slots (
    entity_id      TEXT NOT NULL,
    slot_key       TEXT NOT NULL,
    value          TEXT NOT NULL,
    status         TEXT NOT NULL,
    winner_event_id TEXT NOT NULL,
    source         TEXT NOT NULL,
    confidence     DOUBLE PRECISION NOT NULL,
    importance     DOUBLE PRECISION NOT NULL,
    privacy_level  TEXT NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (entity_id, slot_key)
);

-- Searchable retrieval units with vector embedding + FTS
CREATE TABLE IF NOT EXISTS retrieval_units (
    unit_id              TEXT PRIMARY KEY,
    entity_id            TEXT NOT NULL,
    slot_key             TEXT NOT NULL,
    content              TEXT NOT NULL,
    content_type         TEXT NOT NULL DEFAULT 'belief',
    signal_tier          TEXT NOT NULL DEFAULT 'belief',
    promotion_status     TEXT NOT NULL DEFAULT 'promoted',
    chunk_index          INTEGER,
    source_uri           TEXT,
    source_kind          TEXT,
    recency_score        DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    importance           DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    reliability          DOUBLE PRECISION NOT NULL DEFAULT 0.8,
    contradiction_penalty DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    visibility           TEXT NOT NULL DEFAULT 'public',

    embedding            vector,
    embedding_model      TEXT,
    embedding_dim        INTEGER,
    layer                TEXT NOT NULL DEFAULT 'working',
    provenance_source_class TEXT,
    provenance_reference    TEXT,
    provenance_evidence_uri TEXT,
    retention_tier       TEXT NOT NULL DEFAULT 'working',
    retention_expires_at TIMESTAMPTZ,

    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Auto-maintained tsvector for full-text search
    fts_document         tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', coalesce(slot_key, '')), 'A') ||
        setweight(to_tsvector('simple', coalesce(content, '')), 'B')
    ) STORED,

    UNIQUE (entity_id, slot_key, chunk_index)
);
CREATE INDEX idx_retrieval_units_entity
    ON retrieval_units (entity_id);
CREATE INDEX idx_retrieval_units_entity_slot
    ON retrieval_units (entity_id, slot_key);
CREATE INDEX idx_retrieval_units_signal_tier
    ON retrieval_units (signal_tier);
CREATE INDEX idx_retrieval_units_promotion
    ON retrieval_units (promotion_status);
CREATE INDEX idx_retrieval_units_entity_visibility
    ON retrieval_units (entity_id, visibility, updated_at DESC);
CREATE INDEX idx_retrieval_units_retention
    ON retrieval_units (retention_expires_at)
    WHERE retention_expires_at IS NOT NULL;

-- GIN index for full-text search (tsvector)
CREATE INDEX idx_retrieval_units_fts
    ON retrieval_units USING GIN (fts_document);

-- GIN trigram index for substring/fuzzy matching
CREATE INDEX idx_retrieval_units_content_trgm
    ON retrieval_units USING GIN (content gin_trgm_ops);

-- Knowledge graph nodes
CREATE TABLE IF NOT EXISTS graph_entities (
    graph_entity_id  TEXT PRIMARY KEY,
    owner_entity_id  TEXT NOT NULL,
    entity_type      TEXT NOT NULL,
    label            TEXT NOT NULL,
    value            TEXT NOT NULL,
    source           TEXT NOT NULL,
    confidence       DOUBLE PRECISION NOT NULL,
    importance       DOUBLE PRECISION NOT NULL,
    privacy_level    TEXT NOT NULL,
    updated_at       TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_graph_entities_owner_type
    ON graph_entities (owner_entity_id, entity_type, updated_at DESC);
CREATE INDEX idx_graph_entities_owner_label
    ON graph_entities (owner_entity_id, label);

-- Knowledge graph edges
CREATE TABLE IF NOT EXISTS graph_edges (
    graph_edge_id    TEXT PRIMARY KEY,
    owner_entity_id  TEXT NOT NULL,
    from_entity_id   TEXT NOT NULL,
    to_entity_id     TEXT NOT NULL,
    relation_type    TEXT NOT NULL,
    weight           DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    event_id         TEXT,
    created_at       TIMESTAMPTZ NOT NULL,
    UNIQUE (owner_entity_id, from_entity_id, to_entity_id, relation_type)
);
CREATE INDEX idx_graph_edges_owner_relation
    ON graph_edges (owner_entity_id, relation_type, created_at DESC);
CREATE INDEX idx_graph_edges_to_entity
    ON graph_edges (to_entity_id, relation_type);

-- Deletion audit ledger with integrity hash chain
CREATE TABLE IF NOT EXISTS deletion_ledger (
    seq_id               BIGSERIAL   NOT NULL UNIQUE,
    ledger_id            TEXT        PRIMARY KEY,
    entity_id            TEXT        NOT NULL,
    target_slot_key      TEXT        NOT NULL,
    phase                TEXT        NOT NULL,
    reason               TEXT        NOT NULL,
    requested_by         TEXT        NOT NULL,
    executed_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    integrity_prev_hash  TEXT        NOT NULL DEFAULT '',
    integrity_hash       TEXT        NOT NULL DEFAULT ''
);
CREATE INDEX idx_deletion_ledger_entity_slot_phase
    ON deletion_ledger (entity_id, target_slot_key, phase);
CREATE INDEX idx_deletion_ledger_seq
    ON deletion_ledger (seq_id);
