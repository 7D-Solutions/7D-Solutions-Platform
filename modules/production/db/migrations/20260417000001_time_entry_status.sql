-- Add approval workflow to time_entries.
-- status defaults to 'pending'; transitions: pending→approved or pending→rejected.
-- approved_by/approved_at are populated atomically with the outbox event on approve.
-- rejected_by/rejected_reason are populated on reject (no event emitted).

ALTER TABLE time_entries
    ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'
        CONSTRAINT time_entries_status_check CHECK (status IN ('pending', 'approved', 'rejected')),
    ADD COLUMN approved_by TEXT,
    ADD COLUMN approved_at TIMESTAMPTZ,
    ADD COLUMN rejected_by TEXT,
    ADD COLUMN rejected_reason TEXT;

CREATE INDEX IF NOT EXISTS idx_time_entries_status
    ON time_entries (tenant_id, status);
