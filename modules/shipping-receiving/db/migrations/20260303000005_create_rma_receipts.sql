-- Shipping-Receiving: RMA Receiving Workflow (bd-2p78n)
--
-- Tracks returned goods (RMA = Return Merchandise Authorization):
--   rma_receipts:      header with customer, condition notes, disposition status
--   rma_receipt_items:  line items on the RMA
--
-- Disposition state machine:
--   received → inspect → quarantine → return_to_stock | scrap
--                      → return_to_stock | scrap
-- Terminal states: return_to_stock, scrap

-- ─── rma_receipts ──────────────────────────────────────────────────────────────

CREATE TABLE rma_receipts (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          UUID NOT NULL,
    rma_id             TEXT NOT NULL,
    customer_id        UUID NOT NULL,
    condition_notes    TEXT,
    disposition_status TEXT NOT NULL DEFAULT 'received',
    idempotency_key    TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_rma_disposition_status
        CHECK (disposition_status IN ('received', 'inspect', 'quarantine', 'return_to_stock', 'scrap'))
);

-- Tenant-scoped query by rma_id
CREATE INDEX idx_rma_receipts_tenant_rma
    ON rma_receipts (tenant_id, rma_id);

-- Tenant-scoped query by disposition status
CREATE INDEX idx_rma_receipts_tenant_status
    ON rma_receipts (tenant_id, disposition_status);

-- Tenant-scoped query by customer
CREATE INDEX idx_rma_receipts_tenant_customer
    ON rma_receipts (tenant_id, customer_id);

-- Idempotency: same key within a tenant cannot create a duplicate receipt
CREATE UNIQUE INDEX uq_rma_receipts_idem
    ON rma_receipts (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- ─── rma_receipt_items ─────────────────────────────────────────────────────────

CREATE TABLE rma_receipt_items (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    rma_receipt_id  UUID NOT NULL REFERENCES rma_receipts(id),
    sku             TEXT NOT NULL,
    qty             BIGINT NOT NULL DEFAULT 1,
    condition_notes TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_rma_item_qty CHECK (qty > 0)
);

-- Items by receipt
CREATE INDEX idx_rma_receipt_items_tenant_receipt
    ON rma_receipt_items (tenant_id, rma_receipt_id);
