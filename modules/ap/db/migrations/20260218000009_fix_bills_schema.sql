-- Migration: align vendor_bills status constraint + add FX metadata + fix quantity type
--
-- bd-332p: Vendor bill creation (unmatched) + due date calc + multi-currency fields
--
-- Changes:
--   1. Rename initial status 'pending' -> 'open'; add 'partially_paid'
--      Status machine: open -> approved -> partially_paid -> paid -> voided
--      (also keeps 'matched' for the future 3-way match engine)
--   2. Add fx_rate_id column for multi-currency FX metadata
--      (references GL fx_rates row UUID; NULL when bill currency = functional currency)
--   3. Change bill_lines.quantity from NUMERIC(18,6) to DOUBLE PRECISION
--      to avoid bigdecimal dependency and align with f64 in event contracts

-- =============================================================================
-- 1. Fix vendor_bills status constraint
-- =============================================================================

-- Drop the inline check constraint by finding its name in the pg catalog
DO $$
DECLARE v_name TEXT;
BEGIN
    SELECT conname INTO v_name
    FROM pg_constraint
    WHERE conrelid = 'vendor_bills'::regclass
      AND contype = 'c'
      AND conname LIKE '%status%';
    IF v_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE vendor_bills DROP CONSTRAINT %I', v_name);
    END IF;
END $$;

-- Migrate any existing 'pending' rows to 'open'
UPDATE vendor_bills SET status = 'open' WHERE status = 'pending';

-- Add updated constraint (named) with full status machine
ALTER TABLE vendor_bills
    ADD CONSTRAINT vendor_bills_status_check
    CHECK (status IN ('open', 'matched', 'approved', 'partially_paid', 'paid', 'voided'));

-- Change default to 'open'
ALTER TABLE vendor_bills
    ALTER COLUMN status SET DEFAULT 'open';

-- =============================================================================
-- 2. Add FX rate reference column
-- =============================================================================

ALTER TABLE vendor_bills
    ADD COLUMN IF NOT EXISTS fx_rate_id UUID;

COMMENT ON COLUMN vendor_bills.fx_rate_id IS
    'UUID of the GL fx_rates row used for FX conversion. '
    'NULL when bill currency matches the tenant functional currency. '
    'Reuses existing GL FX infrastructure — do not store rates here.';

-- =============================================================================
-- 3. Fix bill_lines.quantity: NUMERIC(18,6) -> DOUBLE PRECISION
-- =============================================================================

-- Drop the quantity check constraint first (recreated below)
DO $$
DECLARE v_name TEXT;
BEGIN
    SELECT conname INTO v_name
    FROM pg_constraint
    WHERE conrelid = 'bill_lines'::regclass
      AND contype = 'c'
      AND conname LIKE '%quantity%';
    IF v_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE bill_lines DROP CONSTRAINT %I', v_name);
    END IF;
END $$;

ALTER TABLE bill_lines
    ALTER COLUMN quantity TYPE DOUBLE PRECISION USING quantity::DOUBLE PRECISION;

-- Re-add the quantity > 0 check
ALTER TABLE bill_lines
    ADD CONSTRAINT bill_lines_quantity_check CHECK (quantity > 0);
