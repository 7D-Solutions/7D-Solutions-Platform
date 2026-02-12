-- Payments Module Outbox/Inbox Infrastructure
-- Enables reliable event publishing and idempotent consumption

-- Events Outbox Table
-- Stores events that need to be published to the event bus
CREATE TABLE payments_events_outbox (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    occurred_at TIMESTAMP NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    correlation_id VARCHAR(255),
    causation_id VARCHAR(255),
    payload JSONB NOT NULL,
    published_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX payments_events_outbox_published_at ON payments_events_outbox(published_at)
    WHERE published_at IS NULL;
CREATE INDEX payments_events_outbox_occurred_at ON payments_events_outbox(occurred_at);
CREATE INDEX payments_events_outbox_event_type ON payments_events_outbox(event_type);

-- Processed Events Table
-- Tracks consumed events for idempotent processing
CREATE TABLE payments_processed_events (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    source_module VARCHAR(50) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT unique_processed_event UNIQUE (event_id)
);

CREATE INDEX payments_processed_events_processed_at ON payments_processed_events(processed_at);
CREATE INDEX payments_processed_events_event_type ON payments_processed_events(event_type);
