-- Inventory: Lot Tracking Tables
--
-- inventory_lots:
--   One row per named lot per item per tenant.
--   A lot groups a quantity of an item received together (same origin, expiry, etc.).
--   Lot codes are unique per (tenant_id, item_id) — two items can share codes.
--
-- FIFO layer association:
--   inventory_layers.lot_id references inventory_lots(id).
--   Non-lot-tracked items leave lot_id NULL.
--   Lot-tracked receipts create a lot (or reference an existing one) and bind the
--   resulting FIFO layer to it.

-- Lots catalog (one row per named lot per tenant+item)
CREATE TABLE inventory_lots (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   TEXT NOT NULL,
    item_id     UUID NOT NULL REFERENCES items(id),
    lot_code    TEXT NOT NULL,
    -- Optional free-form metadata (expiry date, supplier batch, etc.)
    attributes  JSONB,
    created_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Lot codes are unique within a tenant+item scope
    CONSTRAINT inventory_lots_unique_code UNIQUE (tenant_id, item_id, lot_code)
);

CREATE INDEX idx_lots_tenant_item    ON inventory_lots(tenant_id, item_id);
CREATE INDEX idx_lots_tenant_code    ON inventory_lots(tenant_id, lot_code);

-- Associate FIFO layers with lots (nullable; only set for lot-tracked items)
ALTER TABLE inventory_layers
    ADD COLUMN lot_id UUID REFERENCES inventory_lots(id);

-- Fast lookup: all layers belonging to a lot
CREATE INDEX idx_layers_lot_id ON inventory_layers(lot_id)
    WHERE lot_id IS NOT NULL;
