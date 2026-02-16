-- Add correlation_id to journal_entries for audit traceability
--
-- Phase 16: Enable tracing GL entries back to originating AR/Payment context
--
-- Purpose:
-- - Track correlation_id from source events (invoice.issued, payment.succeeded)
-- - Enable audit queries like "show all GL entries for invoice X"
-- - Support financial reconciliation across modules

ALTER TABLE journal_entries
ADD COLUMN correlation_id UUID;

-- Index for correlation_id queries
CREATE INDEX idx_journal_entries_correlation_id ON journal_entries(correlation_id);

-- Composite index for tenant + correlation queries
CREATE INDEX idx_journal_entries_tenant_correlation ON journal_entries(tenant_id, correlation_id);
