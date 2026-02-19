-- Party Module: Outbox + Idempotency
--
-- Transactional outbox pattern for exactly-once event publishing.
--
-- party_outbox:
--   Events written atomically with business mutations.
--   Background publisher reads unpublished rows and sends to NATS.
--   correlation_id / causation_id carried for distributed tracing
--   (matches EventEnvelope constitutional metadata).
--
-- party_processed_events:
--   Records each event_id consumed by this service.
--   Consumers check this table before processing to skip duplicates.
--
-- party_idempotency_keys:
--   HTTP-level idempotency for API callers supplying Idempotency-Key header.
--   Scoped per (app_id, idempotency_key) to prevent cross-tenant collisions.

-- ============================================================
-- TRANSACTIONAL OUTBOX
-- ============================================================

CREATE TABLE party_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,
    event_type      TEXT NOT NULL,
    aggregate_type  TEXT NOT NULL,
    aggregate_id    TEXT NOT NULL,
    app_id          TEXT NOT NULL,
    payload         JSONB NOT NULL,
    -- EventEnvelope tracing metadata
    correlation_id  TEXT,
    causation_id    TEXT,
    schema_version  TEXT NOT NULL DEFAULT '1.0.0',
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    published_at    TIMESTAMP WITH TIME ZONE
);

-- Background publisher scans unpublished rows ordered by created_at
CREATE INDEX idx_party_outbox_unpublished ON party_outbox(created_at)
    WHERE published_at IS NULL;
CREATE INDEX idx_party_outbox_app_id ON party_outbox(app_id);

-- ============================================================
-- IDEMPOTENT CONSUMER TRACKING
-- ============================================================

CREATE TABLE party_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_party_processed_event_id  ON party_processed_events(event_id);
CREATE INDEX idx_party_processed_at        ON party_processed_events(processed_at);

-- ============================================================
-- HTTP IDEMPOTENCY KEYS
-- ============================================================

CREATE TABLE party_idempotency_keys (
    id               BIGSERIAL PRIMARY KEY,
    app_id           TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    request_hash     TEXT NOT NULL,
    response_body    JSONB NOT NULL,
    status_code      INT NOT NULL,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    expires_at       TIMESTAMP WITH TIME ZONE NOT NULL,

    CONSTRAINT party_idempotency_keys_app_key_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_party_idempotency_expires ON party_idempotency_keys(expires_at);
CREATE INDEX idx_party_idempotency_app_id  ON party_idempotency_keys(app_id);
