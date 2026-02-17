-- Reconciliation Matching Engine v1 (bd-2cn)
--
-- Append-only tables for deterministic payment ↔ invoice matching.
-- Match decisions are immutable once written — raw inputs are never mutated.

-- ============================================================
-- Reconciliation Runs
-- ============================================================

CREATE TABLE IF NOT EXISTS ar_recon_runs (
    id SERIAL PRIMARY KEY,
    recon_run_id UUID NOT NULL UNIQUE,
    app_id VARCHAR(50) NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'running',
    matching_strategy VARCHAR(50) NOT NULL DEFAULT 'deterministic_v1',
    payment_count INTEGER NOT NULL DEFAULT 0,
    invoice_count INTEGER NOT NULL DEFAULT 0,
    match_count INTEGER NOT NULL DEFAULT 0,
    exception_count INTEGER NOT NULL DEFAULT 0,
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at TIMESTAMP,
    correlation_id VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ar_recon_runs_app_id ON ar_recon_runs(app_id);
CREATE INDEX IF NOT EXISTS ar_recon_runs_status ON ar_recon_runs(status);
CREATE INDEX IF NOT EXISTS ar_recon_runs_started_at ON ar_recon_runs(started_at);

-- ============================================================
-- Reconciliation Matches (append-only)
-- ============================================================

CREATE TABLE IF NOT EXISTS ar_recon_matches (
    id SERIAL PRIMARY KEY,
    match_id UUID NOT NULL UNIQUE,
    recon_run_id UUID NOT NULL REFERENCES ar_recon_runs(recon_run_id),
    app_id VARCHAR(50) NOT NULL,
    payment_id VARCHAR(255) NOT NULL,
    invoice_id VARCHAR(255) NOT NULL,
    matched_amount_minor BIGINT NOT NULL,
    currency VARCHAR(3) NOT NULL,
    confidence_score NUMERIC(3,2) NOT NULL,
    match_method VARCHAR(50) NOT NULL,
    matched_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ar_recon_matches_recon_run_id ON ar_recon_matches(recon_run_id);
CREATE INDEX IF NOT EXISTS ar_recon_matches_app_id ON ar_recon_matches(app_id);
CREATE INDEX IF NOT EXISTS ar_recon_matches_payment_id ON ar_recon_matches(app_id, payment_id);
CREATE INDEX IF NOT EXISTS ar_recon_matches_invoice_id ON ar_recon_matches(app_id, invoice_id);

-- ============================================================
-- Reconciliation Exceptions (append-only)
-- ============================================================

CREATE TABLE IF NOT EXISTS ar_recon_exceptions (
    id SERIAL PRIMARY KEY,
    exception_id UUID NOT NULL UNIQUE,
    recon_run_id UUID NOT NULL REFERENCES ar_recon_runs(recon_run_id),
    app_id VARCHAR(50) NOT NULL,
    payment_id VARCHAR(255),
    invoice_id VARCHAR(255),
    exception_kind VARCHAR(50) NOT NULL,
    description TEXT NOT NULL,
    amount_minor BIGINT,
    currency VARCHAR(3),
    raised_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ar_recon_exceptions_recon_run_id ON ar_recon_exceptions(recon_run_id);
CREATE INDEX IF NOT EXISTS ar_recon_exceptions_app_id ON ar_recon_exceptions(app_id);
CREATE INDEX IF NOT EXISTS ar_recon_exceptions_kind ON ar_recon_exceptions(exception_kind);
