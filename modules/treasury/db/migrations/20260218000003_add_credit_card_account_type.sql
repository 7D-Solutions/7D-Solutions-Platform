-- Treasury: Add credit_card account type + CC-specific fields
-- Extends treasury_bank_accounts with account_type enum (bank|credit_card)
-- and adds auth_date/settle_date/merchant descriptors to treasury_bank_transactions.
-- Existing bank rows default to account_type='bank' — fully backward compatible.

-- ============================================================
-- ACCOUNT TYPE ENUM
-- ============================================================

CREATE TYPE treasury_account_type AS ENUM (
    'bank',
    'credit_card'
);

-- ============================================================
-- ACCOUNT-LEVEL CC FIELDS
-- ============================================================

-- account_type: bank (default, backward-compat) or credit_card
ALTER TABLE treasury_bank_accounts
    ADD COLUMN account_type        treasury_account_type NOT NULL DEFAULT 'bank',
    -- Credit card specific: credit limit in minor units (null for bank)
    ADD COLUMN credit_limit_minor  BIGINT,
    -- Day of month (1–31) on which CC statement closes (null for bank)
    ADD COLUMN statement_closing_day INTEGER,
    -- Card network: Visa, Mastercard, Amex, Discover, etc. (null for bank)
    ADD COLUMN cc_network          VARCHAR(50);

CREATE INDEX treasury_bank_accounts_account_type
    ON treasury_bank_accounts(app_id, account_type);

-- ============================================================
-- TRANSACTION-LEVEL CC FIELDS
-- ============================================================

-- auth_date: date card was authorised (may differ from settle_date)
-- settle_date: date transaction settled with the issuer (= transaction_date for bank)
-- merchant_name: cleaned merchant descriptor from statement
-- merchant_category_code: ISO 18245 MCC (4-digit string)

ALTER TABLE treasury_bank_transactions
    ADD COLUMN auth_date               DATE,
    ADD COLUMN settle_date             DATE,
    ADD COLUMN merchant_name           VARCHAR(255),
    ADD COLUMN merchant_category_code  VARCHAR(4);

CREATE INDEX treasury_bank_transactions_auth_date
    ON treasury_bank_transactions(app_id, auth_date)
    WHERE auth_date IS NOT NULL;
