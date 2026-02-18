-- Location-aware inventory ledger and on-hand projection
--
-- Adds nullable location_id to:
--   1. inventory_ledger  — records which bin/shelf a movement touched
--   2. item_on_hand      — separates on-hand quantities by location
--
-- Backward-compatibility guarantee:
--   All existing rows have location_id = NULL. Existing code paths that do not
--   supply a location continue to work identically — they operate on the
--   (tenant_id, item_id, warehouse_id, NULL) partition of item_on_hand.
--
-- Unique constraint strategy for nullable location_id:
--   PostgreSQL treats NULL values as distinct in unique indexes, so a plain
--   UNIQUE (tenant_id, item_id, warehouse_id, location_id) would allow multiple
--   rows with location_id = NULL. We use two partial unique indexes instead:
--     - item_on_hand_null_loc   : enforces uniqueness when location_id IS NULL
--     - item_on_hand_with_loc   : enforces uniqueness when location_id IS NOT NULL

-- ─── inventory_ledger ───────────────────────────────────────────────────────

ALTER TABLE inventory_ledger
    ADD COLUMN location_id UUID REFERENCES locations(id);

CREATE INDEX idx_ledger_location ON inventory_ledger(tenant_id, item_id, location_id)
    WHERE location_id IS NOT NULL;

-- ─── item_on_hand ───────────────────────────────────────────────────────────

-- Drop the original named unique constraint (replaced by partial indexes below)
ALTER TABLE item_on_hand DROP CONSTRAINT item_on_hand_unique;

ALTER TABLE item_on_hand
    ADD COLUMN location_id UUID REFERENCES locations(id);

-- Drop index that references the old unique constraint (idx_on_hand_item_wh was separate)
DROP INDEX IF EXISTS idx_on_hand_item_wh;

-- Partial unique index: one row per (tenant, item, warehouse) when no location is set
CREATE UNIQUE INDEX item_on_hand_null_loc
    ON item_on_hand(tenant_id, item_id, warehouse_id)
    WHERE location_id IS NULL;

-- Partial unique index: one row per (tenant, item, warehouse, location) when location is set
CREATE UNIQUE INDEX item_on_hand_with_loc
    ON item_on_hand(tenant_id, item_id, warehouse_id, location_id)
    WHERE location_id IS NOT NULL;

-- Recreate supporting indexes with location awareness
CREATE INDEX idx_on_hand_item_wh
    ON item_on_hand(item_id, warehouse_id);

CREATE INDEX idx_on_hand_tenant_item_loc
    ON item_on_hand(tenant_id, item_id, warehouse_id, location_id)
    WHERE location_id IS NOT NULL;
