-- Payment allocation table for explicit, replay-safe allocation tracking
--
-- Phase 22 (bd-14f): Partial payment allocation model (FIFO default)
--
-- Design:
-- - Each row records one payment->invoice allocation
-- - FIFO strategy: oldest due invoices allocated first
-- - Idempotent: unique idempotency_key prevents duplicate allocations on retry
-- - Sum of allocations for a payment should equal or be less than payment amount
-- - Sum of allocations for an invoice should equal or be less than invoice amount

CREATE TABLE IF NOT EXISTS ar_payment_allocations (
    id SERIAL PRIMARY KEY,
    app_id VARCHAR(50) NOT NULL,
    payment_id VARCHAR(255) NOT NULL,
    invoice_id INTEGER NOT NULL REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    allocated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    strategy VARCHAR(50) NOT NULL DEFAULT 'fifo',
    idempotency_key VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_payment_allocation_idempotency UNIQUE (idempotency_key)
);

-- Query pattern: "all allocations for a payment"
CREATE INDEX IF NOT EXISTS idx_payment_allocations_payment
    ON ar_payment_allocations(app_id, payment_id);

-- Query pattern: "all allocations against an invoice" (for remaining balance calc)
CREATE INDEX IF NOT EXISTS idx_payment_allocations_invoice
    ON ar_payment_allocations(invoice_id);

-- Query pattern: "allocations for a customer's invoices in date order"
CREATE INDEX IF NOT EXISTS idx_payment_allocations_app_id
    ON ar_payment_allocations(app_id);

COMMENT ON TABLE ar_payment_allocations IS 'Explicit payment-to-invoice allocation rows (Phase 22). FIFO default strategy. Idempotent via unique idempotency_key.';
