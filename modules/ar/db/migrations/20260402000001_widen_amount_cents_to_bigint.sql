-- Widen amount_cents columns from INTEGER (i32) to BIGINT (i64).
-- INTEGER truncates at 2,147,483,647 cents (~$21.47M).
-- BIGINT supports up to ~$92 quadrillion — no practical limit.
--
-- bd-a924k: BUG: AR amount_cents is i32 — truncates at 21.47M

ALTER TABLE ar_invoices ALTER COLUMN amount_cents TYPE BIGINT;
ALTER TABLE ar_charges ALTER COLUMN amount_cents TYPE BIGINT;
ALTER TABLE ar_refunds ALTER COLUMN amount_cents TYPE BIGINT;
ALTER TABLE ar_disputes ALTER COLUMN amount_cents TYPE BIGINT;
ALTER TABLE ar_payment_allocations ALTER COLUMN amount_cents TYPE BIGINT;
ALTER TABLE ar_invoice_line_items ALTER COLUMN amount_cents TYPE BIGINT;
ALTER TABLE ar_tax_calculations ALTER COLUMN taxable_amount_cents TYPE BIGINT;
ALTER TABLE ar_tax_calculations ALTER COLUMN tax_amount_cents TYPE BIGINT;
ALTER TABLE ar_discount_applications ALTER COLUMN discount_amount_cents TYPE BIGINT;
