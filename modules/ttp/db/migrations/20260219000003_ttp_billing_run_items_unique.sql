-- Add UNIQUE constraint on (run_id, party_id) for ttp_billing_run_items.
-- This enables ON CONFLICT (run_id, party_id) DO UPDATE for idempotent upserts
-- during billing run re-execution.
--
-- Note: PostgreSQL does not support ADD CONSTRAINT IF NOT EXISTS.
-- Using CREATE UNIQUE INDEX IF NOT EXISTS instead, which is functionally
-- equivalent for ON CONFLICT usage.

CREATE UNIQUE INDEX IF NOT EXISTS uq_ttp_billing_run_items_run_party
    ON ttp_billing_run_items (run_id, party_id);
