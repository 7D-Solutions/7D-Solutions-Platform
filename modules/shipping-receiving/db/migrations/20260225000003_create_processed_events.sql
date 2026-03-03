-- Idempotency table for inbound event consumers.
-- Prevents duplicate processing of PO_APPROVED, SO_RELEASED, etc.
--
-- Note: migration 000001 also creates sr_processed_events with a different
-- schema (BIGSERIAL id, processor NOT NULL). This migration fixes it by
-- dropping that version first and creating the correct schema.

DROP TABLE IF EXISTS sr_processed_events CASCADE;

CREATE TABLE sr_processed_events (
    event_id   UUID PRIMARY KEY,
    event_type TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sr_processed_events_type
    ON sr_processed_events (event_type, processed_at);
