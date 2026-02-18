-- Inventory: Low-Stock Signal Dedup State
--
-- Tracks whether a low-stock signal has been emitted for each
-- (tenant, item, location) combination.  The row acts as a flip-flop:
--
--   below_threshold = FALSE  — stock is at or above reorder_point;
--                              the next drop WILL fire a signal.
--   below_threshold = TRUE   — signal already emitted; no new signal
--                              until stock recovers above reorder_point.
--
-- State transitions (enforced in Rust evaluator):
--   available < reorder_point  AND below_threshold = FALSE
--       → set below_threshold = TRUE, emit inventory.low_stock_triggered
--   available >= reorder_point AND below_threshold = TRUE
--       → set below_threshold = FALSE  (re-arms for the next crossing)
--   all other combinations → no-op
--
-- Key matches the reorder_policies table:
--   location_id IS NULL  → global policy (item-wide)
--   location_id IS NOT NULL → location-scoped policy
--
-- Depends on: 000001 (items), 000012 (locations), 000018 (reorder_policies)

CREATE TABLE inv_low_stock_state (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT    NOT NULL,
    item_id         UUID    NOT NULL REFERENCES items(id),
    location_id     UUID    REFERENCES locations(id),
    below_threshold BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- One state row per global (null-location) policy
CREATE UNIQUE INDEX uq_low_stock_state_global
    ON inv_low_stock_state(tenant_id, item_id)
    WHERE location_id IS NULL;

-- One state row per location-scoped policy
CREATE UNIQUE INDEX uq_low_stock_state_location
    ON inv_low_stock_state(tenant_id, item_id, location_id)
    WHERE location_id IS NOT NULL;

-- Lookup index for listing all state rows for an item
CREATE INDEX idx_low_stock_state_tenant_item
    ON inv_low_stock_state(tenant_id, item_id);
