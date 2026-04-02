-- Minimal schema for the vertical proof module.
-- Only the outbox table is needed to prove event publishing.

CREATE TABLE IF NOT EXISTS events_outbox (
    event_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type  TEXT        NOT NULL,
    payload     JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ
);
