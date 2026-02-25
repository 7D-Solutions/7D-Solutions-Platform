-- Shipping-Receiving: Outbox + Processed Events
-- bd-37xa: Module-standard transactional outbox for exactly-once event publishing.
--
-- sr_events_outbox:
--   Events written atomically with business mutations.
--   Background publisher reads unpublished rows and sends to NATS.
--
-- sr_processed_events:
--   Records each event_id that has been fully processed by this service.
--   Consumers check before processing to skip duplicates.

-- Transactional outbox
CREATE TABLE sr_events_outbox (
    id               BIGSERIAL PRIMARY KEY,
    event_id         UUID NOT NULL UNIQUE,
    event_type       TEXT NOT NULL,
    aggregate_type   TEXT NOT NULL,
    aggregate_id     TEXT NOT NULL,
    tenant_id        TEXT NOT NULL,
    payload          JSONB NOT NULL,
    correlation_id   TEXT,
    causation_id     TEXT,
    schema_version   TEXT NOT NULL DEFAULT '1.0.0',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at     TIMESTAMPTZ
);

CREATE INDEX idx_sr_outbox_unpublished ON sr_events_outbox(created_at)
    WHERE published_at IS NULL;
CREATE INDEX idx_sr_outbox_tenant_id   ON sr_events_outbox(tenant_id);

-- Idempotent consumer tracking
CREATE TABLE sr_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sr_processed_event_id ON sr_processed_events(event_id);
CREATE INDEX idx_sr_processed_at       ON sr_processed_events(processed_at);
