-- Inventory: Reorder Policies (Low-Stock Configuration)
--
-- Stores per-item (and optionally per-location) reorder thresholds.
-- This is configuration only — signal emission lives in a later bead.
--
-- Design decisions:
--   - reorder_point: QOH level at which a restock is triggered
--   - safety_stock:  minimum buffer quantity to keep on hand
--   - max_qty:       optional upper bound (for order-to-max workflows)
--   - location_id is optional; NULL means the policy applies to the item globally
--   - Unique constraint: one policy per (tenant, item) globally AND
--     one policy per (tenant, item, location) when location is specified
--   - Audit fields: created_by / updated_by are caller-supplied strings
--   - Quantities are BIGINT (whole units), consistent with the rest of the module
--
-- Invariants enforced by partial unique indexes:
--   - UNIQUE (tenant_id, item_id) WHERE location_id IS NULL
--   - UNIQUE (tenant_id, item_id, location_id) WHERE location_id IS NOT NULL

CREATE TABLE reorder_policies (
    id             UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id      TEXT    NOT NULL,
    item_id        UUID    NOT NULL REFERENCES items(id),
    location_id    UUID    REFERENCES locations(id),
    -- thresholds (whole units, >= 0)
    reorder_point  BIGINT  NOT NULL CHECK (reorder_point >= 0),
    safety_stock   BIGINT  NOT NULL CHECK (safety_stock >= 0),
    max_qty        BIGINT           CHECK (max_qty IS NULL OR max_qty >= 0),
    -- optional free-text annotation
    notes          TEXT,
    -- audit
    created_by     TEXT    NOT NULL DEFAULT 'system',
    updated_by     TEXT    NOT NULL DEFAULT 'system',
    created_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- One global policy per (tenant, item) when no location is specified
CREATE UNIQUE INDEX uq_reorder_policies_item_global
    ON reorder_policies(tenant_id, item_id)
    WHERE location_id IS NULL;

-- One location-scoped policy per (tenant, item, location)
CREATE UNIQUE INDEX uq_reorder_policies_item_location
    ON reorder_policies(tenant_id, item_id, location_id)
    WHERE location_id IS NOT NULL;

-- Lookup index for listing all policies on an item
CREATE INDEX idx_reorder_policies_tenant_item
    ON reorder_policies(tenant_id, item_id);
