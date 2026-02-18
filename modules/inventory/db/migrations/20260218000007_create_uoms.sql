-- UoM catalog (tenant-scoped)
--
-- uoms: defines available units of measure per tenant (e.g. 'ea', 'kg', 'box')
-- items.base_uom_id: FK to the item's canonical stock unit
-- item_uom_conversions: factor-based conversion between UoMs for a specific item
--
-- Invariants (enforced by constraints):
--   - UoM code is unique per tenant
--   - Conversion factor must be positive
--   - (item_id, from_uom_id, to_uom_id) is unique
--   - from_uom_id != to_uom_id (no self-conversions)

CREATE TABLE uoms (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  TEXT NOT NULL,
    code       TEXT NOT NULL,  -- e.g. 'ea', 'kg', 'ltr', 'box'
    name       TEXT NOT NULL,  -- e.g. 'Each', 'Kilogram', 'Box of 12'
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT uoms_tenant_code_unique UNIQUE (tenant_id, code)
);

CREATE INDEX idx_uoms_tenant_id ON uoms(tenant_id);

-- Attach base UoM to items (nullable — pre-existing items may not have one)
ALTER TABLE items
    ADD COLUMN base_uom_id UUID REFERENCES uoms(id) ON DELETE SET NULL;

-- Item-level UoM conversion factors
-- factor: multiply from_uom quantity by factor to get to_uom quantity
-- e.g. 1 box = 12 ea  →  from=box, to=ea, factor=12
CREATE TABLE item_uom_conversions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   TEXT NOT NULL,
    item_id     UUID NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    from_uom_id UUID NOT NULL REFERENCES uoms(id),
    to_uom_id   UUID NOT NULL REFERENCES uoms(id),
    factor      DOUBLE PRECISION NOT NULL,
    created_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- One conversion per direction per item (no duplicate paths)
    CONSTRAINT item_uom_conversions_unique        UNIQUE (item_id, from_uom_id, to_uom_id),
    -- Factor must be strictly positive
    CONSTRAINT item_uom_conversions_factor_pos    CHECK  (factor > 0),
    -- Self-conversions are meaningless
    CONSTRAINT item_uom_conversions_no_self       CHECK  (from_uom_id != to_uom_id)
);

CREATE INDEX idx_item_uom_conversions_item_id   ON item_uom_conversions(item_id);
CREATE INDEX idx_item_uom_conversions_tenant_id ON item_uom_conversions(tenant_id);
