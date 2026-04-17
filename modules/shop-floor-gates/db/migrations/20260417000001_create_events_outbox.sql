-- Shop-Floor-Gates Events Outbox
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

CREATE INDEX idx_sfg_outbox_unpublished ON events_outbox (created_at) WHERE published_at IS NULL;
CREATE INDEX idx_sfg_outbox_published ON events_outbox (published_at) WHERE published_at IS NOT NULL;

CREATE TABLE processed_events (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processor VARCHAR(100) NOT NULL
);

CREATE INDEX idx_sfg_processed_events_event_id ON processed_events (event_id);
