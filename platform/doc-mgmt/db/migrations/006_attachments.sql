-- DOC6: entity-agnostic file attachments with presigned URL support.
--
-- No file bytes flow through the service. The attachment record tracks metadata
-- and the S3 key; upload/download happen via presigned URLs issued by the
-- blob-storage crate (ADR-018).

CREATE TABLE IF NOT EXISTS attachments (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID         NOT NULL,
    entity_type TEXT         NOT NULL,
    entity_id   TEXT         NOT NULL,
    filename    TEXT         NOT NULL,
    mime_type   TEXT         NOT NULL,
    size_bytes  BIGINT       NOT NULL DEFAULT 0,
    s3_key      TEXT         NOT NULL,
    status      TEXT         NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'uploaded', 'deleted')),
    uploaded_at TIMESTAMPTZ,
    deleted_at  TIMESTAMPTZ,
    created_by  UUID         NOT NULL,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT now()
);

-- Fast lookup by owning entity, excluding deleted rows.
CREATE INDEX idx_attachments_entity
    ON attachments (tenant_id, entity_type, entity_id, created_at DESC)
    WHERE status != 'deleted';
