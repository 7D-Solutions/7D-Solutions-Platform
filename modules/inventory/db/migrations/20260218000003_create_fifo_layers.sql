-- Inventory: FIFO Cost Layers + Consumption Records
--
-- inventory_layers:
--   Each row is a receipt batch (one per 'received' ledger entry).
--   FIFO selection is deterministic via ORDER BY (received_at, ledger_entry_id ASC).
--   ledger_entry_id is BIGSERIAL so it is globally monotonic — stable tie-breaker.
--   quantity_remaining decreases as layers are consumed; never goes below 0.
--
-- layer_consumptions:
--   Append-only log of each partial or full layer deduction.
--   Links an issue ledger entry to the specific layer(s) it consumed.
--   Sum of layer_consumptions.quantity_consumed for a layer_id
--   = quantity_received - quantity_remaining.

-- FIFO cost layers (one row per receipt batch)
CREATE TABLE inventory_layers (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    item_id             UUID NOT NULL REFERENCES items(id),
    warehouse_id        UUID NOT NULL,
    -- Back-reference to the 'received' ledger entry that created this layer
    -- Also serves as the FIFO tie-breaker (monotonically increasing)
    ledger_entry_id     BIGINT NOT NULL UNIQUE REFERENCES inventory_ledger(id),
    -- FIFO primary sort key
    received_at         TIMESTAMP WITH TIME ZONE NOT NULL,
    quantity_received   BIGINT NOT NULL CHECK (quantity_received > 0),
    quantity_remaining  BIGINT NOT NULL CHECK (quantity_remaining >= 0),
    unit_cost_minor     BIGINT NOT NULL CHECK (unit_cost_minor >= 0),
    currency            TEXT NOT NULL DEFAULT 'usd',
    -- Set when layer is fully consumed (quantity_remaining = 0)
    exhausted_at        TIMESTAMP WITH TIME ZONE,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT layers_remaining_lte_received
        CHECK (quantity_remaining <= quantity_received)
);

-- Deterministic FIFO ordering index:
-- oldest received_at first; tie-break by ledger_entry_id (monotonic BIGSERIAL)
-- Partial index excludes exhausted layers from FIFO selection scans.
CREATE INDEX idx_layers_fifo_select ON inventory_layers(item_id, warehouse_id, received_at, ledger_entry_id)
    WHERE quantity_remaining > 0;

CREATE INDEX idx_layers_tenant_item_wh ON inventory_layers(tenant_id, item_id, warehouse_id);

-- Layer consumption records (append-only; sum = total consumed from a layer)
CREATE TABLE layer_consumptions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The FIFO layer being consumed
    layer_id            UUID NOT NULL REFERENCES inventory_layers(id),
    -- The 'issued' ledger entry that triggered this consumption
    ledger_entry_id     BIGINT NOT NULL REFERENCES inventory_ledger(id),
    quantity_consumed   BIGINT NOT NULL CHECK (quantity_consumed > 0),
    -- Snapshot of cost at time of consumption (from layer)
    unit_cost_minor     BIGINT NOT NULL CHECK (unit_cost_minor >= 0),
    consumed_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_consumptions_layer_id    ON layer_consumptions(layer_id);
CREATE INDEX idx_consumptions_ledger_entry ON layer_consumptions(ledger_entry_id);
