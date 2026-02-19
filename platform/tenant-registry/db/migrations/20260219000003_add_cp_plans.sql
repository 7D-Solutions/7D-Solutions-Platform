-- Platform control-plane plan pricing table
-- Provides a canonical, DB-editable source of truth for platform billing plan fees.
-- Phase 40 (bd-pxo5): replaces the hardcoded plan_fee_cents lookup in control-plane.
--
-- plan_code: the product tier identifier stored in tenants.product_code
-- monthly_fee_minor: monthly fee in minor units (cents for USD)

CREATE TABLE IF NOT EXISTS cp_plans (
    plan_code    TEXT     PRIMARY KEY,
    name         TEXT     NOT NULL,
    monthly_fee_minor BIGINT  NOT NULL,
    currency     CHAR(3)  NOT NULL DEFAULT 'usd'
);

COMMENT ON TABLE  cp_plans IS 'Platform billing plan definitions — source of truth for monthly fees per product tier';
COMMENT ON COLUMN cp_plans.plan_code          IS 'Product tier identifier (e.g. starter, professional, enterprise)';
COMMENT ON COLUMN cp_plans.monthly_fee_minor  IS 'Monthly fee in minor units (cents for USD)';
COMMENT ON COLUMN cp_plans.currency           IS 'ISO 4217 currency code (3 chars, lowercase)';

-- ============================================================
-- DEFAULT PLANS
-- ============================================================

INSERT INTO cp_plans (plan_code, name, monthly_fee_minor, currency) VALUES
    ('starter',      'Starter',      2900,  'usd'),
    ('professional', 'Professional', 7900,  'usd'),
    ('enterprise',   'Enterprise',   29900, 'usd')
ON CONFLICT (plan_code) DO NOTHING;
