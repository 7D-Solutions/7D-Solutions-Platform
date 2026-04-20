-- Integrations outbox: add structured failure_reason column
--
-- Replaces free-text error_message parsing in DLQ queries with a stable enum
-- so /sync/dlq can filter deterministically without string matching.
--
-- Reason codes:
--   bus_publish_failed   — NATS publish failed, still within retry budget
--   retry_exhausted      — retry budget exhausted, row permanently failed
--   needs_reauth         — OAuth token no longer valid; operator must reconnect
--   authority_superseded — another push attempt superseded this one

ALTER TABLE integrations_outbox
    ADD COLUMN IF NOT EXISTS failure_reason TEXT
    CHECK (failure_reason IN (
        'bus_publish_failed',
        'retry_exhausted',
        'needs_reauth',
        'authority_superseded'
    ));

CREATE INDEX IF NOT EXISTS integrations_outbox_failure_reason_idx
    ON integrations_outbox (failure_reason)
    WHERE failed_at IS NOT NULL;
