-- AP tax snapshots: audit-ready tax lifecycle per vendor bill.
-- Supports quote -> commit -> void lifecycle with idempotency guarantees.
-- Each bill has at most one active (non-voided) snapshot.

CREATE TABLE IF NOT EXISTS ap_tax_snapshots (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bill_id         UUID NOT NULL REFERENCES vendor_bills(bill_id),
    tenant_id       TEXT NOT NULL,
    provider        TEXT NOT NULL,
    provider_quote_ref  TEXT NOT NULL,
    provider_commit_ref TEXT,
    quote_hash      TEXT NOT NULL,
    total_tax_minor BIGINT NOT NULL,
    tax_by_line     JSONB NOT NULL DEFAULT '[]'::jsonb,
    status          TEXT NOT NULL DEFAULT 'quoted'
                    CHECK (status IN ('quoted', 'committed', 'voided')),
    quoted_at       TIMESTAMPTZ NOT NULL,
    committed_at    TIMESTAMPTZ,
    voided_at       TIMESTAMPTZ,
    void_reason     TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Fast lookup by bill
CREATE INDEX IF NOT EXISTS idx_ap_tax_snapshots_bill_id
    ON ap_tax_snapshots(bill_id);

-- At most one active (non-voided) snapshot per bill
CREATE UNIQUE INDEX IF NOT EXISTS idx_ap_tax_snapshots_bill_active
    ON ap_tax_snapshots(bill_id) WHERE status != 'voided';
