CREATE TABLE IF NOT EXISTS events_outbox (
    event_id     UUID PRIMARY KEY,
    event_type   TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    payload      JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_events_outbox_unpublished
    ON events_outbox (created_at ASC)
    WHERE published_at IS NULL;
