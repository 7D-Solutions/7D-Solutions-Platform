-- Phase 62: Credit memo lifecycle for RMA disposition
-- Extends ar_credit_notes from immediate-issued artifacts to:
--   draft -> approved -> issued
-- while preserving append-only issued semantics.

ALTER TABLE ar_credit_notes
    ADD COLUMN IF NOT EXISTS create_idempotency_key UUID,
    ADD COLUMN IF NOT EXISTS issue_idempotency_key UUID,
    ADD COLUMN IF NOT EXISTS approved_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS approved_by VARCHAR(255),
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Existing rows are already issued credit notes; keep that invariant.
UPDATE ar_credit_notes
SET status = 'issued'
WHERE status IS NULL OR status = '';

ALTER TABLE ar_credit_notes
    ALTER COLUMN status SET DEFAULT 'draft';

ALTER TABLE ar_credit_notes
    DROP CONSTRAINT IF EXISTS ar_credit_notes_status_check;

ALTER TABLE ar_credit_notes
    ADD CONSTRAINT ar_credit_notes_status_check
    CHECK (status IN ('draft', 'approved', 'issued'));

CREATE UNIQUE INDEX IF NOT EXISTS ar_credit_notes_app_create_idem_uq
    ON ar_credit_notes(app_id, create_idempotency_key)
    WHERE create_idempotency_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ar_credit_notes_app_issue_idem_uq
    ON ar_credit_notes(app_id, issue_idempotency_key)
    WHERE issue_idempotency_key IS NOT NULL;
