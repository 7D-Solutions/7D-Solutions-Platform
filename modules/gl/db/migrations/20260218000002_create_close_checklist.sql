-- Close Checklist + Approvals Gate
-- Phase 31: Close & Compliance — pre-close checklist and approval signoffs.
-- The close_checklist_items table stores items that must be completed/waived
-- before GL period close can execute. close_approvals records signoffs.

-- ============================================================
-- CLOSE CHECKLIST ITEMS TABLE
-- ============================================================

CREATE TABLE IF NOT EXISTS close_checklist_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    label TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'complete', 'waived')),
    completed_by TEXT,
    completed_at TIMESTAMPTZ,
    waive_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================
-- CLOSE APPROVALS TABLE
-- ============================================================

CREATE TABLE IF NOT EXISTS close_approvals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    actor_id TEXT NOT NULL,
    approval_type TEXT NOT NULL,
    notes TEXT,
    approved_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_close_approval_tenant_period_type UNIQUE (tenant_id, period_id, approval_type)
);

-- ============================================================
-- INDEXES
-- ============================================================

CREATE INDEX IF NOT EXISTS idx_close_checklist_tenant_period
    ON close_checklist_items(tenant_id, period_id);

CREATE INDEX IF NOT EXISTS idx_close_approvals_tenant_period
    ON close_approvals(tenant_id, period_id);

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE close_checklist_items IS
    'Pre-close checklist items that must be completed or waived before GL period close can execute. Append-only status transitions.';

COMMENT ON COLUMN close_checklist_items.status IS
    'Item status: pending (not yet addressed), complete (done), waived (skipped with reason).';

COMMENT ON COLUMN close_checklist_items.waive_reason IS
    'Required when status=waived. Explains why the item was skipped.';

COMMENT ON TABLE close_approvals IS
    'Approval signoffs required before GL period close. Each approval_type per tenant/period is unique (idempotent).';

COMMENT ON COLUMN close_approvals.approval_type IS
    'Type of approval signoff (e.g. controller_signoff, manager_review, cfo_approval).';
