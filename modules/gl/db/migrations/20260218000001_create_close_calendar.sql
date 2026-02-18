-- Close Calendar + Reminder Tracking
-- Phase 31: Close & Compliance — scheduling layer around GL period close.
-- The close_calendar table stores expected close dates and reminder config.
-- The close_calendar_reminders_sent table provides idempotent reminder tracking.

-- ============================================================
-- CLOSE CALENDAR TABLE
-- ============================================================

CREATE TABLE IF NOT EXISTS close_calendar (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    expected_close_date DATE NOT NULL,
    owner_role TEXT NOT NULL,
    reminder_offset_days INTEGER[] NOT NULL DEFAULT '{7, 3, 1}',
    overdue_reminder_interval_days INTEGER NOT NULL DEFAULT 1,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_close_calendar_tenant_period UNIQUE (tenant_id, period_id)
);

-- ============================================================
-- CLOSE CALENDAR REMINDERS SENT (IDEMPOTENCY)
-- ============================================================

CREATE TABLE IF NOT EXISTS close_calendar_reminders_sent (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    calendar_entry_id UUID NOT NULL REFERENCES close_calendar(id),
    reminder_type TEXT NOT NULL CHECK (reminder_type IN ('upcoming', 'overdue')),
    reminder_key TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_reminder_idempotency UNIQUE (tenant_id, calendar_entry_id, reminder_key)
);

-- ============================================================
-- INDEXES
-- ============================================================

CREATE INDEX IF NOT EXISTS idx_close_calendar_tenant
    ON close_calendar(tenant_id);

CREATE INDEX IF NOT EXISTS idx_close_calendar_expected_date
    ON close_calendar(tenant_id, expected_close_date);

CREATE INDEX IF NOT EXISTS idx_close_calendar_reminders_entry
    ON close_calendar_reminders_sent(calendar_entry_id);

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE close_calendar IS
    'Scheduling overlay for GL period close. Stores expected close dates and reminder configuration per tenant/period.';

COMMENT ON COLUMN close_calendar.owner_role IS
    'Role responsible for closing this period (e.g. controller, accounting_manager).';

COMMENT ON COLUMN close_calendar.reminder_offset_days IS
    'Array of days before expected_close_date to send upcoming reminders (e.g. {7,3,1} = 7, 3, and 1 day before).';

COMMENT ON COLUMN close_calendar.overdue_reminder_interval_days IS
    'After expected_close_date passes without close, send overdue reminders every N days.';

COMMENT ON TABLE close_calendar_reminders_sent IS
    'Idempotency tracker for close calendar reminders. Prevents duplicate notification spam.';

COMMENT ON COLUMN close_calendar_reminders_sent.reminder_key IS
    'Deterministic key for dedup (e.g. upcoming:7d:2026-02-28 or overdue:day1:2026-03-01).';
