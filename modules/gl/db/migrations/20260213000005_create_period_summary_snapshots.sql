-- Period Summary Snapshots Table (Phase 12: Reporting Foundation)
-- Persists period summaries (counts/totals) for fast close previews and reporting stability
-- Grain: UNIQUE (tenant_id, period_id, currency)
-- Purpose: Immutable snapshots of period activity without building a full reporting engine

-- ============================================================
-- PERIOD_SUMMARY_SNAPSHOTS TABLE
-- ============================================================

CREATE TABLE period_summary_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    currency TEXT NOT NULL,

    -- Activity counts
    journal_count INTEGER NOT NULL DEFAULT 0 CHECK (journal_count >= 0),
    line_count INTEGER NOT NULL DEFAULT 0 CHECK (line_count >= 0),

    -- Monetary totals (in minor units, e.g., cents)
    total_debits_minor BIGINT NOT NULL DEFAULT 0 CHECK (total_debits_minor >= 0),
    total_credits_minor BIGINT NOT NULL DEFAULT 0 CHECK (total_credits_minor >= 0),

    -- Optional integrity hash/checksum for snapshot validation
    checksum TEXT,

    -- Metadata
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Unique constraint on grain (tenant, period, currency)
    CONSTRAINT unique_snapshot_grain UNIQUE (tenant_id, period_id, currency)
);

-- ============================================================
-- INDEXES
-- ============================================================

-- Primary lookup: tenant + period (for summary queries)
CREATE INDEX idx_period_summary_tenant_period
    ON period_summary_snapshots(tenant_id, period_id);

-- Tenant + period + currency (full grain lookup)
CREATE INDEX idx_period_summary_tenant_period_currency
    ON period_summary_snapshots(tenant_id, period_id, currency);

-- Period FK integrity
CREATE INDEX idx_period_summary_period_id
    ON period_summary_snapshots(period_id);

-- Created_at for temporal queries and audit
CREATE INDEX idx_period_summary_created_at
    ON period_summary_snapshots(created_at);

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE period_summary_snapshots IS 'Period summary snapshots (Phase 12): Fast close previews and reporting stability via pre-computed counts and totals';
COMMENT ON COLUMN period_summary_snapshots.tenant_id IS 'Tenant isolation';
COMMENT ON COLUMN period_summary_snapshots.period_id IS 'Accounting period reference (FK to accounting_periods)';
COMMENT ON COLUMN period_summary_snapshots.currency IS 'ISO 4217 currency code (e.g., "USD", "EUR", "GBP")';
COMMENT ON COLUMN period_summary_snapshots.journal_count IS 'Total number of journal entries in this period (for this tenant + currency)';
COMMENT ON COLUMN period_summary_snapshots.line_count IS 'Total number of journal lines in this period (for this tenant + currency)';
COMMENT ON COLUMN period_summary_snapshots.total_debits_minor IS 'Sum of all debit amounts in minor units (cents)';
COMMENT ON COLUMN period_summary_snapshots.total_credits_minor IS 'Sum of all credit amounts in minor units (cents)';
COMMENT ON COLUMN period_summary_snapshots.checksum IS 'Optional hash/checksum for snapshot integrity validation (e.g., SHA256 of concatenated values)';
COMMENT ON COLUMN period_summary_snapshots.created_at IS 'Snapshot creation timestamp (when the summary was computed)';
