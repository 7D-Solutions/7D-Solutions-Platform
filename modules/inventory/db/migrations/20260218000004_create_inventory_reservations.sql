-- Inventory: Reservations (Compensating Semantics)
--
-- A reservation holds stock for a pending fulfillment.
-- Lifecycle:
--   1. Reserve  → status='active',   reverses_reservation_id=NULL
--   2. Release  → status='released', reverses_reservation_id=<original reserve id>
--      OR
--   2. Fulfill  → status='fulfilled', reverses_reservation_id=<original reserve id>
--
-- INVARIANT: Every release or fulfillment row MUST reference the original reserve row
-- via reverses_reservation_id. This is the compensating linkage required by the bead.
-- A reserve row (reverses_reservation_id IS NULL) is the primary entry.
-- A release/fulfillment row (reverses_reservation_id IS NOT NULL) is the compensating entry.

CREATE TYPE inv_reservation_status AS ENUM (
    'active',      -- stock is currently held
    'released',    -- reservation cancelled; stock returned to available
    'fulfilled'    -- reservation consumed by actual issue
);

CREATE TABLE inventory_reservations (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                TEXT NOT NULL,
    item_id                  UUID NOT NULL REFERENCES items(id),
    warehouse_id             UUID NOT NULL,
    quantity                 BIGINT NOT NULL CHECK (quantity > 0),
    status                   inv_reservation_status NOT NULL DEFAULT 'active',

    -- Compensating linkage:
    --   NULL  → this is the original reserve entry
    --   <uuid> → this is a release/fulfillment that compensates the original
    reverses_reservation_id  UUID REFERENCES inventory_reservations(id),

    -- Business reference (e.g. sales order, fulfillment order)
    reference_type           TEXT,
    reference_id             TEXT,

    -- Ledger entries associated with the reserve and release legs
    reserve_ledger_entry_id  BIGINT REFERENCES inventory_ledger(id),
    release_ledger_entry_id  BIGINT REFERENCES inventory_ledger(id),

    reserved_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    released_at              TIMESTAMP WITH TIME ZONE,
    fulfilled_at             TIMESTAMP WITH TIME ZONE,
    expires_at               TIMESTAMP WITH TIME ZONE,
    created_at               TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Only compensating entries may reference a prior reservation
    CONSTRAINT reservations_compensating_requires_original
        CHECK (
            (reverses_reservation_id IS NULL AND status = 'active')
            OR
            (reverses_reservation_id IS NOT NULL AND status IN ('released', 'fulfilled'))
        )
);

CREATE INDEX idx_reservations_tenant_id   ON inventory_reservations(tenant_id);
CREATE INDEX idx_reservations_item_wh     ON inventory_reservations(item_id, warehouse_id);
CREATE INDEX idx_reservations_status      ON inventory_reservations(status);
CREATE INDEX idx_reservations_reference   ON inventory_reservations(reference_type, reference_id);
CREATE INDEX idx_reservations_reverses    ON inventory_reservations(reverses_reservation_id)
    WHERE reverses_reservation_id IS NOT NULL;
-- Active reservations lookup (most common query)
CREATE INDEX idx_reservations_active_item ON inventory_reservations(item_id, warehouse_id)
    WHERE status = 'active';
