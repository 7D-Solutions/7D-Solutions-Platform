-- AP Allocations (append-only) and Payment Runs
--
-- ap_allocations: Append-only table recording how payments are applied to bills.
--   NEVER UPDATE OR DELETE rows — new rows supersede old ones.
--   Idempotent matching: allocation_id is the stable external anchor.
--   Supports partial payments: a bill may have multiple allocation rows.
--
-- payment_runs: Batch payment execution records (one run, many vendors).
--
-- All monetary fields are BIGINT (i64 minor currency units, e.g. cents).
-- Currency is ISO 4217.

-- =============================================================================
-- payment_runs
-- =============================================================================

CREATE TABLE payment_runs (
    run_id          UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    -- Total across all items in this run (minor currency units, i64)
    total_minor     BIGINT NOT NULL CHECK (total_minor >= 0),
    -- ISO 4217 (single-currency runs)
    currency        CHAR(3) NOT NULL,
    scheduled_date  TIMESTAMP WITH TIME ZONE NOT NULL,
    -- Payment method: "ach", "wire", "check"
    payment_method  TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'executing', 'completed', 'failed')),
    created_by      TEXT NOT NULL,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    executed_at     TIMESTAMP WITH TIME ZONE
);

CREATE INDEX idx_payment_runs_tenant_status   ON payment_runs (tenant_id, status);
CREATE INDEX idx_payment_runs_scheduled_date  ON payment_runs (tenant_id, scheduled_date);

-- =============================================================================
-- ap_allocations (append-only — NO UPDATE, NO DELETE)
-- =============================================================================

CREATE TABLE ap_allocations (
    id              BIGSERIAL PRIMARY KEY,
    -- Stable external anchor for idempotent allocation application
    allocation_id   UUID NOT NULL UNIQUE,
    bill_id         UUID NOT NULL REFERENCES vendor_bills (bill_id),
    -- NULL until a payment run claims this allocation
    payment_run_id  UUID REFERENCES payment_runs (run_id),
    tenant_id       TEXT NOT NULL,
    -- Amount applied in minor currency units (i64)
    amount_minor    BIGINT NOT NULL CHECK (amount_minor > 0),
    -- ISO 4217
    currency        CHAR(3) NOT NULL,
    -- "partial" or "full"
    allocation_type TEXT NOT NULL CHECK (allocation_type IN ('partial', 'full')),
    -- Immutable audit timestamp — set once at INSERT
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- IMPORTANT: This table is APPEND-ONLY. The following constraint documents intent.
-- Application code MUST NOT issue UPDATE or DELETE on ap_allocations.

-- Lookup by bill (sum allocations to determine remaining balance)
CREATE INDEX idx_ap_allocations_bill_id        ON ap_allocations (bill_id);

-- Lookup by payment run (which allocations belong to a run)
CREATE INDEX idx_ap_allocations_payment_run_id ON ap_allocations (payment_run_id)
    WHERE payment_run_id IS NOT NULL;

-- Tenant + bill for cross-tenant safe queries
CREATE INDEX idx_ap_allocations_tenant_bill    ON ap_allocations (tenant_id, bill_id);
