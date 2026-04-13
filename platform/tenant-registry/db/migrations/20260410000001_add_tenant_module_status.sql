-- Per-tenant, per-module provisioning status tracking
--
-- Tracks the outcome of provisioning each individual module for a tenant.
-- Allows partial failure: a tenant can have some modules ready and some failed,
-- with the failed ones retryable without re-running the successful ones.

CREATE TABLE IF NOT EXISTS cp_tenant_module_status (
    tenant_id   UUID        NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    module_code TEXT        NOT NULL,
    status      TEXT        NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'provisioning', 'ready', 'failed')),
    error_msg   TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, module_code)
);

CREATE INDEX IF NOT EXISTS cp_tenant_module_status_tenant
    ON cp_tenant_module_status(tenant_id);

CREATE INDEX IF NOT EXISTS cp_tenant_module_status_failed
    ON cp_tenant_module_status(status)
    WHERE status = 'failed';

COMMENT ON TABLE  cp_tenant_module_status             IS 'Per-tenant, per-module provisioning status. Enables partial failure and independent retry.';
COMMENT ON COLUMN cp_tenant_module_status.tenant_id   IS 'FK to tenants';
COMMENT ON COLUMN cp_tenant_module_status.module_code IS 'Module identifier matching cp_bundle_modules.module_code';
COMMENT ON COLUMN cp_tenant_module_status.status      IS 'pending=not started, provisioning=in progress, ready=success, failed=error (retryable)';
COMMENT ON COLUMN cp_tenant_module_status.error_msg   IS 'Last error message when status=failed';
