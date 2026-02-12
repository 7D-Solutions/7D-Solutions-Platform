-- events_outbox: Transactional outbox pattern for reliable event publishing
CREATE TABLE IF NOT EXISTS events_outbox (
    id BIGSERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    subject VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    tenant_id VARCHAR(255) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    published_at TIMESTAMP WITH TIME ZONE,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    retry_count INT NOT NULL DEFAULT 0,
    error_message TEXT
);

CREATE INDEX idx_events_outbox_status_created ON events_outbox(status, created_at) WHERE status = 'pending';
CREATE INDEX idx_events_outbox_tenant ON events_outbox(tenant_id);
CREATE INDEX idx_events_outbox_event_id ON events_outbox(event_id);
