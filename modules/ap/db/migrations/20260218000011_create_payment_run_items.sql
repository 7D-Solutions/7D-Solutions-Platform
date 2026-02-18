-- Payment Run Items
--
-- Stores per-vendor planned payment detail for a payment run.
-- Each item groups bills from one vendor into a single payment amount.
--
-- Separating items from the payment_runs header allows:
--   - Per-vendor drill-down queries
--   - Partial run completion tracking
--   - Multi-vendor runs in a single batch

CREATE TABLE payment_run_items (
    id              BIGSERIAL PRIMARY KEY,
    run_id          UUID NOT NULL REFERENCES payment_runs (run_id),
    vendor_id       UUID NOT NULL,
    -- Array of bill UUIDs selected for payment to this vendor
    bill_ids        UUID[] NOT NULL,
    -- Total amount to pay this vendor (minor currency units, i64)
    amount_minor    BIGINT NOT NULL CHECK (amount_minor > 0),
    -- ISO 4217 (must match the parent run currency)
    currency        CHAR(3) NOT NULL,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_payment_run_items_run_id    ON payment_run_items (run_id);
CREATE INDEX idx_payment_run_items_vendor_id ON payment_run_items (vendor_id);
