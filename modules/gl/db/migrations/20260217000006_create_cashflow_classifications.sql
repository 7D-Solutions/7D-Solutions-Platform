-- Cash Flow Classification Table (Phase 24b, bd-2w3)
-- Maps accounts to cash flow statement categories:
--   operating, investing, financing
-- Used by cash flow report to classify journal line activity.

-- ============================================================
-- CASH FLOW CLASSIFICATION ENUM
-- ============================================================

CREATE TYPE cashflow_category AS ENUM (
    'operating',   -- Day-to-day business operations (revenue, expenses, working capital)
    'investing',   -- Long-term asset purchases/sales (PP&E, investments)
    'financing'    -- Debt and equity transactions (loans, dividends, share issuance)
);

-- ============================================================
-- CASHFLOW_CLASSIFICATIONS TABLE
-- ============================================================

CREATE TABLE cashflow_classifications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    account_code TEXT NOT NULL,
    category cashflow_category NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Each account can only have one classification per tenant
    CONSTRAINT unique_cashflow_classification UNIQUE (tenant_id, account_code)
);

-- ============================================================
-- INDEXES
-- ============================================================

-- Primary lookup: tenant + account
CREATE INDEX idx_cashflow_classifications_tenant
    ON cashflow_classifications(tenant_id);

-- Tenant + category for filtering
CREATE INDEX idx_cashflow_classifications_tenant_category
    ON cashflow_classifications(tenant_id, category);

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE cashflow_classifications IS 'Cash flow statement account classification (Phase 24b): maps accounts to operating/investing/financing categories';
COMMENT ON COLUMN cashflow_classifications.tenant_id IS 'Tenant isolation';
COMMENT ON COLUMN cashflow_classifications.account_code IS 'Chart of Accounts code — must match accounts.code for the same tenant';
COMMENT ON COLUMN cashflow_classifications.category IS 'Cash flow classification: operating, investing, or financing';
