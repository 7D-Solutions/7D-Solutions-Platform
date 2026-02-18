-- AP HTTP Idempotency Keys
--
-- Stores the result of idempotent API requests so that clients retrying
-- with the same Idempotency-Key header receive the original response.
-- Scoped per tenant to prevent cross-tenant key collisions.
--
-- Pattern matches inv_idempotency_keys from the inventory module.

CREATE TABLE idempotency_keys (
    id               BIGSERIAL PRIMARY KEY,
    tenant_id        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    -- Hash of the request body to detect key reuse with different payloads
    request_hash     TEXT NOT NULL,
    response_body    JSONB NOT NULL,
    status_code      INT NOT NULL,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    -- Keys expire after a TTL (typically 24h–7d); background cleaner uses this
    expires_at       TIMESTAMP WITH TIME ZONE NOT NULL,

    CONSTRAINT uq_idempotency_tenant_key UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_idempotency_expires ON idempotency_keys (expires_at);
CREATE INDEX idx_idempotency_tenant  ON idempotency_keys (tenant_id);
