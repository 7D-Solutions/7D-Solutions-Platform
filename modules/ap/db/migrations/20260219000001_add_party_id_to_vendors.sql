-- Add optional party_id reference to vendors for cross-module party linkage.
-- Nullable and non-breaking - no FK constraint (party lives in a separate service).

ALTER TABLE vendors ADD COLUMN IF NOT EXISTS party_id UUID;

CREATE INDEX IF NOT EXISTS idx_vendors_party_id ON vendors(party_id) WHERE party_id IS NOT NULL;
