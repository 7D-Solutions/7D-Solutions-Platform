-- Payment Run Executions
--
-- Records the outcome of each payment instruction sent to the Payments module
-- for a payment run item (one per vendor).
--
-- Used for:
--   - Idempotency: UNIQUE (run_id, item_id) ensures one execution per item
--   - Audit: preserves payment_id returned by the Payments module
--   - Reconciliation: links AP payment run ↔ Payments disbursement record

CREATE TABLE IF NOT EXISTS payment_run_executions (
    id              BIGSERIAL PRIMARY KEY,
    run_id          UUID NOT NULL REFERENCES payment_runs (run_id),
    item_id         BIGINT NOT NULL REFERENCES payment_run_items (id),
    -- Identifier assigned by the Payments disbursement module (or derived locally)
    payment_id      UUID NOT NULL,
    vendor_id       UUID NOT NULL,
    -- Actual amount disbursed in minor currency units
    amount_minor    BIGINT NOT NULL CHECK (amount_minor > 0),
    currency        CHAR(3) NOT NULL,
    -- "success" or "failed"
    status          TEXT NOT NULL DEFAULT 'success'
                        CHECK (status IN ('success', 'failed')),
    failure_reason  TEXT,
    executed_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Idempotency: exactly one execution record per item in a run
CREATE UNIQUE INDEX IF NOT EXISTS uq_payment_run_executions_item
    ON payment_run_executions (run_id, item_id);

-- Lookup by run (for status reporting)
CREATE INDEX IF NOT EXISTS idx_payment_run_executions_run_id
    ON payment_run_executions (run_id);
