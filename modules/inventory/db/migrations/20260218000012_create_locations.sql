-- Inventory: Locations catalog
--
-- Represents physical or logical storage locations within a warehouse
-- (bins, shelves, zones, cold storage areas, etc.).
--
-- v1 design decisions:
--   - Location is optional (nullable) — existing flows remain unaffected.
--   - No WMS complexity (no path constraints, no putaway rules).
--   - Code is unique per (tenant, warehouse) — human-readable label.
--   - is_active for soft-delete (new movements should not target inactive locations).

CREATE TABLE locations (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    TEXT NOT NULL,
    warehouse_id UUID NOT NULL,
    code         TEXT NOT NULL,
    name         TEXT NOT NULL,
    description  TEXT,
    is_active    BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT locations_tenant_warehouse_code_unique
        UNIQUE (tenant_id, warehouse_id, code)
);

CREATE INDEX idx_locations_tenant_wh ON locations(tenant_id, warehouse_id);
CREATE INDEX idx_locations_tenant_active ON locations(tenant_id, warehouse_id)
    WHERE is_active = TRUE;
