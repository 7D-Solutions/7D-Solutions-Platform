-- Payment Attempts Ledger
-- Phase 15: Deterministic retry tracking, exactly-once side effects, and UNKNOWN protocol
-- UNIQUE constraint ensures no duplicate attempts (tenant_id, payment_id, attempt_no)

-- Payment Attempt Status Enum
CREATE TYPE payment_attempt_status AS ENUM (
    'attempting',
    'succeeded',
    'failed_retry',
    'failed_final',
    'unknown'  -- Phase 15 UNKNOWN protocol: blocks retries and suspension
);

-- Payment Attempts Table
CREATE TABLE payment_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id VARCHAR(50) NOT NULL,
    payment_id UUID NOT NULL,
    invoice_id VARCHAR(255) NOT NULL,  -- AR invoice reference
    attempt_no INTEGER NOT NULL CHECK (attempt_no >= 0),
    status payment_attempt_status NOT NULL,
    attempted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    processor_payment_id VARCHAR(255),  -- PSP reference (e.g., Tilled payment ID)
    payment_method_ref VARCHAR(255),
    failure_code VARCHAR(50),
    failure_message TEXT,
    webhook_event_id VARCHAR(255),  -- Correlation to webhook that updated this attempt
    idempotency_key VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- CRITICAL: Exactly-once enforcement per Phase 15 Coordination Rules
    CONSTRAINT unique_payment_attempt UNIQUE (app_id, payment_id, attempt_no)
);

-- Indexes for query performance
CREATE INDEX payment_attempts_app_payment ON payment_attempts(app_id, payment_id);
CREATE INDEX payment_attempts_invoice ON payment_attempts(invoice_id);
CREATE INDEX payment_attempts_status ON payment_attempts(status);
CREATE INDEX payment_attempts_attempted_at ON payment_attempts(attempted_at);
CREATE INDEX payment_attempts_webhook_event ON payment_attempts(webhook_event_id);
CREATE INDEX payment_attempts_processor_payment ON payment_attempts(processor_payment_id);

-- Comments
COMMENT ON TABLE payment_attempts IS 'Phase 15: Deterministic attempt ledger for payment processing. UNIQUE constraint (app_id, payment_id, attempt_no) enforces exactly-once side effects. Supports UNKNOWN protocol for webhook reconciliation.';
COMMENT ON CONSTRAINT unique_payment_attempt ON payment_attempts IS 'Phase 15 Exactly-Once Rule: Prevents duplicate attempt creation for same (tenant, payment, attempt_no) tuple.';
COMMENT ON COLUMN payment_attempts.status IS 'Phase 15 state machine: attempting → succeeded/failed_retry/failed_final/unknown. UNKNOWN blocks retries and subscription suspension until reconciled.';
