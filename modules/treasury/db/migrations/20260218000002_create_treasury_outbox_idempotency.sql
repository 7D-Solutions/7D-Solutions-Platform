-- Treasury Module: Outbox and Idempotency Tables
-- Transactional outbox for reliable event publishing (Guard → Mutation → Outbox atomicity)
-- Idempotency keys for safe API replay

-- ============================================================
-- EVENTS OUTBOX (transactional outbox pattern)
-- Referenced by modules/treasury/src/outbox/mod.rs
-- ============================================================

CREATE TABLE events_outbox (
    id             BIGSERIAL PRIMARY KEY,
    event_id       UUID NOT NULL UNIQUE,
    event_type     VARCHAR(255) NOT NULL,
    aggregate_type VARCHAR(100) NOT NULL,
    aggregate_id   VARCHAR(255) NOT NULL,
    payload        JSONB NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at   TIMESTAMPTZ
);

-- Partial index: only unpublished events (hot path for the publisher loop)
CREATE INDEX idx_events_outbox_unpublished
    ON events_outbox(created_at)
    WHERE published_at IS NULL;

-- Partial index: published events (for cleanup / monitoring queries)
CREATE INDEX idx_events_outbox_published
    ON events_outbox(published_at)
    WHERE published_at IS NOT NULL;

-- ============================================================
-- PROCESSED EVENTS (idempotent consumer pattern)
-- Prevents duplicate processing of NATS events this service consumes
-- ============================================================

CREATE TABLE processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   VARCHAR(255) NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processor    VARCHAR(100) NOT NULL
);

CREATE INDEX idx_processed_events_event_id
    ON processed_events(event_id);
CREATE INDEX idx_processed_events_processed_at
    ON processed_events(processed_at);

-- ============================================================
-- API IDEMPOTENCY KEYS
-- Prevents duplicate mutations from replayed HTTP requests
-- ============================================================

CREATE TABLE treasury_idempotency_keys (
    id              BIGSERIAL PRIMARY KEY,
    app_id          VARCHAR(50) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    request_hash    VARCHAR(64) NOT NULL,
    response_body   JSONB NOT NULL,
    status_code     INTEGER NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL,
    CONSTRAINT treasury_idempotency_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX treasury_idempotency_keys_app_id
    ON treasury_idempotency_keys(app_id);
CREATE INDEX treasury_idempotency_keys_expires_at
    ON treasury_idempotency_keys(expires_at);
