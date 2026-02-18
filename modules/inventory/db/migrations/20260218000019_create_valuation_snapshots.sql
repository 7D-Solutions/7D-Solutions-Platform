-- Inventory: Valuation Snapshot Schema
--
-- Read-only reporting tables derived from remaining FIFO layers.
-- The builder (see bd-2k0i) populates these; no mutations happen here.
--
-- inventory_valuation_snapshots:
--   One row per (tenant, warehouse, optional location, as_of instant).
--   total_value_minor = sum of all line.total_value_minor under this snapshot.
--
-- inventory_valuation_lines:
--   One row per (item, optional location) under a snapshot.
--   total_value_minor = quantity_on_hand * unit_cost_minor (weighted-average
--   of remaining FIFO layers at as_of time).
--   quantity_on_hand and unit_cost_minor are snapshots of layer state at as_of.

-- ─── Snapshot header ────────────────────────────────────────────────────────

CREATE TABLE inventory_valuation_snapshots (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    warehouse_id        UUID NOT NULL,
    -- NULL means "all locations combined" (warehouse-level roll-up)
    location_id         UUID REFERENCES locations(id),
    -- Point-in-time for which FIFO layers were evaluated
    as_of               TIMESTAMP WITH TIME ZONE NOT NULL,
    -- Pre-computed sum of all line.total_value_minor; in minor currency units
    total_value_minor   BIGINT NOT NULL CHECK (total_value_minor >= 0),
    currency            TEXT NOT NULL DEFAULT 'usd',
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Query pattern: "give me the latest snapshot for tenant+warehouse before date X"
CREATE INDEX idx_val_snapshots_tenant_wh_as_of
    ON inventory_valuation_snapshots(tenant_id, warehouse_id, as_of DESC);

-- Optional: filter snapshots to a specific location within a warehouse
CREATE INDEX idx_val_snapshots_location
    ON inventory_valuation_snapshots(tenant_id, warehouse_id, location_id, as_of DESC)
    WHERE location_id IS NOT NULL;

-- ─── Per-item lines ──────────────────────────────────────────────────────────

CREATE TABLE inventory_valuation_lines (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    snapshot_id         UUID NOT NULL REFERENCES inventory_valuation_snapshots(id)
                            ON DELETE CASCADE,
    item_id             UUID NOT NULL REFERENCES items(id),
    warehouse_id        UUID NOT NULL,
    -- NULL when not tracking at bin/shelf level for this snapshot
    location_id         UUID REFERENCES locations(id),
    -- Remaining on-hand quantity at as_of time (sum of layer.quantity_remaining)
    quantity_on_hand    BIGINT NOT NULL CHECK (quantity_on_hand >= 0),
    -- Weighted-average unit cost across remaining FIFO layers at as_of
    unit_cost_minor     BIGINT NOT NULL CHECK (unit_cost_minor >= 0),
    -- quantity_on_hand * unit_cost_minor (pre-computed; avoids runtime multiply)
    total_value_minor   BIGINT NOT NULL CHECK (total_value_minor >= 0),
    currency            TEXT NOT NULL DEFAULT 'usd'
);

-- Fetch all lines for a snapshot (primary access pattern for reports)
CREATE INDEX idx_val_lines_snapshot
    ON inventory_valuation_lines(snapshot_id);

-- Fetch per-item history across snapshots (e.g. trending item value over time)
CREATE INDEX idx_val_lines_item
    ON inventory_valuation_lines(item_id, snapshot_id);

-- Fetch by item + location within a snapshot
CREATE INDEX idx_val_lines_item_location
    ON inventory_valuation_lines(snapshot_id, item_id, location_id)
    WHERE location_id IS NOT NULL;

-- Uniqueness: one line per (item, location) per snapshot.
-- Nullable location_id requires two partial unique indexes (same pattern as item_on_hand).
CREATE UNIQUE INDEX val_lines_null_loc
    ON inventory_valuation_lines(snapshot_id, item_id, warehouse_id)
    WHERE location_id IS NULL;

CREATE UNIQUE INDEX val_lines_with_loc
    ON inventory_valuation_lines(snapshot_id, item_id, warehouse_id, location_id)
    WHERE location_id IS NOT NULL;
