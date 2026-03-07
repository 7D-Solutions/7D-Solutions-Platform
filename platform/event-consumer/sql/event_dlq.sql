-- Migration template: event_dlq table
-- Copy into your consuming service's migrations directory.
-- Stores events that failed processing for later investigation or replay.

CREATE TABLE IF NOT EXISTS event_dlq (
    event_id      UUID        PRIMARY KEY,
    subject       TEXT        NOT NULL,
    failure_kind  TEXT        NOT NULL CHECK (failure_kind IN ('retryable', 'fatal', 'poison')),
    error_message TEXT        NOT NULL,
    payload       JSONB       NOT NULL,
    payload_hash  TEXT        NOT NULL DEFAULT '',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for filtering by failure kind (e.g. list all retryable entries).
CREATE INDEX IF NOT EXISTS idx_event_dlq_failure_kind
    ON event_dlq (failure_kind);

-- Index for chronological queries and cleanup.
CREATE INDEX IF NOT EXISTS idx_event_dlq_created_at
    ON event_dlq (created_at);
