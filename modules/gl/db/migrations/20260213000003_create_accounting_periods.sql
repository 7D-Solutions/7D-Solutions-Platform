-- Accounting Periods Table
-- Defines fiscal/accounting periods with closed-period governance
-- Phase 10: Enable period-aware posting controls

-- ============================================================
-- PREREQUISITES
-- ============================================================

-- Enable btree_gist extension for EXCLUDE constraints on date ranges
CREATE EXTENSION IF NOT EXISTS btree_gist;

-- ============================================================
-- ACCOUNTING_PERIODS TABLE
-- ============================================================

CREATE TABLE accounting_periods (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_start DATE NOT NULL,
    period_end DATE NOT NULL,
    is_closed BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Ensure period_end is after period_start
    CHECK (period_end > period_start),

    -- Prevent overlapping periods per tenant using EXCLUDE constraint
    -- This enforces that no two periods for the same tenant can overlap
    -- Uses daterange with '[]' (inclusive bounds) and gist indexing
    EXCLUDE USING gist (
        tenant_id WITH =,
        daterange(period_start, period_end, '[]') WITH &&
    )
);

-- ============================================================
-- INDEXES
-- ============================================================

-- Index for tenant-scoped queries (most common)
CREATE INDEX idx_accounting_periods_tenant_id ON accounting_periods(tenant_id);

-- Index for closed period lookups
CREATE INDEX idx_accounting_periods_is_closed ON accounting_periods(is_closed);

-- Composite index for tenant + closed period queries
CREATE INDEX idx_accounting_periods_tenant_closed ON accounting_periods(tenant_id, is_closed);

-- Index for date range queries (finding period by posting date)
CREATE INDEX idx_accounting_periods_tenant_dates ON accounting_periods(tenant_id, period_start, period_end);
