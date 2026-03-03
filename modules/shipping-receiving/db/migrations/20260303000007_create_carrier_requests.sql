-- Shipping-Receiving: Carrier Integration Requests (bd-1qsu8)
--
-- Tracks durable carrier integration requests (rate quotes, label generation,
-- tracking updates). Every request/response is logged before any action.
-- Vendor-agnostic — carrier_code identifies the carrier but the platform
-- never commits to a specific carrier SDK.
--
-- Status state machine:
--   pending → submitted → completed | failed
--   failed → submitted (retry)
-- Terminal states: completed

-- ─── sr_carrier_requests ────────────────────────────────────────────────

CREATE TABLE sr_carrier_requests (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    shipment_id     UUID NOT NULL,
    request_type    TEXT NOT NULL,
    carrier_code    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    payload         JSONB NOT NULL DEFAULT '{}',
    response        JSONB,
    idempotency_key TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_sr_carrier_request_type
        CHECK (request_type IN ('rate', 'label', 'track')),
    CONSTRAINT ck_sr_carrier_request_status
        CHECK (status IN ('pending', 'submitted', 'completed', 'failed'))
);

-- Tenant-scoped query by shipment
CREATE INDEX idx_sr_carrier_req_tenant_shipment
    ON sr_carrier_requests (tenant_id, shipment_id);

-- Tenant-scoped query by status (find pending work)
CREATE INDEX idx_sr_carrier_req_tenant_status
    ON sr_carrier_requests (tenant_id, status);

-- Idempotency: same key within a tenant cannot create a duplicate request
CREATE UNIQUE INDEX uq_sr_carrier_req_idem
    ON sr_carrier_requests (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
