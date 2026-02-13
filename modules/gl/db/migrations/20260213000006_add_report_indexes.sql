-- Performance indexes for Phase 12 reporting queries
-- Ensures bounded, indexed access for all report primitives
-- Additive only - no drops or modifications

-- ============================================================
-- JOURNAL_LINES INDEXES
-- ============================================================

-- Composite index for account activity queries
-- Supports: WHERE account_ref = X AND journal_entry_id IN (...)
-- Used by: query_account_activity, query_entries_by_account_codes
CREATE INDEX idx_journal_lines_account_entry
    ON journal_lines(account_ref, journal_entry_id);

-- ============================================================
-- ACCOUNTS INDEXES
-- ============================================================

-- Composite index for account type filtering
-- Supports: WHERE tenant_id = X AND type = ANY(array)
-- Used by: query_entries_by_account_types, count_entries_by_account_types
CREATE INDEX idx_accounts_tenant_type
    ON accounts(tenant_id, type);

-- ============================================================
-- NOTES
-- ============================================================
-- Existing indexes already cover:
-- - journal_entries(tenant_id, posted_at) via idx_journal_entries_tenant_posted
-- - journal_lines(journal_entry_id) via idx_journal_lines_entry_id
-- - accounts(tenant_id, code) via idx_accounts_tenant_code
--
-- These new indexes complete the coverage for Phase 12 reporting primitives.
