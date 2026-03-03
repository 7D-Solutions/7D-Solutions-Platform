-- Label generation records for barcode/label traceability.
--
-- Each label is a durable record of a generation request. The payload is a
-- deterministic JSON document derived from the item's revision context at
-- generation time, suitable for driving barcode rendering or label printing
-- downstream.
--
-- Invariants:
--   - Every label links to an item revision for full audit trail.
--   - Idempotent: repeated requests with the same idempotency_key return the
--     existing label without creating a duplicate.
--   - Tenant-scoped: all queries filter on tenant_id.

CREATE TABLE inv_labels (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    item_id         UUID NOT NULL REFERENCES items(id),
    revision_id     UUID NOT NULL REFERENCES item_revisions(id),
    label_type      TEXT NOT NULL,             -- 'item_label', 'lot_label'
    barcode_format  TEXT NOT NULL DEFAULT 'code128',
    payload         JSONB NOT NULL,
    idempotency_key TEXT,
    actor_id        UUID,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- label_type must be a known value
    CONSTRAINT inv_labels_type_check
        CHECK (label_type IN ('item_label', 'lot_label')),

    -- barcode_format must be a known value
    CONSTRAINT inv_labels_format_check
        CHECK (barcode_format IN ('code128', 'code39', 'qr', 'datamatrix', 'ean13')),

    -- Idempotency key unique per tenant (NULL keys are exempt)
    CONSTRAINT inv_labels_tenant_idemp_unique
        UNIQUE (tenant_id, idempotency_key)
);

-- Query pattern: labels for an item
CREATE INDEX idx_inv_labels_item
    ON inv_labels(tenant_id, item_id);

-- Query pattern: labels by revision
CREATE INDEX idx_inv_labels_revision
    ON inv_labels(tenant_id, revision_id);

-- Query pattern: labels by type for a tenant
CREATE INDEX idx_inv_labels_type
    ON inv_labels(tenant_id, label_type);
