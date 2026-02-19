-- Bundle tables for Tenant Control Plane (Phase 40)
-- cp_bundles: product packaging definitions
-- cp_bundle_modules: modules included in each bundle (with version pins)
-- cp_tenant_bundle: tenant -> bundle assignment

-- ============================================================
-- cp_bundles: catalog of available product bundles
-- ============================================================

CREATE TABLE IF NOT EXISTS cp_bundles (
    bundle_id       UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    product_code    TEXT        NOT NULL,
    bundle_name     TEXT        NOT NULL,
    is_default      BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Only one default bundle per product_code
CREATE UNIQUE INDEX IF NOT EXISTS cp_bundles_product_default_unique
    ON cp_bundles(product_code)
    WHERE is_default = TRUE;

COMMENT ON TABLE  cp_bundles              IS 'Catalog of product bundle definitions (product packaging)';
COMMENT ON COLUMN cp_bundles.bundle_id    IS 'Unique bundle identifier';
COMMENT ON COLUMN cp_bundles.product_code IS 'Product this bundle belongs to (e.g. starter, professional, enterprise)';
COMMENT ON COLUMN cp_bundles.bundle_name  IS 'Human-readable bundle name';
COMMENT ON COLUMN cp_bundles.is_default   IS 'Whether this is the default bundle for its product_code';

-- ============================================================
-- cp_bundle_modules: modules included in a bundle
-- ============================================================

CREATE TABLE IF NOT EXISTS cp_bundle_modules (
    bundle_id       UUID        NOT NULL REFERENCES cp_bundles(bundle_id) ON DELETE CASCADE,
    module_code     TEXT        NOT NULL,
    module_version  TEXT        NOT NULL DEFAULT 'latest',
    PRIMARY KEY (bundle_id, module_code)
);

COMMENT ON TABLE  cp_bundle_modules                IS 'Module membership for each bundle, with version pins';
COMMENT ON COLUMN cp_bundle_modules.bundle_id      IS 'FK to cp_bundles';
COMMENT ON COLUMN cp_bundle_modules.module_code    IS 'Module identifier (e.g. ar, gl, payments, inventory)';
COMMENT ON COLUMN cp_bundle_modules.module_version IS 'Pinned module schema version; "latest" means always current';

-- ============================================================
-- cp_tenant_bundle: tenant -> bundle assignment (one active per tenant)
-- ============================================================

CREATE TABLE IF NOT EXISTS cp_tenant_bundle (
    tenant_id    UUID        PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    bundle_id    UUID        NOT NULL    REFERENCES cp_bundles(bundle_id),
    status       TEXT        NOT NULL    DEFAULT 'active'
                             CHECK (status IN ('active', 'in_transition')),
    effective_at TIMESTAMPTZ NOT NULL    DEFAULT NOW()
);

COMMENT ON TABLE  cp_tenant_bundle              IS 'Current bundle assignment per tenant (one row per tenant)';
COMMENT ON COLUMN cp_tenant_bundle.tenant_id    IS 'FK to tenants; each tenant has exactly one active bundle';
COMMENT ON COLUMN cp_tenant_bundle.bundle_id    IS 'FK to cp_bundles';
COMMENT ON COLUMN cp_tenant_bundle.status       IS 'active = fully on this bundle; in_transition = mid-upgrade/downgrade';
COMMENT ON COLUMN cp_tenant_bundle.effective_at IS 'When this bundle assignment took effect';
