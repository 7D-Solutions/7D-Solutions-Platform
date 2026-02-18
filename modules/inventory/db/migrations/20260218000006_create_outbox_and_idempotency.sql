-- Inventory: Outbox + Processed Events + Idempotency Keys
--
-- Transactional outbox pattern for exactly-once event publishing.
-- All three tables work together to guarantee exactly-once behavior
-- across retries and replays.
--
-- inv_outbox:
--   Events written atomically with business mutations.
--   Background publisher reads unpublished rows and sends to NATS.
--   correlation_id / causation_id carried for distributed tracing.
--
-- inv_processed_events:
--   Records each event_id that has been fully processed by this service.
--   Consumers check this table before processing to skip duplicates.
--
-- inv_idempotency_keys:
--   HTTP-level idempotency for API callers supplying Idempotency-Key header.
--   Scoped per tenant to prevent cross-tenant collisions.

-- Transactional outbox
CREATE TABLE inv_outbox (
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
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    published_at     TIMESTAMP WITH TIME ZONE
);

-- Background publisher scans unpublished rows ordered by created_at
CREATE INDEX idx_inv_outbox_unpublished ON inv_outbox(created_at)
    WHERE published_at IS NULL;
CREATE INDEX idx_inv_outbox_tenant_id   ON inv_outbox(tenant_id);

-- Idempotent consumer tracking
CREATE TABLE inv_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_inv_processed_event_id  ON inv_processed_events(event_id);
CREATE INDEX idx_inv_processed_at        ON inv_processed_events(processed_at);

-- HTTP idempotency keys (scoped per tenant)
CREATE TABLE inv_idempotency_keys (
    id               BIGSERIAL PRIMARY KEY,
    tenant_id        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    request_hash     TEXT NOT NULL,
    response_body    JSONB NOT NULL,
    status_code      INT NOT NULL,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    expires_at       TIMESTAMP WITH TIME ZONE NOT NULL,

    CONSTRAINT inv_idempotency_keys_tenant_key_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_inv_idempotency_expires ON inv_idempotency_keys(expires_at);
CREATE INDEX idx_inv_idempotency_tenant  ON inv_idempotency_keys(tenant_id);
