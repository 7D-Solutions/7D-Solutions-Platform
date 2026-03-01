-- Subscription Invoice Generation Attempts Ledger
-- Phase 15 bd-184: Cycle gating for exactly-once invoice generation
-- UNIQUE constraint ensures no duplicate invoices per subscription cycle

-- Invoice Generation Attempt Status Enum
CREATE TYPE subscription_invoice_attempt_status AS ENUM (
    'attempting',
    'succeeded',
    'failed_retry',
    'failed_final'
);

-- Subscription Invoice Attempts Table
CREATE TABLE subscription_invoice_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id VARCHAR(255) NOT NULL,
    subscription_id UUID NOT NULL REFERENCES subscriptions(id) ON DELETE RESTRICT,
    cycle_key VARCHAR(20) NOT NULL,  -- Format: YYYY-MM (e.g., "2026-02")
    cycle_start DATE NOT NULL,
    cycle_end DATE NOT NULL,
    status subscription_invoice_attempt_status NOT NULL,
    ar_invoice_id INTEGER,  -- NULL if creation failed
    attempted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    failure_code VARCHAR(50),
    failure_message TEXT,
    idempotency_key VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- CRITICAL: Exactly-once invoice per subscription per cycle
    CONSTRAINT unique_subscription_cycle_invoice UNIQUE (tenant_id, subscription_id, cycle_key)
);

-- Indexes for query performance
CREATE INDEX subscription_invoice_attempts_tenant_subscription ON subscription_invoice_attempts(tenant_id, subscription_id);
CREATE INDEX subscription_invoice_attempts_status ON subscription_invoice_attempts(status);
CREATE INDEX subscription_invoice_attempts_cycle ON subscription_invoice_attempts(cycle_start, cycle_end);
CREATE INDEX subscription_invoice_attempts_attempted_at ON subscription_invoice_attempts(attempted_at);

-- Comments
COMMENT ON TABLE subscription_invoice_attempts IS 'Phase 15 bd-184: Deterministic attempt ledger for subscription invoice generation. UNIQUE constraint (tenant_id, subscription_id, cycle_key) enforces exactly-once invoice per cycle.';
COMMENT ON CONSTRAINT unique_subscription_cycle_invoice ON subscription_invoice_attempts IS 'Phase 15 Exactly-Once Rule: Prevents duplicate invoice creation for same subscription cycle under replay/concurrency.';
COMMENT ON COLUMN subscription_invoice_attempts.cycle_key IS 'Normalized cycle identifier (YYYY-MM format). Ensures cycle boundaries are stable and deterministic.';
