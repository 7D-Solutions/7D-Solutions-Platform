-- Events outbox for reliable event publishing
CREATE TABLE events_outbox (
    id BIGSERIAL PRIMARY KEY,
    subject VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at TIMESTAMPTZ,
    CONSTRAINT events_outbox_published_check CHECK (published_at IS NULL OR published_at >= created_at)
);

-- Index for efficient polling of unpublished events
CREATE INDEX idx_events_outbox_unpublished ON events_outbox(created_at) WHERE published_at IS NULL;
