-- Extend cp_plans with BFF-required columns
-- Phase 42 (bd-8zo2): plan catalog route needs included_seats, status, pricing_model

ALTER TABLE cp_plans ADD COLUMN IF NOT EXISTS included_seats  INTEGER DEFAULT 5;
ALTER TABLE cp_plans ADD COLUMN IF NOT EXISTS status          TEXT    DEFAULT 'active';
ALTER TABLE cp_plans ADD COLUMN IF NOT EXISTS pricing_model   TEXT    DEFAULT 'flat_monthly';

COMMENT ON COLUMN cp_plans.included_seats  IS 'Number of seats included at base price';
COMMENT ON COLUMN cp_plans.status          IS 'Plan availability status: active | archived';
COMMENT ON COLUMN cp_plans.pricing_model   IS 'Billing model: flat_monthly | per_seat | tiered';

-- Backfill existing rows with plan-specific defaults
UPDATE cp_plans SET included_seats = 5,   pricing_model = 'flat'     WHERE plan_code = 'starter';
UPDATE cp_plans SET included_seats = 25,  pricing_model = 'per_seat' WHERE plan_code = 'professional';
UPDATE cp_plans SET included_seats = 100, pricing_model = 'tiered'   WHERE plan_code = 'enterprise';
