-- Inventory: Item Master (SKU catalog)
--
-- Each row is a unique item (SKU) per tenant.
-- Carries GL account references for double-entry booking:
--   inventory_account_ref  → Inventory asset (debit on receipt, credit on issue)
--   cogs_account_ref       → COGS expense (debit on issue)
--   variance_account_ref   → Purchase price variance (PPV)
--
-- Unique constraint: (tenant_id, sku) — one SKU per tenant

CREATE TABLE items (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id              TEXT NOT NULL,
    sku                    TEXT NOT NULL,
    name                   TEXT NOT NULL,
    description            TEXT,
    -- GL account references (match gl.accounts.account_ref)
    inventory_account_ref  TEXT NOT NULL,
    cogs_account_ref       TEXT NOT NULL,
    variance_account_ref   TEXT NOT NULL,
    -- Unit of measure (e.g. 'ea', 'kg', 'ltr')
    uom                    TEXT NOT NULL DEFAULT 'ea',
    active                 BOOLEAN NOT NULL DEFAULT TRUE,
    created_at             TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at             TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT items_tenant_sku_unique UNIQUE (tenant_id, sku)
);

CREATE INDEX idx_items_tenant_id ON items(tenant_id);
CREATE INDEX idx_items_tenant_active ON items(tenant_id, active);
