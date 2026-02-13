-- Account Balances Table (Phase 11: Balance Engine)
-- Materialized rollup store for fast trial balance queries
-- Grain: UNIQUE (tenant_id, period_id, account_code, currency)

-- ============================================================
-- ACCOUNT_BALANCES TABLE
-- ============================================================

CREATE TABLE account_balances (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    period_id UUID NOT NULL REFERENCES accounting_periods(id),
    account_code TEXT NOT NULL,
    currency TEXT NOT NULL,

    -- Cumulative amounts (in minor units, e.g., cents)
    debit_total_minor BIGINT NOT NULL DEFAULT 0 CHECK (debit_total_minor >= 0),
    credit_total_minor BIGINT NOT NULL DEFAULT 0 CHECK (credit_total_minor >= 0),

    -- Net balance (signed, in minor units)
    -- Positive = net debit position, Negative = net credit position
    net_balance_minor BIGINT NOT NULL DEFAULT 0,

    -- Metadata
    last_journal_entry_id UUID, -- Last entry that updated this balance
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Unique constraint on grain (tenant, period, account, currency)
    CONSTRAINT unique_balance_grain UNIQUE (tenant_id, period_id, account_code, currency)
);

-- ============================================================
-- INDEXES
-- ============================================================

-- Primary lookup: tenant + period (for trial balance queries)
CREATE INDEX idx_account_balances_tenant_period
    ON account_balances(tenant_id, period_id);

-- Tenant + period + account + currency (full grain lookup)
CREATE INDEX idx_account_balances_tenant_period_full
    ON account_balances(tenant_id, period_id, account_code, currency);

-- Account-centric queries (balance history across periods)
CREATE INDEX idx_account_balances_account
    ON account_balances(tenant_id, account_code);

-- Period FK integrity
CREATE INDEX idx_account_balances_period_id
    ON account_balances(period_id);

-- Updated_at for incremental processing and audit
CREATE INDEX idx_account_balances_updated_at
    ON account_balances(updated_at);

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE account_balances IS 'Materialized account balances (Phase 11): Fast trial balance queries, multi-currency support, period-aware rollups';
COMMENT ON COLUMN account_balances.tenant_id IS 'Tenant isolation';
COMMENT ON COLUMN account_balances.period_id IS 'Accounting period reference (FK to accounting_periods)';
COMMENT ON COLUMN account_balances.account_code IS 'Chart of Accounts code (e.g., "1000", "4000")';
COMMENT ON COLUMN account_balances.currency IS 'ISO 4217 currency code (e.g., "USD", "EUR", "GBP")';
COMMENT ON COLUMN account_balances.debit_total_minor IS 'Cumulative debit total in minor units (cents)';
COMMENT ON COLUMN account_balances.credit_total_minor IS 'Cumulative credit total in minor units (cents)';
COMMENT ON COLUMN account_balances.net_balance_minor IS 'Net balance = debit_total - credit_total (signed, minor units)';
COMMENT ON COLUMN account_balances.last_journal_entry_id IS 'Last journal entry that updated this balance (audit trail)';
