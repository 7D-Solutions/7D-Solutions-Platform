-- Reporting module: read-model cache tables and ingestion checkpoints
-- bd-3sli: statement cache, kpi cache, ingestion checkpoints
--
-- Design principles:
--   - All monetary amounts stored as BIGINT minor units (e.g. cents) + TEXT currency
--   - as_of DATE used for point-in-time snapshots; indexed with tenant_id for fast lookup
--   - ingestion_checkpoints keyed by (consumer_name, tenant_id) for idempotent NATS replay
--   - Tables prefixed rpt_ to avoid clashes with source-module schemas

-- ============================================================
-- INGESTION CHECKPOINTS
-- ============================================================
-- Tracks the last successfully processed NATS sequence per consumer per tenant.
-- Consumers upsert here after processing each event so they can resume without re-processing.

CREATE TABLE rpt_ingestion_checkpoints (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    consumer_name   TEXT        NOT NULL,
    tenant_id       TEXT        NOT NULL,
    last_sequence   BIGINT      NOT NULL DEFAULT 0,   -- NATS stream sequence
    last_event_id   TEXT,                              -- idempotency key from EventEnvelope
    processed_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_ingestion_checkpoints_unique
        UNIQUE (consumer_name, tenant_id)
);

CREATE INDEX idx_rpt_ingestion_checkpoints_tenant
    ON rpt_ingestion_checkpoints (tenant_id);

COMMENT ON TABLE rpt_ingestion_checkpoints IS
    'Idempotent replay checkpoints for NATS consumers. Grain: (consumer_name, tenant_id).';

-- ============================================================
-- TRIAL BALANCE CACHE
-- ============================================================
-- Point-in-time snapshot of each account balance for fast trial balance reports.
-- Grain: (tenant_id, as_of, account_code, currency)

CREATE TABLE rpt_trial_balance_cache (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    as_of           DATE        NOT NULL,
    account_code    TEXT        NOT NULL,
    account_name    TEXT        NOT NULL,
    currency        TEXT        NOT NULL,

    debit_minor     BIGINT      NOT NULL DEFAULT 0 CHECK (debit_minor >= 0),
    credit_minor    BIGINT      NOT NULL DEFAULT 0 CHECK (credit_minor >= 0),
    -- Signed net: positive = net debit, negative = net credit
    net_minor       BIGINT      NOT NULL DEFAULT 0,

    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_trial_balance_cache_unique
        UNIQUE (tenant_id, as_of, account_code, currency)
);

CREATE INDEX idx_rpt_trial_balance_tenant_as_of
    ON rpt_trial_balance_cache (tenant_id, as_of);

CREATE INDEX idx_rpt_trial_balance_tenant_account
    ON rpt_trial_balance_cache (tenant_id, account_code);

COMMENT ON TABLE rpt_trial_balance_cache IS
    'Trial balance cache: pre-computed debit/credit/net per account per as_of date.';

-- ============================================================
-- STATEMENT CACHE
-- ============================================================
-- Line-item cache for income statements and balance sheets.
-- Grain: (tenant_id, statement_type, as_of, line_code, currency)
-- statement_type: 'income_statement' | 'balance_sheet'

CREATE TABLE rpt_statement_cache (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    statement_type  TEXT        NOT NULL,   -- 'income_statement' | 'balance_sheet'
    as_of           DATE        NOT NULL,
    line_code       TEXT        NOT NULL,   -- e.g. '4000_revenue', '5000_cogs'
    line_label      TEXT        NOT NULL,
    currency        TEXT        NOT NULL,
    amount_minor    BIGINT      NOT NULL DEFAULT 0,
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_statement_cache_unique
        UNIQUE (tenant_id, statement_type, as_of, line_code, currency)
);

CREATE INDEX idx_rpt_statement_cache_tenant_as_of
    ON rpt_statement_cache (tenant_id, as_of);

CREATE INDEX idx_rpt_statement_cache_tenant_type_as_of
    ON rpt_statement_cache (tenant_id, statement_type, as_of);

COMMENT ON TABLE rpt_statement_cache IS
    'Financial statement line-item cache (income statement, balance sheet).';

-- ============================================================
-- AR AGING CACHE
-- ============================================================
-- Accounts-receivable aging buckets per customer per as_of date.
-- Grain: (tenant_id, as_of, customer_id, currency)

CREATE TABLE rpt_ar_aging_cache (
    id                  UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT    NOT NULL,
    as_of               DATE    NOT NULL,
    customer_id         TEXT    NOT NULL,
    currency            TEXT    NOT NULL,

    -- Aging buckets (minor units)
    current_minor       BIGINT  NOT NULL DEFAULT 0 CHECK (current_minor >= 0),
    bucket_1_30_minor   BIGINT  NOT NULL DEFAULT 0 CHECK (bucket_1_30_minor >= 0),
    bucket_31_60_minor  BIGINT  NOT NULL DEFAULT 0 CHECK (bucket_31_60_minor >= 0),
    bucket_61_90_minor  BIGINT  NOT NULL DEFAULT 0 CHECK (bucket_61_90_minor >= 0),
    bucket_over_90_minor BIGINT NOT NULL DEFAULT 0 CHECK (bucket_over_90_minor >= 0),
    total_minor         BIGINT  NOT NULL DEFAULT 0 CHECK (total_minor >= 0),

    computed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_ar_aging_cache_unique
        UNIQUE (tenant_id, as_of, customer_id, currency)
);

