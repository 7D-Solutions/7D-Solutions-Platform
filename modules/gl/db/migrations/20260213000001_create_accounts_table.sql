-- Chart of Accounts (COA) Table
-- Defines the accounts available for posting journal entries
-- Flat structure (no hierarchy in Phase 10)

-- ============================================================
-- ACCOUNT TYPES
-- ============================================================

-- Account types for double-entry accounting
CREATE TYPE account_type AS ENUM (
    'asset',      -- Assets (e.g., Cash, Accounts Receivable)
    'liability',  -- Liabilities (e.g., Accounts Payable, Loans)
    'equity',     -- Equity (e.g., Capital, Retained Earnings)
    'revenue',    -- Revenue/Income (e.g., Sales, Service Revenue)
    'expense'     -- Expenses (e.g., Rent, Salaries, COGS)
);

-- Normal balance direction for each account type
CREATE TYPE normal_balance AS ENUM (
    'debit',   -- Assets and Expenses increase with debits
    'credit'   -- Liabilities, Equity, and Revenue increase with credits
);

-- ============================================================
-- ACCOUNTS TABLE
-- ============================================================

CREATE TABLE accounts (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    type account_type NOT NULL,
    normal_balance normal_balance NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Unique constraint: each tenant can only have one account per code
    CONSTRAINT unique_tenant_code UNIQUE (tenant_id, code)
);

-- ============================================================
-- INDEXES
-- ============================================================

-- Index for tenant-scoped queries (most common)
CREATE INDEX idx_accounts_tenant_id ON accounts(tenant_id);

-- Index for active account lookups
CREATE INDEX idx_accounts_is_active ON accounts(is_active);

-- Composite index for tenant + active account queries
CREATE INDEX idx_accounts_tenant_active ON accounts(tenant_id, is_active) WHERE is_active = true;

-- Index for code lookups within a tenant
CREATE INDEX idx_accounts_tenant_code ON accounts(tenant_id, code);
