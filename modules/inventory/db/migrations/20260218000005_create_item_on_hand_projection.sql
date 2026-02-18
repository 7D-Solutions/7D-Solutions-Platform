-- Inventory: Item On-Hand Projection
--
-- Derived read model for fast on-hand quantity and valuation lookups.
-- One row per (tenant_id, item_id, warehouse_id) — upserted by the write path.
--
-- quantity_available is a generated column:
--   quantity_available = quantity_on_hand - quantity_reserved
-- It can go negative only if reservations exceed on-hand (data anomaly; flagged).
--
-- last_ledger_entry_id tracks the high-water mark for incremental rebuild.
-- projected_at records when this row was last updated.

CREATE TABLE item_on_hand (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT NOT NULL,
    item_id               UUID NOT NULL REFERENCES items(id),
    warehouse_id          UUID NOT NULL,

    -- Current stock quantities (always in whole units)
    quantity_on_hand      BIGINT NOT NULL DEFAULT 0,
    quantity_reserved     BIGINT NOT NULL DEFAULT 0,
    quantity_available    BIGINT GENERATED ALWAYS AS (quantity_on_hand - quantity_reserved) STORED,

    -- Valuation: total cost of on-hand stock at FIFO cost basis
    total_cost_minor      BIGINT NOT NULL DEFAULT 0 CHECK (total_cost_minor >= 0),
    currency              TEXT NOT NULL DEFAULT 'usd',

    -- Projection rebuild tracking
    last_ledger_entry_id  BIGINT REFERENCES inventory_ledger(id),
    projected_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at            TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT item_on_hand_unique UNIQUE (tenant_id, item_id, warehouse_id)
);

CREATE INDEX idx_on_hand_tenant_id ON item_on_hand(tenant_id);
CREATE INDEX idx_on_hand_item_wh   ON item_on_hand(item_id, warehouse_id);
-- Fast lookup of items with positive available stock
CREATE INDEX idx_on_hand_available ON item_on_hand(tenant_id, item_id, warehouse_id)
    WHERE quantity_available > 0;
