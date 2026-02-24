-- Events outbox table for transactional outbox pattern
CREATE TABLE IF NOT EXISTS events_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,
    subject         TEXT NOT NULL,
    payload         JSONB NOT NULL,
    tenant_id       TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    event_type      TEXT,
    source_module   TEXT,
    source_version  TEXT,
    schema_version  TEXT,
    occurred_at     TIMESTAMPTZ,
    replay_safe     BOOLEAN,
    trace_id        TEXT,
    correlation_id  TEXT,
    causation_id    TEXT,
    reverses_event_id   UUID,
    supersedes_event_id UUID,
    side_effect_id  TEXT,
    mutation_class  TEXT,
    retry_count     INT NOT NULL DEFAULT 0,
    error_message   TEXT,
    published_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_events_outbox_status ON events_outbox (status, created_at);
CREATE INDEX idx_events_outbox_tenant ON events_outbox (tenant_id);

-- Processed events table for idempotency tracking
CREATE TABLE IF NOT EXISTS processed_events (
    event_id        UUID PRIMARY KEY,
    subject         TEXT NOT NULL,
    tenant_id       TEXT NOT NULL,
    source_module   TEXT NOT NULL,
    processed_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
