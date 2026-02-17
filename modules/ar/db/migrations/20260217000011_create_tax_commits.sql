-- Tax Commit/Void Ledger (Phase 23b, bd-3fy)
--
-- Tracks the lifecycle of tax commitments tied to invoices.
-- UNIQUE(app_id, invoice_id) enforces exactly-once commit per invoice.
-- Status transitions: pending_commit → committed → voided
-- Retries on finalize hit UNIQUE constraint → idempotent no-op.

CREATE TABLE IF NOT EXISTS ar_tax_commits (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id VARCHAR(50) NOT NULL,
    invoice_id VARCHAR(255) NOT NULL,
    customer_id VARCHAR(255) NOT NULL,
    provider VARCHAR(50) NOT NULL,
    provider_quote_ref VARCHAR(255) NOT NULL,
    provider_commit_ref VARCHAR(255),
    total_tax_minor BIGINT NOT NULL,
    currency VARCHAR(10) NOT NULL DEFAULT 'usd',
    status VARCHAR(20) NOT NULL DEFAULT 'committed',
    committed_at TIMESTAMPTZ,
    voided_at TIMESTAMPTZ,
    void_reason VARCHAR(255),
    correlation_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_tax_commit_invoice UNIQUE (app_id, invoice_id)
);

CREATE INDEX IF NOT EXISTS idx_tax_commits_app_status
    ON ar_tax_commits(app_id, status);

CREATE INDEX IF NOT EXISTS idx_tax_commits_provider_ref
    ON ar_tax_commits(provider_commit_ref);
