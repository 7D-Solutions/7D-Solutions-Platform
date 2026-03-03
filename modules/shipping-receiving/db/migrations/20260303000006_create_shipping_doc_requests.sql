-- Shipping-Receiving: Shipping Document Requests (bd-12j00)
--
-- Tracks durable requests for shipping document generation (packing slips, BOLs).
-- Provides traceability and reproducibility — the actual rendering is handled
-- downstream by doc-mgmt/pdf workflows.
--
-- Status state machine:
--   requested → generating → completed | failed
--   failed → generating (retry)
-- Terminal states: completed

-- ─── sr_shipping_doc_requests ────────────────────────────────────────────────

CREATE TABLE sr_shipping_doc_requests (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    shipment_id     UUID NOT NULL,
    doc_type        TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'requested',
    payload_ref     TEXT,
    idempotency_key TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT ck_sr_doc_type
        CHECK (doc_type IN ('packing_slip', 'bill_of_lading')),
    CONSTRAINT ck_sr_doc_status
        CHECK (status IN ('requested', 'generating', 'completed', 'failed'))
);

-- Tenant-scoped query by shipment
CREATE INDEX idx_sr_doc_req_tenant_shipment
    ON sr_shipping_doc_requests (tenant_id, shipment_id);

-- Tenant-scoped query by status (find pending work)
CREATE INDEX idx_sr_doc_req_tenant_status
    ON sr_shipping_doc_requests (tenant_id, status);

-- Idempotency: same key within a tenant cannot create a duplicate request
CREATE UNIQUE INDEX uq_sr_doc_req_idem
    ON sr_shipping_doc_requests (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
