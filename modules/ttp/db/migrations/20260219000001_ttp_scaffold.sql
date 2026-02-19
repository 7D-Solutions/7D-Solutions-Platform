-- TTP scaffold migration
-- Placeholder: future beads (bd-3mja, bd-3fq6) will add domain tables.

-- Processed events table for idempotent event consumption
CREATE TABLE IF NOT EXISTS ttp_processed_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL UNIQUE,
    event_type TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ttp_processed_events_event_id
    ON ttp_processed_events (event_id);
