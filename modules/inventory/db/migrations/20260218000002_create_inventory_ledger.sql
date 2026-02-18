-- Inventory: Append-Only Ledger
--
-- This is the authoritative source of truth for all stock movements.
-- DESIGN INVARIANT: rows are NEVER updated or deleted after insert.
-- Every stock movement (receipt, issue, adjustment, transfer leg) is a new row.
--
-- entry_type values:
--   received         → stock in from purchase/inbound shipment
--   issued           → stock out to fulfillment/consumption
--   adjusted         → manual stock adjustment (positive or negative delta)
--   transfer_in      → stock in to destination warehouse (transfer leg)
--   transfer_out     → stock out from source warehouse (transfer leg)
--
-- source_event_id UNIQUE enforces exactly-once insert per event.
-- quantity is signed: positive = stock in, negative = stock out.

CREATE TYPE inv_entry_type AS ENUM (
    'received',
    'issued',
    'adjusted',
    'transfer_in',
    'transfer_out'
);

CREATE TABLE inventory_ledger (
    -- BIGSERIAL gives a monotonically increasing stable ordering key for FIFO
    id                   BIGSERIAL PRIMARY KEY,
    entry_id             UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    tenant_id            TEXT NOT NULL,
    item_id              UUID NOT NULL REFERENCES items(id),
    warehouse_id         UUID NOT NULL,
    entry_type           inv_entry_type NOT NULL,
    -- signed quantity: positive = stock increase, negative = stock decrease
    quantity             BIGINT NOT NULL,
    unit_cost_minor      BIGINT NOT NULL DEFAULT 0 CHECK (unit_cost_minor >= 0),
    currency             TEXT NOT NULL DEFAULT 'usd',
    -- Idempotency: one ledger row per source event
    source_event_id      UUID NOT NULL UNIQUE,
    source_event_type    TEXT NOT NULL,
    -- Optional business reference (order, PO, transfer, adjustment ID)
    reference_type       TEXT,
    reference_id         TEXT,
    notes                TEXT,
    posted_at            TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at           TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Primary access patterns
CREATE INDEX idx_ledger_tenant_id       ON inventory_ledger(tenant_id);
CREATE INDEX idx_ledger_item_warehouse  ON inventory_ledger(item_id, warehouse_id);
CREATE INDEX idx_ledger_source_event_id ON inventory_ledger(source_event_id);
CREATE INDEX idx_ledger_posted_at       ON inventory_ledger(posted_at);
-- For projection rebuilds: replay all entries for a tenant+item in order
CREATE INDEX idx_ledger_tenant_item_seq ON inventory_ledger(tenant_id, item_id, id);
