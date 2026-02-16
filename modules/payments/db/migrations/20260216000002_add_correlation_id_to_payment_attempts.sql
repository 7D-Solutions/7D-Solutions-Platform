-- Phase 16: Add correlation_id to payment_attempts for distributed tracing
-- Enables end-to-end trace from invoice → payment attempt → reconciliation

-- Add correlation_id column
ALTER TABLE payment_attempts
ADD COLUMN correlation_id VARCHAR(255);

-- Index for trace lookups
CREATE INDEX payment_attempts_correlation_id ON payment_attempts(correlation_id);

-- Comment
COMMENT ON COLUMN payment_attempts.correlation_id IS 'Phase 16: Distributed trace ID propagated from invoice creation. Links payment attempts to originating business transaction for end-to-end observability.';
