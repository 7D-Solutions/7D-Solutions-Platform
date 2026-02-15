-- AR Invoice Attempts Ledger
-- Phase 15: Deterministic retry tracking and exactly-once side effect enforcement
-- UNIQUE constraint ensures no duplicate attempts (tenant_id, invoice_id, attempt_no)

-- Attempt Status Enum
CREATE TYPE ar_invoice_attempt_status AS ENUM (
    'attempting',
    'succeeded',
    'failed_retry',
    'failed_final'
);

-- Invoice Attempts Table
CREATE TABLE ar_invoice_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id VARCHAR(50) NOT NULL,
    invoice_id INTEGER NOT NULL REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    attempt_no INTEGER NOT NULL CHECK (attempt_no >= 0),
    status ar_invoice_attempt_status NOT NULL,
    attempted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    failure_code VARCHAR(50),
    failure_message TEXT,
    idempotency_key VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- CRITICAL: Exactly-once enforcement per Phase 15 Coordination Rules
    CONSTRAINT unique_invoice_attempt UNIQUE (app_id, invoice_id, attempt_no)
);

-- Indexes for query performance
CREATE INDEX ar_invoice_attempts_app_invoice ON ar_invoice_attempts(app_id, invoice_id);
CREATE INDEX ar_invoice_attempts_status ON ar_invoice_attempts(status);
CREATE INDEX ar_invoice_attempts_attempted_at ON ar_invoice_attempts(attempted_at);

-- Comments
COMMENT ON TABLE ar_invoice_attempts IS 'Phase 15: Deterministic attempt ledger for invoice payment collection. UNIQUE constraint (app_id, invoice_id, attempt_no) enforces exactly-once side effects.';
COMMENT ON CONSTRAINT unique_invoice_attempt ON ar_invoice_attempts IS 'Phase 15 Exactly-Once Rule: Prevents duplicate attempt creation for same (tenant, invoice, attempt_no) tuple.';
