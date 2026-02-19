-- cp_entitlements: per-tenant entitlements for concurrency enforcement
-- Phase 40: Tenant Control Plane
--
-- identity-auth reads this table to determine concurrent_user_limit.
-- Missing row = deny (fail-closed). Row must be seeded when tenant is provisioned.

CREATE TABLE cp_entitlements (
    -- tenant_id is the PK; one entitlements row per tenant
    tenant_id UUID PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,

    -- plan_code mirrors tenants.plan_code; stored here for fast reads without joins
    plan_code TEXT NOT NULL,

    -- Maximum concurrent sessions allowed for this tenant.
    -- CHECK > 0 ensures it can never be zero or negative.
    concurrent_user_limit INT NOT NULL CHECK (concurrent_user_limit > 0),

    -- When these entitlements take effect (e.g. after trial upgrade)
    effective_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Last modification timestamp
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Index for common lookup by tenant
CREATE INDEX cp_entitlements_tenant_id ON cp_entitlements(tenant_id);

COMMENT ON TABLE cp_entitlements IS 'Per-tenant entitlements for concurrency enforcement; queried by identity-auth';
COMMENT ON COLUMN cp_entitlements.tenant_id IS 'FK to tenants; one row per tenant';
COMMENT ON COLUMN cp_entitlements.plan_code IS 'Plan code at time of entitlement provisioning (e.g. monthly, annual)';
COMMENT ON COLUMN cp_entitlements.concurrent_user_limit IS 'Max concurrent sessions allowed; identity-auth enforces this limit';
COMMENT ON COLUMN cp_entitlements.effective_at IS 'Timestamp when these entitlements became effective';
COMMENT ON COLUMN cp_entitlements.updated_at IS 'Last update timestamp (updated on plan changes)';
