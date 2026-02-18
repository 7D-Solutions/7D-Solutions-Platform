-- Revrec Contract Modifications — Phase 24a Wave 2 (bd-1qi)
-- Append-only amendment ledger for ASC 606 contract modifications.
-- Each modification record links to its contract and captures the change type,
-- effective date, and audit reason. Schedules are versioned separately.

CREATE TABLE IF NOT EXISTS revrec_contract_modifications (
    modification_id             UUID PRIMARY KEY,
    contract_id                 UUID NOT NULL REFERENCES revrec_contracts(contract_id),
    tenant_id                   TEXT NOT NULL,
    modification_type           TEXT NOT NULL,
    effective_date              DATE NOT NULL,
    new_transaction_price_minor BIGINT,
    reason                      TEXT NOT NULL,
    requires_cumulative_catchup BOOLEAN NOT NULL DEFAULT FALSE,
    modified_at                 TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_revrec_modifications_contract
    ON revrec_contract_modifications(contract_id);

CREATE INDEX IF NOT EXISTS idx_revrec_modifications_tenant
    ON revrec_contract_modifications(tenant_id);

COMMENT ON TABLE revrec_contract_modifications IS
    'Append-only amendment ledger — each row is an immutable record of a contract modification. '
    'History is never rewritten; recognition schedules are versioned separately.';
