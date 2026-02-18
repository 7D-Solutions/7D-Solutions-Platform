-- AP Vendors Table
--
-- Master record for each vendor/supplier. Amounts use i64 minor units.
-- Append-friendly: updates change mutable fields but the record is the
-- canonical vendor identity anchor for all downstream AP tables.
--
-- Indexed for:
--   - Vendor lookup by (tenant_id, name) for duplicate detection
--   - Tenant-scoped listing (tenant_id)

CREATE TABLE vendors (
    vendor_id           UUID PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    name                TEXT NOT NULL,
    tax_id              TEXT,
    -- ISO 4217 currency code (e.g. "USD")
    currency            CHAR(3) NOT NULL,
    -- Net payment terms in calendar days (e.g. 30 for Net-30)
    payment_terms_days  INT NOT NULL CHECK (payment_terms_days >= 0),
    -- Preferred payment method: "ach", "wire", "check", etc.
    payment_method      TEXT,
    remittance_email    TEXT,
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Vendor lookup: find by name within a tenant (duplicate detection + search)
CREATE INDEX idx_vendors_tenant_name ON vendors (tenant_id, name);

-- Tenant scoped listing
CREATE INDEX idx_vendors_tenant_id ON vendors (tenant_id);

-- Only one active vendor per (tenant_id, name) — prevent accidental duplicates
CREATE UNIQUE INDEX idx_vendors_tenant_name_active
    ON vendors (tenant_id, name)
    WHERE is_active = TRUE;
