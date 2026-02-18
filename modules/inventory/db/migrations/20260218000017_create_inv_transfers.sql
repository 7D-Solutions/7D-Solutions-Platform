-- Inventory: Transfer Records
--
-- inv_transfers is the business record for an inter-warehouse transfer.
-- Each transfer creates exactly two ledger rows:
--   - 'transfer_out': debits the source warehouse (negative quantity)
--   - 'transfer_in':  credits the destination warehouse (positive quantity)
-- Both rows share the same reference_id = inv_transfers.id for traceability.
--
-- FIFO consumption on the source side mirrors the issue flow.
-- A new FIFO layer is created on the destination side at weighted average cost.

CREATE TABLE inv_transfers (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    item_id           UUID NOT NULL REFERENCES items(id),
    from_warehouse_id UUID NOT NULL,
    to_warehouse_id   UUID NOT NULL,
    quantity          BIGINT NOT NULL CHECK (quantity > 0),
    -- Outbox event id (used in inventory.transfer_completed payload)
    event_id          UUID NOT NULL UNIQUE,
    -- Both ledger legs for full auditability
    issue_ledger_id   BIGINT NOT NULL REFERENCES inventory_ledger(id),
    receipt_ledger_id BIGINT NOT NULL REFERENCES inventory_ledger(id),
    transferred_at    TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_inv_transfers_tenant      ON inv_transfers(tenant_id);
CREATE INDEX idx_inv_transfers_tenant_item ON inv_transfers(tenant_id, item_id);
