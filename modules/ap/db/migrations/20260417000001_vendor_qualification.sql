-- Vendor Qualification Gate (bd-vf7mt)
--
-- Adds supplier eligibility tracking to the vendors table.
-- Existing rows are backfilled to 'qualified' so current POs keep working.
-- New vendors default to 'unqualified' and must be explicitly approved.

ALTER TABLE vendors
    ADD COLUMN IF NOT EXISTS qualification_status  TEXT NOT NULL DEFAULT 'unqualified'
        CHECK (qualification_status IN ('unqualified','pending_review','qualified','restricted','disqualified')),
    ADD COLUMN IF NOT EXISTS qualification_notes   TEXT,
    ADD COLUMN IF NOT EXISTS qualified_by          TEXT,
    ADD COLUMN IF NOT EXISTS qualified_at          TIMESTAMP WITH TIME ZONE,
    ADD COLUMN IF NOT EXISTS preferred_vendor      BOOLEAN NOT NULL DEFAULT FALSE;

-- Backfill all existing vendor rows to 'qualified' so no current POs break
UPDATE vendors SET qualification_status = 'qualified' WHERE qualification_status = 'unqualified';

-- Audit trail for every qualification status change
CREATE TABLE IF NOT EXISTS vendor_qualification_events (
    id              UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    vendor_id       UUID NOT NULL REFERENCES vendors(vendor_id),
    from_status     TEXT,
    to_status       TEXT NOT NULL,
    reason          TEXT,
    changed_by      TEXT NOT NULL,
    changed_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_vqe_vendor_tenant
    ON vendor_qualification_events (vendor_id, tenant_id, changed_at DESC);
