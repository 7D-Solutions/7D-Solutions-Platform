-- Create failed_events table for Dead Letter Queue (DLQ)
-- This table stores events that failed to process after all retries

CREATE TABLE IF NOT EXISTS failed_events (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL,
    subject TEXT NOT NULL,
    envelope_json JSONB NOT NULL,
    error TEXT NOT NULL,
    failed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    retry_count INT NOT NULL DEFAULT 0,

    -- Indexes for common queries
    CONSTRAINT failed_events_event_id_unique UNIQUE (event_id)
);

CREATE INDEX idx_failed_events_subject ON failed_events(subject);
CREATE INDEX idx_failed_events_failed_at ON failed_events(failed_at);
