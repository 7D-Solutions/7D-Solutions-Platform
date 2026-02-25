-- Shipping-Receiving: Core schema v1
-- bd-37xa: shipments, shipment_lines, indexes, constraints.
--
-- Direction drives valid status sets:
--   inbound:  draft → confirmed → in_transit → arrived → receiving → closed → cancelled
--   outbound: draft → confirmed → picking → packed → shipped → delivered → closed → cancelled

-- ─── shipments ───────────────────────────────────────────────────────────────

CREATE TABLE shipments (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             UUID NOT NULL,
    direction             TEXT NOT NULL,
    status                TEXT NOT NULL DEFAULT 'draft',
    carrier_party_id      UUID,
    tracking_number       TEXT,
    freight_cost_minor    BIGINT,
    currency              TEXT,
    expected_arrival_date TIMESTAMPTZ,
    arrived_at            TIMESTAMPTZ,
    shipped_at            TIMESTAMPTZ,
    delivered_at          TIMESTAMPTZ,
    closed_at             TIMESTAMPTZ,
    created_by            UUID,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- direction must be inbound or outbound
    CONSTRAINT ck_shipments_direction
        CHECK (direction IN ('inbound', 'outbound')),

    -- status constrained per direction
    CONSTRAINT ck_shipments_status CHECK (
        (direction = 'inbound'  AND status IN ('draft','confirmed','in_transit','arrived','receiving','closed','cancelled'))
        OR
        (direction = 'outbound' AND status IN ('draft','confirmed','picking','packed','shipped','delivered','closed','cancelled'))
    )
);

-- Primary query paths
CREATE INDEX idx_shipments_tenant_dir_status
    ON shipments (tenant_id, direction, status, created_at);

CREATE INDEX idx_shipments_tenant_tracking
    ON shipments (tenant_id, tracking_number)
    WHERE tracking_number IS NOT NULL;

CREATE INDEX idx_shipments_tenant_carrier
    ON shipments (tenant_id, carrier_party_id)
    WHERE carrier_party_id IS NOT NULL;

-- ─── shipment_lines ──────────────────────────────────────────────────────────

CREATE TABLE shipment_lines (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID NOT NULL,
    shipment_id      UUID NOT NULL REFERENCES shipments(id),
    sku              TEXT,
    uom              TEXT,
    warehouse_id     UUID,
    qty_expected     BIGINT NOT NULL DEFAULT 0,
    qty_shipped      BIGINT NOT NULL DEFAULT 0,
    qty_received     BIGINT NOT NULL DEFAULT 0,
    qty_accepted     BIGINT NOT NULL DEFAULT 0,
    qty_rejected     BIGINT NOT NULL DEFAULT 0,
    source_ref_type  TEXT,
    source_ref_id    UUID,
    po_id            UUID,
    po_line_id       UUID,
    inventory_ref_id UUID,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- All quantities must be non-negative
    CONSTRAINT ck_line_qty_expected  CHECK (qty_expected  >= 0),
    CONSTRAINT ck_line_qty_shipped   CHECK (qty_shipped   >= 0),
    CONSTRAINT ck_line_qty_received  CHECK (qty_received  >= 0),
    CONSTRAINT ck_line_qty_accepted  CHECK (qty_accepted  >= 0),
    CONSTRAINT ck_line_qty_rejected  CHECK (qty_rejected  >= 0)
);

-- Lines by shipment
CREATE INDEX idx_shipment_lines_tenant_shipment
    ON shipment_lines (tenant_id, shipment_id);

-- Cross-reference lookups
CREATE INDEX idx_shipment_lines_tenant_po
    ON shipment_lines (tenant_id, po_id)
    WHERE po_id IS NOT NULL;

CREATE INDEX idx_shipment_lines_tenant_po_line
    ON shipment_lines (tenant_id, po_line_id)
    WHERE po_line_id IS NOT NULL;

CREATE INDEX idx_shipment_lines_tenant_source_ref
    ON shipment_lines (tenant_id, source_ref_type, source_ref_id)
    WHERE source_ref_type IS NOT NULL;

CREATE INDEX idx_shipment_lines_tenant_sku
    ON shipment_lines (tenant_id, sku)
    WHERE sku IS NOT NULL;
