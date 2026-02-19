-- Add UNIQUE constraint on (run_id, party_id) for ttp_billing_run_items.
-- This enables ON CONFLICT (run_id, party_id) DO UPDATE for idempotent upserts
-- during billing run re-execution.

ALTER TABLE ttp_billing_run_items
    ADD CONSTRAINT IF NOT EXISTS uq_ttp_billing_run_items_run_party
    UNIQUE (run_id, party_id);
