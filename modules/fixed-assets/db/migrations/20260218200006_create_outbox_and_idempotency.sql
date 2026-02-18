-- Fixed Assets: Outbox + Processed Events + Idempotency Keys
-- bd-2s2s: Transactional outbox pattern for exactly-once event publishing.
--
-- fa_events_outbox:
--   Events written atomically with business mutations.
--   Background publisher reads unpublished rows and sends to NATS.
--   correlation_id / causation_id carried for distributed tracing.
--
-- fa_processed_events:
--   Records each event_id that has been fully processed by this service.
--   Consumers check this table before processing to skip duplicates.
--
-- fa_idempotency_keys:
--   HTTP-level idempotency for API callers supplying Idempotency-Key header.
--   Scoped per tenant to prevent cross-tenant collisions.

-- Transactional outbox
CREATE TABLE fa_events_outbox (
    id               BIGSERIAL PRIMARY KEY,
    event_id         UUID NOT NULL UNIQUE,
    event_type       TEXT NOT NULL,
    aggregate_type   TEXT NOT NULL,
    aggregate_id     TEXT NOT NULL,
    tenant_id        TEXT NOT NULL,
    payload          JSONB NOT NULL,
    -- EventEnvelope tracing metadata
    correlation_id   TEXT,
    causation_id     TEXT,
    schema_version   TEXT NOT NULL DEFAULT '1.0.0',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at     TIMESTAMPTZ
);

-- Background publisher scans unpublished rows ordered by created_at
CREATE INDEX idx_fa_outbox_unpublished ON fa_events_outbox(created_at)
    WHERE published_at IS NULL;
CREATE INDEX idx_fa_outbox_tenant_id   ON fa_events_outbox(tenant_id);

-- Idempotent consumer tracking
CREATE TABLE fa_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_fa_processed_event_id  ON fa_processed_events(event_id);
CREATE INDEX idx_fa_processed_at        ON fa_processed_events(processed_at);

-- HTTP idempotency keys (scoped per tenant)
CREATE TABLE fa_idempotency_keys (
    id               BIGSERIAL PRIMARY KEY,
    tenant_id        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    request_hash     TEXT NOT NULL,
    response_body    JSONB NOT NULL,
    status_code      INT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at       TIMESTAMPTZ NOT NULL,

    CONSTRAINT fa_idempotency_keys_tenant_key_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_fa_idempotency_expires ON fa_idempotency_keys(expires_at);
CREATE INDEX idx_fa_idempotency_tenant  ON fa_idempotency_keys(tenant_id);
