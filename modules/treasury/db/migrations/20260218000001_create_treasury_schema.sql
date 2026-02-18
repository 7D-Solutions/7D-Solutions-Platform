-- Treasury Module: Core Schema
-- Bank accounts, imported statements, normalized transactions, reconciliation matches
-- All monetary amounts stored as BIGINT (i64 minor units, e.g. cents) + ISO 4217 currency

-- ============================================================
-- ENUMS
-- ============================================================

CREATE TYPE treasury_account_status AS ENUM (
    'active',
    'inactive',
    'closed'
);

CREATE TYPE treasury_statement_status AS ENUM (
    'pending',
    'imported',
    'reconciled'
);

CREATE TYPE treasury_txn_status AS ENUM (
    'unmatched',
    'matched',
    'excluded'
);

CREATE TYPE treasury_recon_match_status AS ENUM (
    'pending',
    'confirmed',
    'rejected'
);

CREATE TYPE treasury_recon_match_type AS ENUM (
    'auto',
    'manual',
    'suggested'
);

-- ============================================================
-- BANK ACCOUNTS
-- ============================================================

CREATE TABLE treasury_bank_accounts (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id               VARCHAR(50) NOT NULL,
    account_name         VARCHAR(255) NOT NULL,
    institution          VARCHAR(255),
    account_number_last4 VARCHAR(4),
    routing_number       VARCHAR(50),
    currency             VARCHAR(3) NOT NULL DEFAULT 'USD',
    current_balance_minor BIGINT NOT NULL DEFAULT 0,
    status               treasury_account_status NOT NULL DEFAULT 'active',
    metadata             JSONB,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX treasury_bank_accounts_app_id
    ON treasury_bank_accounts(app_id);
CREATE INDEX treasury_bank_accounts_app_status
    ON treasury_bank_accounts(app_id, status);

-- ============================================================
-- BANK STATEMENTS (imported period statements)
-- ============================================================

CREATE TABLE treasury_bank_statements (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id                VARCHAR(50) NOT NULL,
    account_id            UUID NOT NULL REFERENCES treasury_bank_accounts(id) ON DELETE RESTRICT,
    period_start          DATE NOT NULL,
    period_end            DATE NOT NULL,
    opening_balance_minor BIGINT NOT NULL,
    closing_balance_minor BIGINT NOT NULL,
    currency              VARCHAR(3) NOT NULL DEFAULT 'USD',
    status                treasury_statement_status NOT NULL DEFAULT 'pending',
    imported_at           TIMESTAMPTZ,
    source_filename       VARCHAR(500),
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT treasury_statements_unique_period
        UNIQUE (account_id, period_start, period_end)
);

-- Primary access pattern: (tenant, account, date)
CREATE INDEX treasury_bank_statements_app_account_period
    ON treasury_bank_statements(app_id, account_id, period_start);
CREATE INDEX treasury_bank_statements_status
    ON treasury_bank_statements(status);

-- ============================================================
-- BANK TRANSACTIONS (normalized, one row per transaction line)
-- ============================================================

CREATE TABLE treasury_bank_transactions (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id           VARCHAR(50) NOT NULL,
    account_id       UUID NOT NULL REFERENCES treasury_bank_accounts(id) ON DELETE RESTRICT,
    statement_id     UUID REFERENCES treasury_bank_statements(id) ON DELETE SET NULL,
    transaction_date DATE NOT NULL,
    -- positive = credit (money in), negative = debit (money out)
    amount_minor     BIGINT NOT NULL,
    currency         VARCHAR(3) NOT NULL DEFAULT 'USD',
    description      TEXT,
    reference        VARCHAR(255),
    -- bank-assigned ID; used for dedup on re-import (NULL allowed for manual entries)
    external_id      VARCHAR(255),
    status           treasury_txn_status NOT NULL DEFAULT 'unmatched',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- prevents double-import of the same bank transaction
    CONSTRAINT treasury_txn_unique_external
        UNIQUE (account_id, external_id)
);

-- Primary access pattern: (tenant, account, date)
CREATE INDEX treasury_bank_transactions_app_account_date
    ON treasury_bank_transactions(app_id, account_id, transaction_date);
CREATE INDEX treasury_bank_transactions_app_date
    ON treasury_bank_transactions(app_id, transaction_date);
CREATE INDEX treasury_bank_transactions_status
    ON treasury_bank_transactions(status);
CREATE INDEX treasury_bank_transactions_statement
    ON treasury_bank_transactions(statement_id);

-- ============================================================
-- RECON MATCHES (bank transaction ↔ GL journal entry)
-- ============================================================

CREATE TABLE treasury_recon_matches (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              VARCHAR(50) NOT NULL,
    bank_transaction_id UUID NOT NULL REFERENCES treasury_bank_transactions(id) ON DELETE CASCADE,
    -- soft reference to gl_journal_entries.id (cross-module boundary, no FK)
    gl_entry_id         BIGINT,
    match_type          treasury_recon_match_type NOT NULL DEFAULT 'suggested',
    -- 0.0000 to 1.0000; NULL for manual matches
    confidence_score    NUMERIC(5, 4),
    matched_by          VARCHAR(255),
    status              treasury_recon_match_status NOT NULL DEFAULT 'pending',
    matched_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX treasury_recon_matches_app_txn
    ON treasury_recon_matches(app_id, bank_transaction_id);
CREATE INDEX treasury_recon_matches_app_gl
    ON treasury_recon_matches(app_id, gl_entry_id);
CREATE INDEX treasury_recon_matches_status
    ON treasury_recon_matches(status);
