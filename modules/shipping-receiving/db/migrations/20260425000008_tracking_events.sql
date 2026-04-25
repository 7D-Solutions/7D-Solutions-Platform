-- Shipping-Receiving: Carrier Tracking Events (bd-4dbh3)
--
-- tracking_events: canonical log of carrier webhook / poll events.
--   Idempotency: UNIQUE (tracking_number, carrier_code, raw_payload_hash)
--   prevents duplicate rows on webhook replay or retry storms.
--
-- shipments additions:
--   carrier_status / carrier_status_updated_at — latest carrier-reported
--     tracking status, separate from the shipment state-machine status.
--   parent_shipment_id — nullable FK for multi-package shipments where each
--     child parcel has its own tracking number. Recomputation rule: master
--     carrier_status = "least advanced" of all children.

-- ─── tracking_events ─────────────────────────────────────────────────────────

CREATE TABLE tracking_events (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT        NOT NULL,
    shipment_id      UUID        REFERENCES shipments(id),
    tracking_number  TEXT        NOT NULL,
    carrier_code     TEXT        NOT NULL,
    status           TEXT        NOT NULL,
    status_dttm      TIMESTAMPTZ NOT NULL,
    location         TEXT,
    raw_payload_hash TEXT        NOT NULL,
    received_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT ck_tracking_status CHECK (
        status IN (
            'pending','picked_up','in_transit','out_for_delivery',
            'delivered','exception','returned','lost'
        )
    ),

    -- Idempotency: replay of the exact same carrier payload is a no-op.
    CONSTRAINT uq_tracking_idempotent
        UNIQUE (tracking_number, carrier_code, raw_payload_hash)
);

-- Fast latest-status lookup per tracking number (e.g. "what is 1ZABC latest?")
CREATE INDEX idx_tracking_events_num_dttm
    ON tracking_events (tracking_number, status_dttm DESC);

-- Shipment-scoped event history
CREATE INDEX idx_tracking_events_shipment
    ON tracking_events (shipment_id)
    WHERE shipment_id IS NOT NULL;

CREATE INDEX idx_tracking_events_tenant
    ON tracking_events (tenant_id);

-- ─── shipments: carrier_status + multi-package ────────────────────────────────

-- Latest carrier-reported tracking status.
-- NULL = no webhook/poll events received yet for this shipment.
ALTER TABLE shipments
    ADD COLUMN carrier_status            TEXT,
    ADD COLUMN carrier_status_updated_at TIMESTAMPTZ;

-- Nullable FK: child shipments reference their master.
-- Only set for multi-package label groups where each child has its own
-- tracking number.
ALTER TABLE shipments
    ADD COLUMN parent_shipment_id UUID REFERENCES shipments(id);

CREATE INDEX idx_shipments_parent
    ON shipments (parent_shipment_id)
    WHERE parent_shipment_id IS NOT NULL;
