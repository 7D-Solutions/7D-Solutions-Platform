-- Add optional party_id reference to AR tables for cross-module party linkage.
-- All columns are nullable and non-breaking (no FK constraint - party lives in a separate service).

ALTER TABLE ar_customers ADD COLUMN IF NOT EXISTS party_id UUID;
ALTER TABLE ar_invoices ADD COLUMN IF NOT EXISTS party_id UUID;
ALTER TABLE ar_subscriptions ADD COLUMN IF NOT EXISTS party_id UUID;

-- Indexes for party lookup
CREATE INDEX IF NOT EXISTS idx_ar_customers_party_id ON ar_customers(party_id) WHERE party_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_ar_invoices_party_id ON ar_invoices(party_id) WHERE party_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_ar_subscriptions_party_id ON ar_subscriptions(party_id) WHERE party_id IS NOT NULL;
