-- Status buckets schema: available | quarantine | damaged
--
-- Invariants:
-- - Status is set on receipt (default: 'available')
-- - Status changes are future movements between buckets, never in-place updates
-- - available_status_on_hand mirrors the 'available' bucket in item_on_hand_by_status
-- - quantity_available = available_status_on_hand - quantity_reserved
-- - Only 'available' stock counts toward reservable stock
--
-- Depends on: 005 (item_on_hand), 001 (items)

-- Step 1: Create the status enum
CREATE TYPE inv_item_status AS ENUM ('available', 'quarantine', 'damaged');

-- Step 2: Create status-bucketed on-hand projection
CREATE TABLE item_on_hand_by_status (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    item_id          UUID NOT NULL REFERENCES items(id),
    warehouse_id     UUID NOT NULL,
    status           inv_item_status NOT NULL,
    quantity_on_hand BIGINT NOT NULL DEFAULT 0 CHECK (quantity_on_hand >= 0),
    updated_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    CONSTRAINT item_on_hand_by_status_unique
        UNIQUE (tenant_id, item_id, warehouse_id, status)
);

CREATE INDEX idx_on_hand_by_status_available
    ON item_on_hand_by_status (tenant_id, item_id, warehouse_id)
    WHERE status = 'available' AND quantity_on_hand > 0;

-- Step 3: Add available_status_on_hand to item_on_hand (default 0 for backfill safety)
ALTER TABLE item_on_hand
    ADD COLUMN available_status_on_hand BIGINT NOT NULL DEFAULT 0;

-- Step 4: Backfill — all existing on-hand stock is assumed 'available'
UPDATE item_on_hand SET available_status_on_hand = quantity_on_hand;

-- Step 5: Populate status buckets from existing on-hand (backfill)
INSERT INTO item_on_hand_by_status (tenant_id, item_id, warehouse_id, status, quantity_on_hand)
SELECT tenant_id, item_id, warehouse_id, 'available'::inv_item_status, quantity_on_hand
FROM item_on_hand
WHERE quantity_on_hand > 0
ON CONFLICT (tenant_id, item_id, warehouse_id, status) DO NOTHING;

-- Step 6: Rebuild quantity_available with the new formula.
-- Drop index first (it references quantity_available), then drop + re-add the column.
DROP INDEX IF EXISTS idx_on_hand_available;

ALTER TABLE item_on_hand DROP COLUMN quantity_available;

ALTER TABLE item_on_hand
    ADD COLUMN quantity_available BIGINT
        GENERATED ALWAYS AS (available_status_on_hand - quantity_reserved) STORED;

-- Step 7: Recreate index with new formula
CREATE INDEX idx_on_hand_available
    ON item_on_hand (tenant_id, item_id, warehouse_id)
    WHERE quantity_available > 0;
