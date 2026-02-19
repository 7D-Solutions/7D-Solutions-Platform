-- Extend tenants schema: product_code, plan_code, trial/past_due statuses, app_id bridge
-- Phase 40: Tenant Control Plane foundation

-- ============================================================
-- ADD NEW COLUMNS
-- ============================================================

-- Product code identifies the purchased product (e.g. 'starter', 'professional', 'enterprise')
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS product_code TEXT;

-- Plan code identifies the billing plan within a product (e.g. 'monthly', 'annual')
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS plan_code TEXT;

-- app_id bridge: AR module is app_id-based; this column maps tenant to the AR app namespace
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS app_id VARCHAR(50);

-- Unique index on app_id (nullable values are excluded from uniqueness check by postgres)
CREATE UNIQUE INDEX IF NOT EXISTS tenants_app_id_unique ON tenants(app_id) WHERE app_id IS NOT NULL;

-- ============================================================
-- EXPAND STATUS CHECK CONSTRAINT
-- ============================================================

-- Drop existing constraint and re-add with trial and past_due added
-- 'trial': tenant is on a free trial period (access allowed, billing not yet started)
-- 'past_due': tenant has an overdue payment (access may be gated downstream)
ALTER TABLE tenants
    DROP CONSTRAINT IF EXISTS tenants_status_check;

ALTER TABLE tenants
    ADD CONSTRAINT tenants_status_check
    CHECK (status IN ('pending', 'provisioning', 'active', 'failed', 'suspended', 'deleted', 'trial', 'past_due'));

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON COLUMN tenants.product_code IS 'Purchased product identifier (e.g. starter, professional, enterprise)';
COMMENT ON COLUMN tenants.plan_code IS 'Billing plan within the product (e.g. monthly, annual)';
COMMENT ON COLUMN tenants.app_id IS 'Bridge to AR module app namespace (AR is app_id-based); unique per tenant';
