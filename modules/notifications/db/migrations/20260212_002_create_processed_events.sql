-- processed_events: Idempotency tracking for consumed events
CREATE TABLE IF NOT EXISTS processed_events (
    event_id UUID PRIMARY KEY,
    subject VARCHAR(255) NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    source_module VARCHAR(100) NOT NULL
);

CREATE INDEX idx_processed_events_tenant ON processed_events(tenant_id);
CREATE INDEX idx_processed_events_processed_at ON processed_events(processed_at);
