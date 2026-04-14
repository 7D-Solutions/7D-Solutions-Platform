-- Feature flags: per-tenant and global flag storage.
--
-- Per-tenant rows override the global row for the same flag_name.
-- Lookup order: per-tenant → global → false (absent = disabled).
--
-- NULL tenant_id means the flag applies globally to all tenants.
-- A non-NULL tenant_id means the flag applies only to that tenant.

CREATE TABLE IF NOT EXISTS feature_flags (
    flag_name   TEXT        NOT NULL,
    tenant_id   UUID,
    enabled     BOOLEAN     NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One global row per flag name (tenant_id IS NULL).
CREATE UNIQUE INDEX IF NOT EXISTS feature_flags_global_uq
    ON feature_flags (flag_name)
    WHERE tenant_id IS NULL;

-- One per-tenant row per (flag_name, tenant_id) pair.
CREATE UNIQUE INDEX IF NOT EXISTS feature_flags_tenant_uq
    ON feature_flags (flag_name, tenant_id)
    WHERE tenant_id IS NOT NULL;

COMMENT ON TABLE feature_flags IS
    'Feature flags: per-tenant overrides take precedence over global flags.';
