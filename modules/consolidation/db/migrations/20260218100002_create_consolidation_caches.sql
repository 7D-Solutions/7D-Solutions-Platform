-- Consolidation Module: Cache Tables
-- bd-2ye7: consolidated trial balance cache and statement cache
--
-- Design principles:
--   - Caches keyed by (group_id, as_of) as required by acceptance criteria
--   - All monetary amounts stored as BIGINT minor units + TEXT currency
--   - input_hash enables deterministic verification (rerun yields identical results)
--   - Tables prefixed csl_ to avoid clashes with source-module schemas

-- ============================================================
-- CONSOLIDATED TRIAL BALANCE CACHE
-- ============================================================
-- Pre-computed consolidated trial balance after COA mapping, FX translation,
-- and elimination adjustments. Keyed by (group_id, as_of) with per-account
-- granularity. The input_hash captures the hash of all source snapshots
-- used, enabling deterministic rerun verification.

CREATE TABLE csl_trial_balance_cache (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id        UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    as_of           DATE        NOT NULL,
    account_code    TEXT        NOT NULL,   -- group-level consolidated account code
    account_name    TEXT        NOT NULL,
    currency        TEXT        NOT NULL,   -- group reporting currency

    debit_minor     BIGINT      NOT NULL DEFAULT 0 CHECK (debit_minor >= 0),
    credit_minor    BIGINT      NOT NULL DEFAULT 0 CHECK (credit_minor >= 0),
    -- Signed net: positive = net debit, negative = net credit
    net_minor       BIGINT      NOT NULL DEFAULT 0,

    -- Hash of all input snapshots used to produce this row
    input_hash      TEXT        NOT NULL,
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_trial_balance_cache_unique
        UNIQUE (group_id, as_of, account_code, currency)
);

CREATE INDEX idx_csl_tb_cache_group_as_of
    ON csl_trial_balance_cache (group_id, as_of);

CREATE INDEX idx_csl_tb_cache_group_account
    ON csl_trial_balance_cache (group_id, account_code);

COMMENT ON TABLE csl_trial_balance_cache IS
    'Consolidated TB cache: post-mapping, post-FX, post-elimination balances per group per as_of.';

-- ============================================================
-- CONSOLIDATED STATEMENT CACHE
-- ============================================================
-- Pre-computed consolidated financial statement lines (income statement,
-- balance sheet). Derived from the consolidated TB cache. Keyed by
-- (group_id, statement_type, as_of) with per-line granularity.

CREATE TABLE csl_statement_cache (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id        UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    statement_type  TEXT        NOT NULL,   -- 'income_statement' | 'balance_sheet'
    as_of           DATE        NOT NULL,
    line_code       TEXT        NOT NULL,   -- e.g. '4000_revenue', '5000_cogs'
    line_label      TEXT        NOT NULL,
    currency        TEXT        NOT NULL,   -- group reporting currency
    amount_minor    BIGINT      NOT NULL DEFAULT 0,

    -- Hash of all input snapshots used to produce this row
    input_hash      TEXT        NOT NULL,
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_statement_cache_unique
        UNIQUE (group_id, statement_type, as_of, line_code, currency)
);

CREATE INDEX idx_csl_stmt_cache_group_as_of
    ON csl_statement_cache (group_id, as_of);

CREATE INDEX idx_csl_stmt_cache_group_type_as_of
    ON csl_statement_cache (group_id, statement_type, as_of);

COMMENT ON TABLE csl_statement_cache IS
    'Consolidated statement cache: P&L and balance sheet lines per group per as_of.';
