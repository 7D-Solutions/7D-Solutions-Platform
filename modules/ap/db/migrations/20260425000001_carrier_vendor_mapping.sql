-- AP Carrier-to-Vendor Mapping
--
-- Maps a carrier code (e.g. "ups", "fedex") to the corresponding AP vendor
-- for a given tenant. Used by the shipping cost consumer to route carrier
-- obligations to the correct vendor record for AP matching.
--
-- Seeded per-tenant when the tenant configures their carrier account.
-- The setup UX is a vertical concern; the platform exposes the table directly.

CREATE TABLE ap_carrier_vendor_mapping (
    tenant_id         TEXT NOT NULL,
    carrier_code      TEXT NOT NULL,
    vendor_id         UUID NOT NULL REFERENCES vendors (vendor_id),
    -- Default GL account code for shipping expense lines (e.g. "6200" = freight)
    default_gl_account_code TEXT NOT NULL DEFAULT '6200',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, carrier_code)
);

-- Index for vendor-scoped lookups (e.g. "which carriers does this vendor service?")
CREATE INDEX idx_carrier_vendor_mapping_vendor
    ON ap_carrier_vendor_mapping (vendor_id);
