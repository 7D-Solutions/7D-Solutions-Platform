-- Processed events for idempotent event consumption
CREATE TABLE processed_events (
    event_id VARCHAR(255) PRIMARY KEY,
    subject VARCHAR(255) NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for cleanup queries (e.g., removing old events)
CREATE INDEX idx_processed_events_processed_at ON processed_events(processed_at);
