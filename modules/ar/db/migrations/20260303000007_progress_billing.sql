-- Progress billing / milestone invoicing (Phase 63, bd-7nvjh)
--
-- Supports invoicing against project milestones or percentage-of-completion
-- rather than full delivery. Core invariant: cumulative billed amount never
-- exceeds the contract total.

CREATE TABLE IF NOT EXISTS ar_progress_billing_contracts (
    id              SERIAL PRIMARY KEY,
    contract_id     UUID NOT NULL,
    app_id          TEXT NOT NULL,
    customer_id     TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    total_amount_minor BIGINT NOT NULL CHECK (total_amount_minor > 0),
    currency        TEXT NOT NULL DEFAULT 'usd',
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'completed', 'cancelled')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (app_id, contract_id)
);

CREATE INDEX idx_pb_contracts_app_status
    ON ar_progress_billing_contracts (app_id, status);

CREATE TABLE IF NOT EXISTS ar_progress_billing_milestones (
    id              SERIAL PRIMARY KEY,
    milestone_id    UUID NOT NULL,
    contract_row_id INTEGER NOT NULL REFERENCES ar_progress_billing_contracts(id),
    app_id          TEXT NOT NULL,
    name            TEXT NOT NULL,
    percentage      INTEGER NOT NULL CHECK (percentage > 0 AND percentage <= 100),
    amount_minor    BIGINT NOT NULL CHECK (amount_minor > 0),
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'billed')),
    billed_at       TIMESTAMPTZ,
    invoice_id      INTEGER,
    idempotency_key UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (app_id, milestone_id),
    UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_pb_milestones_contract
    ON ar_progress_billing_milestones (contract_row_id, status);

CREATE INDEX idx_pb_milestones_app_idem
    ON ar_progress_billing_milestones (app_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
