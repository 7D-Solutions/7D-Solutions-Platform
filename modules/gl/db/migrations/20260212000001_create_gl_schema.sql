-- GL (General Ledger) Database Schema
-- Double-entry accounting system with idempotent event sourcing

-- ============================================================
-- JOURNAL TABLES
-- ============================================================

-- Journal Entries (Header)
CREATE TABLE journal_entries (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    source_module TEXT NOT NULL,
    source_event_id UUID NOT NULL UNIQUE,
    source_subject TEXT NOT NULL,
    posted_at TIMESTAMP WITH TIME ZONE NOT NULL,
    currency TEXT NOT NULL,
    description TEXT,
    reference_type VARCHAR(50),
    reference_id TEXT,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Indexes for common queries
CREATE INDEX idx_journal_entries_tenant_id ON journal_entries(tenant_id);
CREATE INDEX idx_journal_entries_posted_at ON journal_entries(posted_at);
CREATE INDEX idx_journal_entries_source_event_id ON journal_entries(source_event_id);
CREATE INDEX idx_journal_entries_tenant_posted ON journal_entries(tenant_id, posted_at);

-- Journal Lines (Detail)
CREATE TABLE journal_lines (
    id UUID PRIMARY KEY,
    journal_entry_id UUID NOT NULL REFERENCES journal_entries(id),
    line_no INT NOT NULL,
    account_ref TEXT NOT NULL,
    debit_minor BIGINT NOT NULL DEFAULT 0 CHECK (debit_minor >= 0),
    credit_minor BIGINT NOT NULL DEFAULT 0 CHECK (credit_minor >= 0),
    memo TEXT,
    CONSTRAINT unique_entry_line_no UNIQUE (journal_entry_id, line_no)
);

-- Indexes for journal lines
CREATE INDEX idx_journal_lines_entry_id ON journal_lines(journal_entry_id);
CREATE INDEX idx_journal_lines_account_ref ON journal_lines(account_ref);
