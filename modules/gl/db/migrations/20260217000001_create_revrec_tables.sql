-- Revenue Recognition (Revrec) Data Model — Phase 24a Wave 1
-- ASC 606 / IFRS 15 contract and obligation backbone
--
-- Tables: revrec_contracts, revrec_obligations, revrec_schedules, revrec_schedule_lines
-- All writes are atomic with outbox (transactional outbox pattern).
-- Idempotency: contract_id is the caller-supplied stable business key.

-- ============================================================
-- CONTRACTS
-- ============================================================

CREATE TABLE revrec_contracts (
    contract_id  UUID PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    customer_id  TEXT NOT NULL,
    contract_name TEXT NOT NULL,
    contract_start DATE NOT NULL,
    contract_end   DATE,
    total_transaction_price_minor BIGINT NOT NULL,
    currency     TEXT NOT NULL,
    external_contract_ref TEXT,
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_revrec_contracts_tenant ON revrec_contracts(tenant_id);
CREATE INDEX idx_revrec_contracts_customer ON revrec_contracts(tenant_id, customer_id);
CREATE UNIQUE INDEX idx_revrec_contracts_idempotent ON revrec_contracts(contract_id);

-- ============================================================
-- PERFORMANCE OBLIGATIONS
-- ============================================================

CREATE TABLE revrec_obligations (
    obligation_id UUID PRIMARY KEY,
    contract_id   UUID NOT NULL REFERENCES revrec_contracts(contract_id),
    tenant_id     TEXT NOT NULL,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL,
    allocated_amount_minor BIGINT NOT NULL,
    recognition_pattern JSONB NOT NULL,
    satisfaction_start DATE NOT NULL,
    satisfaction_end   DATE,
    status        TEXT NOT NULL DEFAULT 'unsatisfied',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_revrec_obligations_contract ON revrec_obligations(contract_id);
CREATE INDEX idx_revrec_obligations_tenant ON revrec_obligations(tenant_id);

-- ============================================================
-- RECOGNITION SCHEDULES
-- ============================================================

CREATE TABLE revrec_schedules (
    schedule_id  UUID PRIMARY KEY,
    contract_id  UUID NOT NULL REFERENCES revrec_contracts(contract_id),
    obligation_id UUID NOT NULL REFERENCES revrec_obligations(obligation_id),
    tenant_id    TEXT NOT NULL,
    total_to_recognize_minor BIGINT NOT NULL,
    currency     TEXT NOT NULL,
    first_period TEXT NOT NULL,
    last_period  TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_revrec_schedules_contract ON revrec_schedules(contract_id);
CREATE INDEX idx_revrec_schedules_obligation ON revrec_schedules(obligation_id);
CREATE INDEX idx_revrec_schedules_tenant ON revrec_schedules(tenant_id);

-- ============================================================
-- SCHEDULE LINES (amortization table)
-- ============================================================

CREATE TABLE revrec_schedule_lines (
    id           BIGSERIAL PRIMARY KEY,
    schedule_id  UUID NOT NULL REFERENCES revrec_schedules(schedule_id),
    period       TEXT NOT NULL,
    amount_to_recognize_minor BIGINT NOT NULL,
    deferred_revenue_account TEXT NOT NULL,
    recognized_revenue_account TEXT NOT NULL,
    recognized   BOOLEAN NOT NULL DEFAULT FALSE,
    recognized_at TIMESTAMPTZ
);

CREATE INDEX idx_revrec_schedule_lines_schedule ON revrec_schedule_lines(schedule_id);
CREATE UNIQUE INDEX idx_revrec_schedule_lines_unique ON revrec_schedule_lines(schedule_id, period);

COMMENT ON TABLE revrec_contracts IS 'Revenue contracts (ASC 606 Step 1) — root entity for revrec lifecycle';
COMMENT ON TABLE revrec_obligations IS 'Performance obligations (ASC 606 Step 2) — distinct promises within a contract';
COMMENT ON TABLE revrec_schedules IS 'Recognition schedules — amortization plan per obligation';
COMMENT ON TABLE revrec_schedule_lines IS 'Schedule lines — one entry per period in a recognition schedule';
