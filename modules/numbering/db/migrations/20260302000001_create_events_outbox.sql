-- Numbering Events Outbox Table
-- Transactional outbox pattern for reliable event publishing
CREATE TABLE events_outbox (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    aggregate_type VARCHAR(100) NOT NULL,
    aggregate_id VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TIMESTAMP
);

-- Index for unpublished events (used by background publisher)
CREATE INDEX idx_events_outbox_unpublished ON events_outbox (created_at)
WHERE published_at IS NULL;

-- Index for cleanup queries
CREATE INDEX idx_events_outbox_published ON events_outbox (published_at)
WHERE published_at IS NOT NULL;
