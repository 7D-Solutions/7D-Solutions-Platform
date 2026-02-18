-- Timekeeping: Outbox + Processed Events + Idempotency Keys
--
-- Transactional outbox pattern for exactly-once event publishing.
-- Follows the same convention as treasury and inventory modules.
--
-- events_outbox:       Written atomically with business mutations.
-- processed_events:    Tracks consumed NATS events to prevent double-processing.
-- tk_idempotency_keys: HTTP-level idempotency for API callers.

-- ============================================================
-- EVENTS OUTBOX (transactional outbox pattern)
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

-- Partial index: unpublished events (hot path for publisher loop)
CREATE INDEX idx_events_outbox_unpublished
    ON events_outbox(created_at)
    WHERE published_at IS NULL;

-- Partial index: published events (cleanup / monitoring)
CREATE INDEX idx_events_outbox_published
    ON events_outbox(published_at)
    WHERE published_at IS NOT NULL;

-- ============================================================
-- PROCESSED EVENTS (idempotent consumer pattern)
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
-- ============================================================

CREATE TABLE tk_idempotency_keys (
    id              BIGSERIAL PRIMARY KEY,
    app_id          VARCHAR(50) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    request_hash    VARCHAR(64) NOT NULL,
    response_body   JSONB NOT NULL,
    status_code     INTEGER NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL,

    CONSTRAINT tk_idempotency_unique
        UNIQUE (app_id, idempotency_key)
);

CREATE INDEX tk_idempotency_keys_app_id
    ON tk_idempotency_keys(app_id);
CREATE INDEX tk_idempotency_keys_expires_at
    ON tk_idempotency_keys(expires_at);
