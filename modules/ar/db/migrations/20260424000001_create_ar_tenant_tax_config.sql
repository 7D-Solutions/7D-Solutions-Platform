-- ar_tenant_tax_config: Per-tenant tax calculation source configuration (bd-kkhf4)
--
-- Default: external_accounting_software — preserves current behavior where QBO AST computes tax.
-- Set to 'platform' to use the platform tax provider (local, zero, or avalara).
--
-- config_version: monotonically incremented on every SET. Used to version tax cache entries
-- so that config changes invalidate cached quotes for in-flight invoice batches.

CREATE TABLE IF NOT EXISTS ar_tenant_tax_config (
    tenant_id UUID PRIMARY KEY,
    tax_calculation_source TEXT NOT NULL
        CHECK (tax_calculation_source IN ('platform', 'external_accounting_software'))
        DEFAULT 'external_accounting_software',
    provider_name TEXT NOT NULL DEFAULT 'local'
        CHECK (provider_name IN ('local', 'zero', 'avalara')),
    config_version BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by UUID NOT NULL
);

COMMENT ON TABLE ar_tenant_tax_config IS
    'Per-tenant tax calculation source: platform provider or external accounting software';
COMMENT ON COLUMN ar_tenant_tax_config.tax_calculation_source IS
    'platform = AR computes via provider; external_accounting_software = QBO/other computes';
COMMENT ON COLUMN ar_tenant_tax_config.provider_name IS
    'Active provider when source=platform: local (deterministic), zero (no tax), avalara (AvaTax)';
COMMENT ON COLUMN ar_tenant_tax_config.config_version IS
    'Monotonically incremented on each mutation; embedded in tax cache idempotency keys';
