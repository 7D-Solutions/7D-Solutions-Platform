-- Integrations outbox retry metadata
--
-- Adds durable retry/failure tracking so the relay worker can survive restarts
-- without losing publish-attempt state.

ALTER TABLE integrations_outbox
    ADD COLUMN IF NOT EXISTS retry_count INT NOT NULL DEFAULT 0;

ALTER TABLE integrations_outbox
    ADD COLUMN IF NOT EXISTS error_message TEXT;

ALTER TABLE integrations_outbox
    ADD COLUMN IF NOT EXISTS failed_at TIMESTAMP WITH TIME ZONE;

CREATE INDEX IF NOT EXISTS idx_integrations_outbox_retryable
    ON integrations_outbox(created_at)
    WHERE published_at IS NULL AND failed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_integrations_outbox_failed
    ON integrations_outbox(failed_at)
    WHERE failed_at IS NOT NULL;
