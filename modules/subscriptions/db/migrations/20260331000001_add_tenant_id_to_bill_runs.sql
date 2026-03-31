-- Add tenant_id to bill_runs for tenant isolation.
-- Existing rows get a placeholder; production has no bill_run data yet.
ALTER TABLE bill_runs ADD COLUMN tenant_id VARCHAR(255);
UPDATE bill_runs SET tenant_id = 'unknown' WHERE tenant_id IS NULL;
ALTER TABLE bill_runs ALTER COLUMN tenant_id SET NOT NULL;
CREATE INDEX idx_bill_runs_tenant ON bill_runs(tenant_id);
