-- Work Order Parts & Labor
-- Standalone mode: manual entry with description + cost.
-- part_ref links to Inventory SKU (informational in v1).
-- inventory_issue_ref links to future Inventory issue transaction.

CREATE TABLE work_order_parts (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    work_order_id       UUID NOT NULL REFERENCES work_orders(id),
    part_description    TEXT NOT NULL,
    part_ref            TEXT,
    quantity            INTEGER NOT NULL CHECK (quantity > 0),
    unit_cost_minor     BIGINT NOT NULL CHECK (unit_cost_minor >= 0),
    currency            TEXT NOT NULL DEFAULT 'USD' CHECK (length(currency) = 3),
    inventory_issue_ref UUID,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_work_order_parts_tenant ON work_order_parts(tenant_id);
CREATE INDEX idx_work_order_parts_wo ON work_order_parts(tenant_id, work_order_id);

CREATE TABLE work_order_labor (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id      TEXT NOT NULL,
    work_order_id  UUID NOT NULL REFERENCES work_orders(id),
    technician_ref TEXT NOT NULL,
    hours_decimal  NUMERIC(8, 2) NOT NULL CHECK (hours_decimal > 0),
    rate_minor     BIGINT NOT NULL CHECK (rate_minor >= 0),
    currency       TEXT NOT NULL DEFAULT 'USD' CHECK (length(currency) = 3),
    description    TEXT,
    created_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_work_order_labor_tenant ON work_order_labor(tenant_id);
CREATE INDEX idx_work_order_labor_wo ON work_order_labor(tenant_id, work_order_id);
