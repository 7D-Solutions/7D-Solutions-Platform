-- Events Infrastructure Tables
-- Transactional outbox pattern and idempotent consumer tracking

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

-- Failed Events Table (Dead Letter Queue)
-- Stores events that failed to process after all retries
CREATE TABLE IF NOT EXISTS failed_events (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL,
    subject TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    envelope_json JSONB NOT NULL,
    error TEXT NOT NULL,
    failed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    retry_count INT NOT NULL DEFAULT 0,

    -- Indexes for common queries
    CONSTRAINT failed_events_event_id_unique UNIQUE (event_id)
);

CREATE INDEX idx_failed_events_subject ON failed_events(subject);
CREATE INDEX idx_failed_events_failed_at ON failed_events(failed_at);
CREATE INDEX idx_failed_events_tenant_id ON failed_events(tenant_id);
