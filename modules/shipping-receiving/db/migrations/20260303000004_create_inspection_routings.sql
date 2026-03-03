-- Shipping-Receiving: Inspection routing at receiving (bd-x6ok5)
--
-- Tracks route decisions for inbound shipment lines:
--   direct_to_stock       → item goes straight to inventory
--   send_to_inspection    → item is queued for quality inspection
--
-- One routing decision per line. Idempotency via (tenant_id, idempotency_key).

CREATE TABLE inspection_routings (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        UUID NOT NULL,
    shipment_id      UUID NOT NULL REFERENCES shipments(id),
    shipment_line_id UUID NOT NULL REFERENCES shipment_lines(id),
    route_decision   TEXT NOT NULL,
    reason           TEXT,
    routed_by        UUID,
    routed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    idempotency_key  TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Only two valid decisions
    CONSTRAINT ck_route_decision
        CHECK (route_decision IN ('direct_to_stock', 'send_to_inspection')),

    -- One routing per line per tenant
    CONSTRAINT uq_inspection_routing_line
        UNIQUE (tenant_id, shipment_line_id)
);

-- Idempotency: same key within a tenant cannot create a second routing
CREATE UNIQUE INDEX uq_inspection_routing_idem
    ON inspection_routings (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- Query routings by shipment
CREATE INDEX idx_inspection_routings_tenant_shipment
    ON inspection_routings (tenant_id, shipment_id);

-- Query routings by decision type (e.g., find all items sent to inspection)
CREATE INDEX idx_inspection_routings_decision
    ON inspection_routings (tenant_id, route_decision);
