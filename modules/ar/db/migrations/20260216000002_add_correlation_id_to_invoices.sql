-- Add correlation_id to AR invoices for distributed tracing
-- Phase 16: Enable correlation across invoice lifecycle boundaries

-- Add correlation_id column to invoices table
ALTER TABLE ar_invoices
ADD COLUMN correlation_id VARCHAR(255);

-- Create index for correlation lookups (tracing queries)
CREATE INDEX ar_invoices_correlation_id ON ar_invoices(correlation_id);

-- Add comment for documentation
COMMENT ON COLUMN ar_invoices.correlation_id IS 'Distributed tracing correlation ID - links invoice creation to upstream context (e.g., subscription billing cycle, bill run execution). Enables cross-service event correlation.';