CREATE INDEX idx_rpt_ar_aging_tenant_as_of
    ON rpt_ar_aging_cache (tenant_id, as_of);

CREATE INDEX idx_rpt_ar_aging_tenant_customer
    ON rpt_ar_aging_cache (tenant_id, customer_id);

COMMENT ON TABLE rpt_ar_aging_cache IS
    'AR aging cache: receivables bucketed by days-past-due per customer per as_of date.';

-- ============================================================
-- AP AGING CACHE
-- ============================================================
-- Accounts-payable aging buckets per vendor per as_of date.
-- Grain: (tenant_id, as_of, vendor_id, currency)

CREATE TABLE rpt_ap_aging_cache (
    id                  UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT    NOT NULL,
    as_of               DATE    NOT NULL,
    vendor_id           TEXT    NOT NULL,
    currency            TEXT    NOT NULL,

    -- Aging buckets (minor units)
    current_minor       BIGINT  NOT NULL DEFAULT 0 CHECK (current_minor >= 0),
    bucket_1_30_minor   BIGINT  NOT NULL DEFAULT 0 CHECK (bucket_1_30_minor >= 0),
    bucket_31_60_minor  BIGINT  NOT NULL DEFAULT 0 CHECK (bucket_31_60_minor >= 0),
    bucket_61_90_minor  BIGINT  NOT NULL DEFAULT 0 CHECK (bucket_61_90_minor >= 0),
    bucket_over_90_minor BIGINT NOT NULL DEFAULT 0 CHECK (bucket_over_90_minor >= 0),
    total_minor         BIGINT  NOT NULL DEFAULT 0 CHECK (total_minor >= 0),

    computed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_ap_aging_cache_unique
        UNIQUE (tenant_id, as_of, vendor_id, currency)
);

CREATE INDEX idx_rpt_ap_aging_tenant_as_of
    ON rpt_ap_aging_cache (tenant_id, as_of);

CREATE INDEX idx_rpt_ap_aging_tenant_vendor
    ON rpt_ap_aging_cache (tenant_id, vendor_id);

COMMENT ON TABLE rpt_ap_aging_cache IS
    'AP aging cache: payables bucketed by days-past-due per vendor per as_of date.';

-- ============================================================
-- CASHFLOW CACHE
-- ============================================================
-- Cash flow statement lines per reporting period.
-- Grain: (tenant_id, period_start, period_end, activity_type, line_code, currency)
-- as_of = period_end for the (tenant_id, as_of) index requirement.

CREATE TABLE rpt_cashflow_cache (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    period_start    DATE        NOT NULL,
    period_end      DATE        NOT NULL,   -- treated as as_of for index purposes
    activity_type   TEXT        NOT NULL,   -- 'operating' | 'investing' | 'financing'
    line_code       TEXT        NOT NULL,   -- e.g. 'net_income', 'depreciation', 'capex'
    line_label      TEXT        NOT NULL,
    currency        TEXT        NOT NULL,
    amount_minor    BIGINT      NOT NULL DEFAULT 0,  -- signed: positive = inflow
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_cashflow_cache_unique
        UNIQUE (tenant_id, period_start, period_end, activity_type, line_code, currency)
);

-- as_of index using period_end
CREATE INDEX idx_rpt_cashflow_tenant_as_of
    ON rpt_cashflow_cache (tenant_id, period_end);

CREATE INDEX idx_rpt_cashflow_tenant_period
    ON rpt_cashflow_cache (tenant_id, period_start, period_end);

COMMENT ON TABLE rpt_cashflow_cache IS
    'Cash flow statement cache: operating/investing/financing lines per period.';

-- ============================================================
-- KPI CACHE
-- ============================================================
-- Point-in-time KPI snapshots (MRR, ARR, DSO, DPO, churn rate, etc.).
-- Grain: (tenant_id, as_of, kpi_name, COALESCE(currency, ''))
-- Monetary KPIs: amount_minor non-null; rate/ratio KPIs: basis_points non-null.

CREATE TABLE rpt_kpi_cache (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    as_of           DATE        NOT NULL,
    kpi_name        TEXT        NOT NULL,   -- e.g. 'mrr', 'arr', 'churn_rate', 'dso', 'dpo'
    -- Use empty string '' for dimensionless KPIs (rates, ratios, counts) to allow unique index.
    currency        TEXT        NOT NULL DEFAULT '',

    -- Monetary KPI (minor units, e.g. cents)
    amount_minor    BIGINT,

    -- Rate/ratio KPI stored as basis points (10000 bp = 100%)
    -- e.g. 5.25% churn → 525 bp
    basis_points    BIGINT,

    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_kpi_cache_unique
        UNIQUE (tenant_id, as_of, kpi_name, currency),

    CONSTRAINT rpt_kpi_cache_value_check CHECK (
        amount_minor IS NOT NULL OR basis_points IS NOT NULL
    )
);

CREATE INDEX idx_rpt_kpi_tenant_as_of
    ON rpt_kpi_cache (tenant_id, as_of);

CREATE INDEX idx_rpt_kpi_tenant_name
    ON rpt_kpi_cache (tenant_id, kpi_name);

COMMENT ON TABLE rpt_kpi_cache IS
    'KPI snapshot cache: monetary KPIs in minor units, rate KPIs in basis points.';
