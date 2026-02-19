-- TTP domain schema migration
-- Customers, service agreements, one-time charges, billing runs

-- Customers: lightweight reference to Party, tracking TTP-level status
CREATE TABLE IF NOT EXISTS ttp_customers (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id      UUID        NOT NULL,
    party_id       UUID        NOT NULL,
    external_ref   TEXT,
    status         TEXT        NOT NULL DEFAULT 'active'
                               CHECK (status IN ('active', 'suspended', 'cancelled')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_ttp_customers_tenant_party
    ON ttp_customers (tenant_id, party_id);

CREATE INDEX IF NOT EXISTS idx_ttp_customers_tenant_status
    ON ttp_customers (tenant_id, status);

-- Service agreements: recurring subscription plans per party
CREATE TABLE IF NOT EXISTS ttp_service_agreements (
    agreement_id   UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id      UUID        NOT NULL,
    party_id       UUID        NOT NULL,
    plan_code      TEXT        NOT NULL,
    amount_minor   BIGINT      NOT NULL CHECK (amount_minor >= 0),
    currency       CHAR(3)     NOT NULL,
    billing_cycle  TEXT        NOT NULL DEFAULT 'monthly'
                               CHECK (billing_cycle IN ('monthly', 'quarterly', 'annual')),
    status         TEXT        NOT NULL DEFAULT 'active'
                               CHECK (status IN ('active', 'suspended', 'cancelled')),
    effective_from DATE        NOT NULL,
    effective_to   DATE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ttp_agreements_tenant_party
    ON ttp_service_agreements (tenant_id, party_id);

CREATE INDEX IF NOT EXISTS idx_ttp_agreements_tenant_status
    ON ttp_service_agreements (tenant_id, status);

-- One-time charges: ad-hoc amounts to be included in the next billing run
CREATE TABLE IF NOT EXISTS ttp_one_time_charges (
    charge_id      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id      UUID        NOT NULL,
    party_id       UUID        NOT NULL,
    description    TEXT        NOT NULL,
    amount_minor   BIGINT      NOT NULL CHECK (amount_minor >= 0),
    currency       CHAR(3)     NOT NULL,
    status         TEXT        NOT NULL DEFAULT 'pending'
                               CHECK (status IN ('pending', 'billed', 'cancelled')),
    ar_invoice_id  UUID,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ttp_charges_tenant_party
    ON ttp_one_time_charges (tenant_id, party_id);

CREATE INDEX IF NOT EXISTS idx_ttp_charges_tenant_status
    ON ttp_one_time_charges (tenant_id, status);

-- Billing runs: one run per (tenant, billing_period); idempotency enforced
CREATE TABLE IF NOT EXISTS ttp_billing_runs (
    run_id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID        NOT NULL,
    billing_period  TEXT        NOT NULL,   -- e.g. '2026-02'
    status          TEXT        NOT NULL DEFAULT 'pending'
                                CHECK (status IN ('pending', 'processing', 'completed', 'failed')),
    idempotency_key TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_ttp_billing_runs_tenant_period
        UNIQUE (tenant_id, billing_period)
);

CREATE INDEX IF NOT EXISTS idx_ttp_billing_runs_tenant_status
    ON ttp_billing_runs (tenant_id, status);

-- Billing run items: one row per party per run
CREATE TABLE IF NOT EXISTS ttp_billing_run_items (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id         UUID        NOT NULL REFERENCES ttp_billing_runs (run_id),
    party_id       UUID        NOT NULL,
    ar_invoice_id  UUID,
    amount_minor   BIGINT      NOT NULL CHECK (amount_minor >= 0),
    currency       CHAR(3)     NOT NULL,
    status         TEXT        NOT NULL DEFAULT 'pending'
                               CHECK (status IN ('pending', 'invoiced', 'failed')),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ttp_run_items_run_id
    ON ttp_billing_run_items (run_id);

CREATE INDEX IF NOT EXISTS idx_ttp_run_items_party_id
    ON ttp_billing_run_items (run_id, party_id);
