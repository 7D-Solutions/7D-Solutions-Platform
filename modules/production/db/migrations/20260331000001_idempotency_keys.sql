-- Production idempotency keys (scoped per tenant)
-- Prevents double-submit creating duplicate outbox events.

CREATE TABLE IF NOT EXISTS production_idempotency_keys (
    id               BIGSERIAL PRIMARY KEY,
    tenant_id        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    request_hash     TEXT NOT NULL,
    response_body    JSONB NOT NULL,
    status_code      SMALLINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at       TIMESTAMPTZ NOT NULL,

    CONSTRAINT production_idempotency_tenant_key_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_production_idempotency_expires
    ON production_idempotency_keys(expires_at);
CREATE INDEX IF NOT EXISTS idx_production_idempotency_tenant
    ON production_idempotency_keys(tenant_id);
