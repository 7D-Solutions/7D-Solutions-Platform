-- Table render requests for rich formatting (tables).
-- Tracks idempotent table render operations with tenant isolation.

CREATE TABLE table_render_requests (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    table_definition JSONB NOT NULL,
    pdf_output      BYTEA,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'rendered', 'failed')),
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rendered_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_table_render_tenant_idem
    ON table_render_requests(tenant_id, idempotency_key);
CREATE INDEX idx_table_render_tenant
    ON table_render_requests(tenant_id);
