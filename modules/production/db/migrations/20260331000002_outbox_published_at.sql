-- Add published_at column for SDK outbox publisher compatibility.
-- The publisher polls WHERE published_at IS NULL and sets published_at = NOW() after publish.
-- Existing rows with published = TRUE get backfilled so they're not re-published.

ALTER TABLE production_outbox
    ADD COLUMN IF NOT EXISTS published_at TIMESTAMPTZ;

UPDATE production_outbox
SET published_at = created_at
WHERE published = TRUE AND published_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_production_outbox_unpublished_at
    ON production_outbox (created_at) WHERE published_at IS NULL;
