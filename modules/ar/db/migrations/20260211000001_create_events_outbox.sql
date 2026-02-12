-- Events Outbox Table
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

-- Processed Events Table
-- Idempotent consumer pattern to prevent duplicate event processing
CREATE TABLE processed_events (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processor VARCHAR(100) NOT NULL
);

-- Index for duplicate detection
CREATE INDEX idx_processed_events_event_id ON processed_events (event_id);

-- Index for cleanup/monitoring
CREATE INDEX idx_processed_events_processed_at ON processed_events (processed_at);
