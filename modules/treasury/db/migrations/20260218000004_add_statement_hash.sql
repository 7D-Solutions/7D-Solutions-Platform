-- Treasury: Add statement_hash for idempotent CSV import
-- UUID v5 hash of raw CSV content ensures re-import is a no-op.

ALTER TABLE treasury_bank_statements
    ADD COLUMN statement_hash UUID;

CREATE UNIQUE INDEX treasury_bank_statements_hash_unique
    ON treasury_bank_statements(account_id, statement_hash)
    WHERE statement_hash IS NOT NULL;
