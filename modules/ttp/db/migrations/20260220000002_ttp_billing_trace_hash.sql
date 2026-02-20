-- Add trace_hash column to ttp_billing_run_items for metering reconciliation.
-- When a billing run item originates from metered usage, trace_hash stores
-- the SHA-256 of the serialized PriceTrace payload. This provides a stable
-- linkage from the AR invoice back to the exact metering trace that produced it.

ALTER TABLE ttp_billing_run_items
    ADD COLUMN IF NOT EXISTS trace_hash TEXT;

-- Index for lookups by trace_hash (reconciliation queries)
CREATE INDEX IF NOT EXISTS idx_ttp_run_items_trace_hash
    ON ttp_billing_run_items (trace_hash)
    WHERE trace_hash IS NOT NULL;
