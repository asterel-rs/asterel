ALTER TABLE memory_events
    ADD COLUMN IF NOT EXISTS emotion_label TEXT,
    ADD COLUMN IF NOT EXISTS emotion_valence REAL,
    ADD COLUMN IF NOT EXISTS emotion_arousal REAL,
    ADD COLUMN IF NOT EXISTS emotion_confidence REAL;

CREATE INDEX IF NOT EXISTS idx_memory_events_emotion
    ON memory_events (entity_id, emotion_label)
    WHERE emotion_label IS NOT NULL;

INSERT INTO schema_version (version) VALUES (6);
