-- Migration template: event_dedupe table
-- Copy into your consuming service's migrations directory.
-- Tracks which event IDs have been processed for idempotency.

CREATE TABLE IF NOT EXISTS event_dedupe (
    event_id      UUID        PRIMARY KEY,
    subject       TEXT        NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for cleanup queries (e.g. purge entries older than N days).
CREATE INDEX IF NOT EXISTS idx_event_dedupe_first_seen
    ON event_dedupe (first_seen_at);
