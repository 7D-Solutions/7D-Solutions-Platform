-- Operation instances: add routing_step_id linkage and tenant_id for multi-tenant queries.

ALTER TABLE operations
    ADD COLUMN IF NOT EXISTS routing_step_id UUID REFERENCES routing_steps(routing_step_id),
    ADD COLUMN IF NOT EXISTS tenant_id TEXT;

-- Backfill tenant_id from parent work order (no-op on empty table)
UPDATE operations o
SET tenant_id = w.tenant_id
FROM work_orders w
WHERE o.work_order_id = w.work_order_id
  AND o.tenant_id IS NULL;

-- Now make tenant_id NOT NULL
ALTER TABLE operations
    ALTER COLUMN tenant_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_operations_tenant
    ON operations (tenant_id);
