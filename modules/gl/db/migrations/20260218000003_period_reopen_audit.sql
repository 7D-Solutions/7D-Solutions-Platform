-- Period Reopen Audit Trail
-- Phase 31 (bd-2rl9): Controlled reopen with immutable audit log
-- Reopen is exceptional: explicit request → approval → execution, all append-only.

-- ============================================================
-- PERIOD REOPEN REQUESTS TABLE
-- ============================================================

CREATE TABLE IF NOT EXISTS period_reopen_requests (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    requested_by TEXT NOT NULL,
    reason TEXT NOT NULL,
    prior_close_hash TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'requested'
        CHECK (status IN ('requested', 'approved', 'rejected')),
    approved_by TEXT,
    approved_at TIMESTAMP WITH TIME ZONE,
    rejected_by TEXT,
    rejected_at TIMESTAMP WITH TIME ZONE,
    reject_reason TEXT,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- ============================================================
-- ACCOUNTING_PERIODS: Add reopen tracking columns
-- ============================================================

ALTER TABLE accounting_periods
    ADD COLUMN IF NOT EXISTS reopen_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS last_reopened_at TIMESTAMP WITH TIME ZONE;

-- ============================================================
-- INDEXES
-- ============================================================

CREATE INDEX IF NOT EXISTS idx_period_reopen_requests_tenant_period
    ON period_reopen_requests(tenant_id, period_id);

CREATE INDEX IF NOT EXISTS idx_period_reopen_requests_status
    ON period_reopen_requests(tenant_id, status)
    WHERE status = 'requested';

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE period_reopen_requests IS
    'Append-only audit trail for period reopen requests. Each row is immutable once written.';

COMMENT ON COLUMN period_reopen_requests.prior_close_hash IS
    'SHA-256 close hash captured at request time — proves which sealed state the reopen targets.';

COMMENT ON COLUMN accounting_periods.reopen_count IS
    'Number of times this period has been reopened. Monotonically increasing.';

COMMENT ON COLUMN accounting_periods.last_reopened_at IS
    'Timestamp of the most recent reopen. NULL if never reopened.';
