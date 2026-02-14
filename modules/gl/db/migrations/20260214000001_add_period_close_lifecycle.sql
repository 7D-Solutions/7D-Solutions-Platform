-- Add Period Close Lifecycle Fields
-- Extends accounting_periods with close workflow state and audit trail
-- Phase 13: Enable operational period close with validation and immutability

-- ============================================================
-- ALTER ACCOUNTING_PERIODS TABLE
-- ============================================================

-- Add close workflow fields (all nullable - additive only)
-- Using IF NOT EXISTS for idempotency (Postgres 9.6+)
ALTER TABLE accounting_periods
    ADD COLUMN IF NOT EXISTS close_requested_at TIMESTAMP WITH TIME ZONE NULL,
    ADD COLUMN IF NOT EXISTS closed_at TIMESTAMP WITH TIME ZONE NULL,
    ADD COLUMN IF NOT EXISTS closed_by TEXT NULL,
    ADD COLUMN IF NOT EXISTS close_reason TEXT NULL,
    ADD COLUMN IF NOT EXISTS close_hash TEXT NULL;

-- Add CHECK constraint: closed_at must be >= close_requested_at when both exist
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'chk_close_requested_before_closed'
    ) THEN
        ALTER TABLE accounting_periods
            ADD CONSTRAINT chk_close_requested_before_closed
            CHECK (
                close_requested_at IS NULL
                OR closed_at IS NULL
                OR closed_at >= close_requested_at
            );
    END IF;
END $$;

-- Add CHECK constraint: if closed_at is set, close_hash must be set (audit requirement)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'chk_closed_requires_hash'
    ) THEN
        ALTER TABLE accounting_periods
            ADD CONSTRAINT chk_closed_requires_hash
            CHECK (
                closed_at IS NULL
                OR close_hash IS NOT NULL
            );
    END IF;
END $$;

-- ============================================================
-- INDEXES FOR CLOSE STATUS QUERIES
-- ============================================================

-- Index for finding periods by close status (tenant-scoped)
-- Supports queries: "show all closed periods" or "show periods in close-requested state"
CREATE INDEX IF NOT EXISTS idx_accounting_periods_close_status
    ON accounting_periods(tenant_id, closed_at, close_requested_at)
    WHERE closed_at IS NOT NULL OR close_requested_at IS NOT NULL;

-- Index for finding periods ready to close (has close_requested_at but not closed_at)
CREATE INDEX IF NOT EXISTS idx_accounting_periods_pending_close
    ON accounting_periods(tenant_id, close_requested_at)
    WHERE close_requested_at IS NOT NULL AND closed_at IS NULL;

-- ============================================================
-- COMMENTS FOR DOCUMENTATION
-- ============================================================

COMMENT ON COLUMN accounting_periods.close_requested_at IS
    'Timestamp when period close was requested. NULL = not requested. Used for close workflow tracking.';

COMMENT ON COLUMN accounting_periods.closed_at IS
    'Timestamp when period was permanently closed. NULL = not closed. Once set, period becomes immutable. Idempotency key for close operation.';

COMMENT ON COLUMN accounting_periods.closed_by IS
    'User or system identifier who closed the period. NULL if not closed. For audit trail.';

COMMENT ON COLUMN accounting_periods.close_reason IS
    'Optional reason/notes for closing the period. For audit trail.';

COMMENT ON COLUMN accounting_periods.close_hash IS
    'SHA-256 hash of period summary snapshot for tamper detection. REQUIRED when closed_at is set. For audit trail.';
