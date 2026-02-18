-- Inventory: Serial Instance Tracking
--
-- inventory_serial_instances:
--   One row per individual serialised unit.
--   Created when a serial-tracked item is received (one row per serial_code
--   in the receipt payload).
--   Each instance is tied to the receipt ledger entry and FIFO layer that
--   created it, enabling full traceability of where each unit came from.
--
-- Status transitions:
--   on_hand   → issued      (consumed by an issue)
--   on_hand   → transferred (consumed by a transfer)
--   on_hand   → adjusted    (quantity adjustment removed it)
--   (status is terminal once set; serials are never re-received)
--
-- Non-serial items do not create rows here; serial_codes on the issue or
-- transfer reference existing rows in this table.

CREATE TABLE inventory_serial_instances (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               TEXT NOT NULL,
    item_id                 UUID NOT NULL REFERENCES items(id),
    -- The human-readable serial code (barcode, manufacturer SN, etc.)
    serial_code             TEXT NOT NULL,
    -- The receipt ledger entry that created this serial instance
    receipt_ledger_entry_id BIGINT NOT NULL REFERENCES inventory_ledger(id),
    -- The FIFO layer this instance occupies
    layer_id                UUID NOT NULL REFERENCES inventory_layers(id),
    -- Lifecycle state of this unit
    status                  TEXT NOT NULL DEFAULT 'on_hand',
    created_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Serial codes are globally unique per tenant+item
    CONSTRAINT serial_instances_unique_code UNIQUE (tenant_id, item_id, serial_code),

    CONSTRAINT serial_instances_status_check
        CHECK (status IN ('on_hand', 'issued', 'transferred', 'adjusted'))
);

CREATE INDEX idx_serials_tenant_item   ON inventory_serial_instances(tenant_id, item_id);
CREATE INDEX idx_serials_tenant_code   ON inventory_serial_instances(tenant_id, serial_code);
CREATE INDEX idx_serials_layer_id      ON inventory_serial_instances(layer_id);
CREATE INDEX idx_serials_status        ON inventory_serial_instances(tenant_id, item_id, status)
    WHERE status = 'on_hand';
