-- Add correlation_id to work_orders for idempotent creation.
-- A given correlation_id can only produce one WO per tenant.

ALTER TABLE work_orders
    ADD COLUMN IF NOT EXISTS correlation_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_work_orders_correlation
    ON work_orders (tenant_id, correlation_id)
    WHERE correlation_id IS NOT NULL;
