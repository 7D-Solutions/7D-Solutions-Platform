-- Integrations Module: Inbound Webhook Ingestion
--
-- Persists raw inbound webhook payloads received from external systems
-- (e.g. Stripe events, GitHub webhooks, payment gateway callbacks).
--
-- Raw storage first: payloads are written immediately on receipt before
-- any processing. This decouples ingestion latency from processing latency
-- and provides a durable audit trail.
--
-- idempotency_key: dedup key supplied by the source system (e.g. Stripe
--   event ID). NULL for systems that don't provide one. The
--   (app_id, system, idempotency_key) UNIQUE constraint prevents double-
--   processing when the source retries delivery.
--
-- processed_at: NULL until the dispatcher successfully routes the payload.

CREATE TABLE integrations_webhook_ingest (
    id               BIGSERIAL PRIMARY KEY,
    app_id           TEXT    NOT NULL,
    system           TEXT    NOT NULL,   -- source system name, e.g. 'stripe', 'github'
    event_type       TEXT,               -- source event type if known, e.g. 'invoice.payment_succeeded'
    raw_payload      JSONB   NOT NULL,   -- verbatim payload body
    headers          JSONB   NOT NULL DEFAULT '{}'::JSONB,  -- HTTP headers at receipt
    received_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    processed_at     TIMESTAMP WITH TIME ZONE,              -- NULL = pending processing
    idempotency_key  TEXT,               -- source dedup key (e.g. Stripe event ID)

    -- Prevent duplicate delivery processing for systems that supply a dedup key
    CONSTRAINT integrations_webhook_ingest_dedup
        UNIQUE (app_id, system, idempotency_key)
);

-- Dispatcher polls for unprocessed payloads ordered by receipt time
CREATE INDEX idx_integrations_wh_ingest_unprocessed
    ON integrations_webhook_ingest(app_id, received_at)
    WHERE processed_at IS NULL;

-- Audit / replay: look up all ingest records for a given system
CREATE INDEX idx_integrations_wh_ingest_system
    ON integrations_webhook_ingest(app_id, system, received_at DESC);

-- Time-based cleanup / archival queries
CREATE INDEX idx_integrations_wh_ingest_received_at
    ON integrations_webhook_ingest(received_at);
