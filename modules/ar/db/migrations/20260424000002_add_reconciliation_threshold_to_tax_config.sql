-- Add per-tenant reconciliation threshold to ar_tenant_tax_config (bd-jsbai)
--
-- Default 0.5% divergence triggers a flag. Operators can loosen or tighten
-- per-tenant via the tax-config admin API.

ALTER TABLE ar_tenant_tax_config
    ADD COLUMN IF NOT EXISTS reconciliation_threshold_pct NUMERIC(6,4) NOT NULL DEFAULT 0.005;

COMMENT ON COLUMN ar_tenant_tax_config.reconciliation_threshold_pct IS
    'Fraction divergence that triggers a reconciliation flag (0.005 = 0.5%)';
