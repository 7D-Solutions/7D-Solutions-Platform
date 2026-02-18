-- Inventory: Adjustment Ledger
--
-- Tracks named stock adjustments (positive gains, negative shrinkage/write-offs).
-- Each row corresponds to exactly one inventory_ledger row (entry_type = 'adjusted').
-- Design: append-only; rows are never updated or deleted.
--
-- Depends on: 002 (inventory_ledger), 012 (locations)

CREATE TABLE inv_adjustments (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    item_id          UUID NOT NULL REFERENCES items(id),
    warehouse_id     UUID NOT NULL,
    location_id      UUID REFERENCES locations(id),
    -- signed: positive = gain, negative = shrinkage/write-off (enforced by CHECK)
    quantity_delta   BIGINT NOT NULL CHECK (quantity_delta != 0),
    -- human-readable reason code (e.g. "shrinkage", "cycle_count_correction")
    reason           TEXT NOT NULL,
    event_id         UUID NOT NULL UNIQUE,
    ledger_entry_id  BIGINT NOT NULL REFERENCES inventory_ledger(id),
    adjusted_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_adjustments_tenant_item
    ON inv_adjustments(tenant_id, item_id, warehouse_id);
CREATE INDEX idx_adjustments_event_id
    ON inv_adjustments(event_id);
