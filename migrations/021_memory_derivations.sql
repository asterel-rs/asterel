CREATE TABLE IF NOT EXISTS memory_derivations (
    derived_entity_id TEXT NOT NULL,
    derived_slot_key TEXT NOT NULL,
    source_entity_id TEXT NOT NULL,
    source_slot_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (
        derived_entity_id,
        derived_slot_key,
        source_entity_id,
        source_slot_key
    )
);

CREATE INDEX IF NOT EXISTS idx_memory_derivations_source
    ON memory_derivations (source_entity_id, source_slot_key);

INSERT INTO memory_derivations (
    derived_entity_id, derived_slot_key, source_entity_id, source_slot_key
)
SELECT entity_id, slot_key, entity_id,
       substring(provenance_reference FROM length('memory.promotion.from:') + 1)
FROM memory_events
WHERE provenance_reference LIKE 'memory.promotion.from:%'
ON CONFLICT DO NOTHING;

INSERT INTO memory_derivations (
    derived_entity_id, derived_slot_key, source_entity_id, source_slot_key
)
SELECT snapshot.entity_id, snapshot.slot_key, source.entity_id, source.slot_key
FROM memory_events AS snapshot
JOIN retrieval_units AS source
 ON source.entity_id = snapshot.entity_id
 AND source.layer = 'episodic'
 AND (
     source.slot_key =
         substring(snapshot.provenance_reference FROM length('memory.sleep.consolidation:') + 1)
     OR left(source.slot_key, length(
         substring(snapshot.provenance_reference FROM length('memory.sleep.consolidation:') + 1)
     ) + 1) =
         substring(snapshot.provenance_reference FROM length('memory.sleep.consolidation:') + 1)
         || '.'
 )
WHERE snapshot.provenance_reference LIKE 'memory.sleep.consolidation:%'
ON CONFLICT DO NOTHING;

INSERT INTO schema_version (version) VALUES (21);
